use std::{
    collections::{HashMap, HashSet},
    fmt::Write as _,
    sync::Arc,
};

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use context_hub_domain::OntologyDraft;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::{io::AsyncWriteExt, sync::RwLock};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphQuery {
    pub workspace_id: Uuid,
    pub ontology_version_id: Uuid,
    pub root_type: String,
    #[serde(default)]
    pub filters: Vec<GraphFilter>,
    #[serde(default)]
    pub traversal: Vec<TraversalStep>,
    pub limit: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphFilter {
    pub property: String,
    pub operator: FilterOperator,
    pub value: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FilterOperator {
    Equal,
    NotEqual,
    Contains,
    GreaterThan,
    LessThan,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TraversalStep {
    pub link_type: String,
    pub target_type: String,
    #[serde(default)]
    pub reverse: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompiledGraphQuery {
    pub sql: String,
    pub parameters: Vec<String>,
}

#[derive(Debug, Error)]
pub enum QueryError {
    #[error("invalid graph identifier '{0}'")]
    InvalidIdentifier(String),
    #[error("traversal depth {0} exceeds the maximum of 6")]
    TraversalTooDeep(usize),
    #[error("query limit {0} exceeds the maximum of 5000")]
    LimitExceeded(u32),
}

/// Compiles a bounded graph query into parameterized `ClickHouse` SQL.
///
/// # Errors
///
/// Returns [`QueryError`] when identifiers, traversal depth, or the result limit violate the
/// public query constraints.
pub fn compile_graph_query(query: &GraphQuery) -> Result<CompiledGraphQuery, QueryError> {
    validate_graph_identifier(&query.root_type)?;
    if query.traversal.len() > 6 {
        return Err(QueryError::TraversalTooDeep(query.traversal.len()));
    }
    let limit = if query.limit == 0 { 500 } else { query.limit };
    if limit > 5_000 {
        return Err(QueryError::LimitExceeded(limit));
    }

    let mut parameters = Vec::new();
    let mut joins = String::new();
    let mut current = "n0".to_owned();
    for (index, step) in query.traversal.iter().enumerate() {
        validate_graph_identifier(&step.link_type)?;
        validate_graph_identifier(&step.target_type)?;
        let edge = format!("e{}", index + 1);
        let next = format!("n{}", index + 1);
        let (from, to, from_type) = if step.reverse {
            ("target_id", "source_id", "target_type")
        } else {
            ("source_id", "target_id", "source_type")
        };
        write!(&mut joins, " JOIN graph_edges AS {edge} FINAL ON {edge}.workspace_id = {current}.workspace_id AND {edge}.ontology_version_id = {current}.ontology_version_id AND {edge}.{from} = {current}.object_id AND {edge}.{from_type} = {current}.object_type AND {edge}.link_type = ? AND {edge}.deleted = false")
            .expect("writing to a String cannot fail");
        write!(&mut joins, " JOIN graph_nodes AS {next} FINAL ON {next}.workspace_id = {edge}.workspace_id AND {next}.ontology_version_id = {edge}.ontology_version_id AND {next}.object_id = {edge}.{to} AND {next}.object_type = ? AND {next}.deleted = false")
            .expect("writing to a String cannot fail");
        parameters.push(step.link_type.clone());
        parameters.push(step.target_type.clone());
        current = next;
    }

    parameters.extend([
        query.workspace_id.to_string(),
        query.ontology_version_id.to_string(),
        query.root_type.clone(),
    ]);

    let mut filters = String::new();
    for filter in &query.filters {
        validate_graph_identifier(&filter.property)?;
        let operation = match filter.operator {
            FilterOperator::Equal => "=",
            FilterOperator::NotEqual => "!=",
            FilterOperator::Contains => "ILIKE",
            FilterOperator::GreaterThan => ">",
            FilterOperator::LessThan => "<",
        };
        write!(
            &mut filters,
            " AND toString(n0.properties.{}) {operation} ?",
            filter.property
        )
        .expect("writing to a String cannot fail");
        parameters.push(if matches!(filter.operator, FilterOperator::Contains) {
            format!("%{}%", filter.value)
        } else {
            filter.value.clone()
        });
    }
    let sql = format!(
        "SELECT {current}.object_id, {current}.object_type, toJSONString({current}.properties) FROM graph_nodes AS n0 FINAL{joins} WHERE n0.workspace_id = ? AND n0.ontology_version_id = ? AND n0.object_type = ? AND n0.deleted = false{filters} ORDER BY {current}.object_id LIMIT {limit}"
    );
    Ok(CompiledGraphQuery { sql, parameters })
}

fn validate_graph_identifier(value: &str) -> Result<(), QueryError> {
    let valid = !value.is_empty()
        && value.len() <= 64
        && value
            .chars()
            .next()
            .is_some_and(|character| character.is_ascii_lowercase())
        && value.chars().all(|character| {
            character.is_ascii_lowercase() || character.is_ascii_digit() || character == '_'
        });
    if valid {
        Ok(())
    } else {
        Err(QueryError::InvalidIdentifier(value.to_owned()))
    }
}

#[derive(Debug, Error)]
pub enum StorageError {
    #[error("ClickHouse store error: {0}")]
    ClickHouse(#[from] clickhouse::error::Error),
    #[error("I/O error while writing to the store: {0}")]
    Io(#[from] std::io::Error),
    #[error("serialization error: {0}")]
    Serialization(#[from] serde_json::Error),
    #[error("stored record is invalid: {0}")]
    InvalidRecord(String),
    #[error("resource not found")]
    NotFound,
    #[error("draft revision conflict: expected {expected}, current {current}")]
    RevisionConflict { expected: u64, current: u64 },
}

#[async_trait]
pub trait OntologyRepository: Send + Sync {
    async fn get_draft(&self, id: Uuid) -> Result<OntologyDraft, StorageError>;
    async fn save_draft(
        &self,
        draft: &OntologyDraft,
        expected_revision: u64,
    ) -> Result<(), StorageError>;
}

#[derive(Clone)]
pub struct ClickHouseOntologyRepository {
    client: clickhouse::Client,
}

impl ClickHouseOntologyRepository {
    pub fn new(client: clickhouse::Client) -> Self {
        Self { client }
    }
}

#[async_trait]
impl OntologyRepository for ClickHouseOntologyRepository {
    async fn get_draft(&self, id: Uuid) -> Result<OntologyDraft, StorageError> {
        let row = self
            .client
            .query(
                "SELECT toString(id), toString(workspace_id), revision, definition_json, layout_json, toUnixTimestamp64Micro(updated_at) FROM ontology_drafts FINAL WHERE id = ? AND deleted = false LIMIT 1",
            )
            .bind(id.to_string())
            .fetch_optional::<(String, String, u64, String, String, i64)>()
            .await?
            .ok_or(StorageError::NotFound)?;
        Ok(OntologyDraft {
            id: parse_uuid(&row.0, "ontology draft id")?,
            workspace_id: parse_uuid(&row.1, "workspace id")?,
            revision: row.2,
            definition: serde_json::from_str(&row.3)?,
            layout: serde_json::from_str(&row.4)?,
            updated_at: timestamp_from_micros(row.5)?,
        })
    }

    async fn save_draft(
        &self,
        draft: &OntologyDraft,
        expected_revision: u64,
    ) -> Result<(), StorageError> {
        let current_revision = self
            .client
            .query(
                "SELECT revision, deleted FROM ontology_drafts FINAL WHERE id = ? AND deleted = false LIMIT 1",
            )
            .bind(draft.id.to_string())
            .fetch_optional::<(u64, bool)>()
            .await?
            .ok_or(StorageError::NotFound)?
            .0;
        if current_revision != expected_revision {
            return Err(StorageError::RevisionConflict {
                expected: expected_revision,
                current: current_revision,
            });
        }
        self.client
            .query(
                "INSERT INTO ontology_drafts (id, workspace_id, revision, definition_json, layout_json, updated_at, deleted) SELECT toUUID(?), toUUID(?), ?, ?, ?, fromUnixTimestamp64Micro(?), false",
            )
            .bind(draft.id.to_string())
            .bind(draft.workspace_id.to_string())
            .bind(draft.revision)
            .bind(serde_json::to_string(&draft.definition)?)
            .bind(serde_json::to_string(&draft.layout)?)
            .bind(draft.updated_at.timestamp_micros())
            .execute()
            .await?;
        Ok(())
    }
}

fn parse_uuid(value: &str, field: &str) -> Result<Uuid, StorageError> {
    Uuid::parse_str(value).map_err(|error| StorageError::InvalidRecord(format!("{field}: {error}")))
}

fn timestamp_from_micros(value: i64) -> Result<DateTime<Utc>, StorageError> {
    DateTime::from_timestamp_micros(value).ok_or_else(|| {
        StorageError::InvalidRecord(format!("timestamp {value} is outside the supported range"))
    })
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredNode {
    pub workspace_id: Uuid,
    pub ontology_version_id: Uuid,
    pub object_type: String,
    pub object_id: String,
    pub properties_json: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StoredEdge {
    pub workspace_id: Uuid,
    pub ontology_version_id: Uuid,
    pub link_type: String,
    pub edge_id: String,
    pub source_type: String,
    pub source_id: String,
    pub target_type: String,
    pub target_id: String,
    pub properties_json: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphNodeWrite {
    pub workspace_id: Uuid,
    pub ontology_version_id: Uuid,
    pub object_type: String,
    pub object_id: String,
    pub source_id: Uuid,
    pub external_id: String,
    pub properties_json: String,
    pub version: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphEdgeWrite {
    pub workspace_id: Uuid,
    pub ontology_version_id: Uuid,
    pub link_type: String,
    pub edge_id: String,
    pub source_type: String,
    pub source_id: String,
    pub target_type: String,
    pub target_id: String,
    pub data_source_id: Uuid,
    pub properties_json: String,
    pub version: u64,
}

#[async_trait]
pub trait GraphRepository: Send + Sync {
    async fn write_graph(
        &self,
        nodes: &[GraphNodeWrite],
        edges: &[GraphEdgeWrite],
    ) -> Result<(), StorageError>;

    async fn query_nodes(&self, query: &GraphQuery) -> Result<Vec<StoredNode>, StorageError>;

    async fn get_node(
        &self,
        workspace_id: Uuid,
        ontology_version_id: Uuid,
        object_type: &str,
        object_id: &str,
    ) -> Result<StoredNode, StorageError>;

    async fn edges_between(
        &self,
        workspace_id: Uuid,
        ontology_version_id: Uuid,
        object_ids: &[String],
        limit: u32,
    ) -> Result<Vec<StoredEdge>, StorageError>;
}

#[derive(Clone)]
pub struct ClickHouseGraphRepository {
    client: clickhouse::Client,
}

impl ClickHouseGraphRepository {
    pub fn new(client: clickhouse::Client) -> Self {
        Self { client }
    }

    /// Fetches one active graph node scoped to a workspace and ontology version.
    ///
    /// # Errors
    ///
    /// Returns [`StorageError::NotFound`] when no node matches, or a backend error when the
    /// `ClickHouse` request fails.
    async fn insert_nodes(&self, nodes: &[GraphNodeWrite]) -> Result<(), StorageError> {
        if nodes.is_empty() {
            return Ok(());
        }
        let mut body = String::new();
        for node in nodes {
            let properties = parse_properties(&node.properties_json)?;
            body.push_str(&serde_json::to_string(&serde_json::json!({
                "workspace_id": node.workspace_id,
                "ontology_version_id": node.ontology_version_id,
                "object_type": node.object_type,
                "object_id": node.object_id,
                "source_id": node.source_id,
                "external_id": node.external_id,
                "properties": properties,
                "version": node.version,
                "deleted": false
            }))?);
            body.push('\n');
        }
        let mut insert = self
            .client
            .insert_formatted_with(
                "INSERT INTO graph_nodes (workspace_id, ontology_version_id, object_type, object_id, source_id, external_id, properties, version, deleted) FORMAT JSONEachRow",
            )
            .buffered();
        insert.write_all(body.as_bytes()).await?;
        insert.end().await?;
        Ok(())
    }

    async fn insert_edges(&self, edges: &[GraphEdgeWrite]) -> Result<(), StorageError> {
        if edges.is_empty() {
            return Ok(());
        }
        let mut body = String::new();
        for edge in edges {
            let properties = parse_properties(&edge.properties_json)?;
            body.push_str(&serde_json::to_string(&serde_json::json!({
                "workspace_id": edge.workspace_id,
                "ontology_version_id": edge.ontology_version_id,
                "link_type": edge.link_type,
                "edge_id": edge.edge_id,
                "source_type": edge.source_type,
                "source_id": edge.source_id,
                "target_type": edge.target_type,
                "target_id": edge.target_id,
                "data_source_id": edge.data_source_id,
                "properties": properties,
                "version": edge.version,
                "deleted": false
            }))?);
            body.push('\n');
        }
        let mut insert = self
            .client
            .insert_formatted_with(
                "INSERT INTO graph_edges (workspace_id, ontology_version_id, link_type, edge_id, source_type, source_id, target_type, target_id, data_source_id, properties, version, deleted) FORMAT JSONEachRow",
            )
            .buffered();
        insert.write_all(body.as_bytes()).await?;
        insert.end().await?;
        Ok(())
    }

    async fn fetch_node(
        &self,
        workspace_id: Uuid,
        ontology_version_id: Uuid,
        object_type: &str,
        object_id: &str,
    ) -> Result<StoredNode, StorageError> {
        let row = self.client
            .query("SELECT toString(workspace_id), toString(ontology_version_id), object_type, object_id, toJSONString(properties) FROM graph_nodes FINAL WHERE workspace_id = ? AND ontology_version_id = ? AND object_type = ? AND object_id = ? AND deleted = false LIMIT 1")
            .bind(workspace_id.to_string()).bind(ontology_version_id.to_string()).bind(object_type).bind(object_id)
            .fetch_optional::<(String, String, String, String, String)>().await?
            .ok_or(StorageError::NotFound)?;
        Ok(StoredNode {
            workspace_id: Uuid::parse_str(&row.0).map_err(|_| StorageError::NotFound)?,
            ontology_version_id: Uuid::parse_str(&row.1).map_err(|_| StorageError::NotFound)?,
            object_type: row.2,
            object_id: row.3,
            properties_json: row.4,
        })
    }
}

#[async_trait]
impl GraphRepository for ClickHouseGraphRepository {
    async fn write_graph(
        &self,
        nodes: &[GraphNodeWrite],
        edges: &[GraphEdgeWrite],
    ) -> Result<(), StorageError> {
        self.insert_nodes(nodes).await?;
        self.insert_edges(edges).await
    }

    async fn query_nodes(&self, query: &GraphQuery) -> Result<Vec<StoredNode>, StorageError> {
        let compiled = compile_graph_query(query)
            .map_err(|error| StorageError::InvalidRecord(error.to_string()))?;
        let mut clickhouse_query = self.client.query(&compiled.sql);
        for parameter in compiled.parameters {
            clickhouse_query = clickhouse_query.bind(parameter);
        }
        clickhouse_query
            .fetch_all::<(String, String, String)>()
            .await?
            .into_iter()
            .map(|row| {
                Ok(StoredNode {
                    workspace_id: query.workspace_id,
                    ontology_version_id: query.ontology_version_id,
                    object_id: row.0,
                    object_type: row.1,
                    properties_json: row.2,
                })
            })
            .collect()
    }

    async fn get_node(
        &self,
        workspace_id: Uuid,
        ontology_version_id: Uuid,
        object_type: &str,
        object_id: &str,
    ) -> Result<StoredNode, StorageError> {
        self.fetch_node(workspace_id, ontology_version_id, object_type, object_id)
            .await
    }

    async fn edges_between(
        &self,
        workspace_id: Uuid,
        ontology_version_id: Uuid,
        object_ids: &[String],
        limit: u32,
    ) -> Result<Vec<StoredEdge>, StorageError> {
        if object_ids.is_empty() {
            return Ok(vec![]);
        }
        let placeholders = std::iter::repeat_n("?", object_ids.len())
            .collect::<Vec<_>>()
            .join(", ");
        let sql = format!(
            "SELECT link_type, edge_id, source_type, source_id, target_type, target_id, toJSONString(properties) FROM graph_edges FINAL WHERE workspace_id = ? AND ontology_version_id = ? AND deleted = false AND source_id IN ({placeholders}) AND target_id IN ({placeholders}) ORDER BY edge_id LIMIT {}",
            limit.min(20_000)
        );
        let mut query = self
            .client
            .query(&sql)
            .bind(workspace_id.to_string())
            .bind(ontology_version_id.to_string());
        for object_id in object_ids.iter().chain(object_ids) {
            query = query.bind(object_id);
        }
        query
            .fetch_all::<(String, String, String, String, String, String, String)>()
            .await?
            .into_iter()
            .map(|row| {
                Ok(StoredEdge {
                    workspace_id,
                    ontology_version_id,
                    link_type: row.0,
                    edge_id: row.1,
                    source_type: row.2,
                    source_id: row.3,
                    target_type: row.4,
                    target_id: row.5,
                    properties_json: row.6,
                })
            })
            .collect()
    }
}

fn parse_properties(value: &str) -> Result<serde_json::Value, StorageError> {
    let value: serde_json::Value = serde_json::from_str(value)?;
    if value.is_object() {
        Ok(value)
    } else {
        Err(StorageError::InvalidRecord(
            "graph properties must be a JSON object".into(),
        ))
    }
}

type MemoryNodeKey = (Uuid, Uuid, String, String);
type MemoryEdgeKey = (Uuid, Uuid, String);

#[derive(Clone, Default)]
pub struct MemoryGraphRepository {
    nodes: Arc<RwLock<HashMap<MemoryNodeKey, GraphNodeWrite>>>,
    edges: Arc<RwLock<HashMap<MemoryEdgeKey, GraphEdgeWrite>>>,
}

#[async_trait]
impl GraphRepository for MemoryGraphRepository {
    async fn write_graph(
        &self,
        nodes: &[GraphNodeWrite],
        edges: &[GraphEdgeWrite],
    ) -> Result<(), StorageError> {
        let mut stored_nodes = self.nodes.write().await;
        for node in nodes {
            parse_properties(&node.properties_json)?;
            stored_nodes.insert(
                (
                    node.workspace_id,
                    node.ontology_version_id,
                    node.object_type.clone(),
                    node.object_id.clone(),
                ),
                node.clone(),
            );
        }
        drop(stored_nodes);
        let mut stored_edges = self.edges.write().await;
        for edge in edges {
            parse_properties(&edge.properties_json)?;
            stored_edges.insert(
                (
                    edge.workspace_id,
                    edge.ontology_version_id,
                    edge.edge_id.clone(),
                ),
                edge.clone(),
            );
        }
        Ok(())
    }

    async fn query_nodes(&self, query: &GraphQuery) -> Result<Vec<StoredNode>, StorageError> {
        compile_graph_query(query)
            .map_err(|error| StorageError::InvalidRecord(error.to_string()))?;
        let nodes = self.nodes.read().await;
        let edges = self.edges.read().await;
        let mut current = nodes
            .values()
            .filter(|node| {
                node.workspace_id == query.workspace_id
                    && node.ontology_version_id == query.ontology_version_id
                    && node.object_type == query.root_type
                    && matches_filters(&node.properties_json, &query.filters)
            })
            .map(|node| node.object_id.clone())
            .collect::<HashSet<_>>();
        for step in &query.traversal {
            let mut next = HashSet::new();
            for edge in edges.values().filter(|edge| {
                edge.workspace_id == query.workspace_id
                    && edge.ontology_version_id == query.ontology_version_id
                    && edge.link_type == step.link_type
            }) {
                let (from, to) = if step.reverse {
                    (&edge.target_id, &edge.source_id)
                } else {
                    (&edge.source_id, &edge.target_id)
                };
                if current.contains(from)
                    && nodes.values().any(|node| {
                        node.workspace_id == query.workspace_id
                            && node.ontology_version_id == query.ontology_version_id
                            && node.object_type == step.target_type
                            && node.object_id == *to
                    })
                {
                    next.insert(to.clone());
                }
            }
            current = next;
        }
        let mut result = nodes
            .values()
            .filter(|node| {
                node.workspace_id == query.workspace_id
                    && node.ontology_version_id == query.ontology_version_id
                    && current.contains(&node.object_id)
            })
            .map(stored_node_from_write)
            .collect::<Vec<_>>();
        result.sort_by(|left, right| left.object_id.cmp(&right.object_id));
        let limit = if query.limit == 0 { 500 } else { query.limit };
        result.truncate(limit.min(5_000) as usize);
        Ok(result)
    }

    async fn get_node(
        &self,
        workspace_id: Uuid,
        ontology_version_id: Uuid,
        object_type: &str,
        object_id: &str,
    ) -> Result<StoredNode, StorageError> {
        self.nodes
            .read()
            .await
            .get(&(
                workspace_id,
                ontology_version_id,
                object_type.to_owned(),
                object_id.to_owned(),
            ))
            .map(stored_node_from_write)
            .ok_or(StorageError::NotFound)
    }

    async fn edges_between(
        &self,
        workspace_id: Uuid,
        ontology_version_id: Uuid,
        object_ids: &[String],
        limit: u32,
    ) -> Result<Vec<StoredEdge>, StorageError> {
        let ids = object_ids.iter().collect::<HashSet<_>>();
        let mut result = self
            .edges
            .read()
            .await
            .values()
            .filter(|edge| {
                edge.workspace_id == workspace_id
                    && edge.ontology_version_id == ontology_version_id
                    && ids.contains(&edge.source_id)
                    && ids.contains(&edge.target_id)
            })
            .map(|edge| StoredEdge {
                workspace_id,
                ontology_version_id,
                link_type: edge.link_type.clone(),
                edge_id: edge.edge_id.clone(),
                source_type: edge.source_type.clone(),
                source_id: edge.source_id.clone(),
                target_type: edge.target_type.clone(),
                target_id: edge.target_id.clone(),
                properties_json: edge.properties_json.clone(),
            })
            .collect::<Vec<_>>();
        result.sort_by(|left, right| left.edge_id.cmp(&right.edge_id));
        result.truncate(limit.min(20_000) as usize);
        Ok(result)
    }
}

fn stored_node_from_write(node: &GraphNodeWrite) -> StoredNode {
    StoredNode {
        workspace_id: node.workspace_id,
        ontology_version_id: node.ontology_version_id,
        object_type: node.object_type.clone(),
        object_id: node.object_id.clone(),
        properties_json: node.properties_json.clone(),
    }
}

fn matches_filters(properties_json: &str, filters: &[GraphFilter]) -> bool {
    let Ok(properties) = serde_json::from_str::<serde_json::Value>(properties_json) else {
        return false;
    };
    filters.iter().all(|filter| {
        let actual = properties.get(&filter.property);
        let actual_string = actual.map_or_else(String::new, json_scalar_string);
        match filter.operator {
            FilterOperator::Equal => actual_string == filter.value,
            FilterOperator::NotEqual => actual_string != filter.value,
            FilterOperator::Contains => actual_string
                .to_lowercase()
                .contains(&filter.value.to_lowercase()),
            FilterOperator::GreaterThan => actual_string > filter.value,
            FilterOperator::LessThan => actual_string < filter.value,
        }
    })
}

fn json_scalar_string(value: &serde_json::Value) -> String {
    value
        .as_str()
        .map_or_else(|| value.to_string(), str::to_owned)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn graph_query_is_scoped_and_parameterized() {
        let query = GraphQuery {
            workspace_id: Uuid::nil(),
            ontology_version_id: Uuid::nil(),
            root_type: "service".into(),
            filters: vec![GraphFilter {
                property: "name".into(),
                operator: FilterOperator::Contains,
                value: "billing' OR 1=1".into(),
            }],
            traversal: vec![TraversalStep {
                link_type: "owned_by".into(),
                target_type: "team".into(),
                reverse: false,
            }],
            limit: 50,
        };
        let compiled = compile_graph_query(&query).unwrap();
        assert!(compiled.sql.contains("n0.workspace_id = ?"));
        assert!(!compiled.sql.contains("billing"));
        assert_eq!(compiled.parameters.last().unwrap(), "%billing' OR 1=1%");
    }

    #[test]
    fn graph_query_rejects_excessive_depth() {
        let query = GraphQuery {
            workspace_id: Uuid::nil(),
            ontology_version_id: Uuid::nil(),
            root_type: "service".into(),
            filters: vec![],
            traversal: (0..7)
                .map(|_| TraversalStep {
                    link_type: "depends_on".into(),
                    target_type: "service".into(),
                    reverse: false,
                })
                .collect(),
            limit: 10,
        };
        assert!(matches!(
            compile_graph_query(&query),
            Err(QueryError::TraversalTooDeep(7))
        ));
    }

    #[tokio::test]
    async fn clickhouse_graph_round_trip() {
        if std::env::var("CONTEXT_HUB_CLICKHOUSE_TEST").as_deref() != Ok("1") {
            return;
        }
        let client = clickhouse::Client::default()
            .with_url(
                std::env::var("CLICKHOUSE_URL").unwrap_or_else(|_| "http://localhost:8123".into()),
            )
            .with_database("context_hub")
            .with_user("context_hub")
            .with_password("context_hub")
            .with_setting("input_format_binary_read_json_as_string", "1")
            .with_setting("output_format_binary_write_json_as_string", "1");
        let repository = ClickHouseGraphRepository::new(client);
        let workspace_id = Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap();
        let ontology_version_id = Uuid::new_v4();
        let source_id = Uuid::new_v4();
        repository
            .write_graph(
                &[
                    GraphNodeWrite {
                        workspace_id,
                        ontology_version_id,
                        object_type: "service".into(),
                        object_id: "service:test".into(),
                        source_id,
                        external_id: "test".into(),
                        properties_json: r#"{"name":"Test"}"#.into(),
                        version: 1,
                    },
                    GraphNodeWrite {
                        workspace_id,
                        ontology_version_id,
                        object_type: "team".into(),
                        object_id: "team:platform".into(),
                        source_id,
                        external_id: "platform".into(),
                        properties_json: r#"{"name":"Platform"}"#.into(),
                        version: 1,
                    },
                ],
                &[GraphEdgeWrite {
                    workspace_id,
                    ontology_version_id,
                    link_type: "owned_by".into(),
                    edge_id: "owned_by:test:platform".into(),
                    source_type: "service".into(),
                    source_id: "service:test".into(),
                    target_type: "team".into(),
                    target_id: "team:platform".into(),
                    data_source_id: source_id,
                    properties_json: "{}".into(),
                    version: 1,
                }],
            )
            .await
            .expect("graph batch can be written to ClickHouse");

        let nodes = repository
            .query_nodes(&GraphQuery {
                workspace_id,
                ontology_version_id,
                root_type: "service".into(),
                filters: vec![],
                traversal: vec![],
                limit: 10,
            })
            .await
            .expect("nodes can be queried from ClickHouse");
        assert_eq!(nodes.len(), 1);
        assert_eq!(nodes[0].object_id, "service:test");
        let traversed = repository
            .query_nodes(&GraphQuery {
                workspace_id,
                ontology_version_id,
                root_type: "service".into(),
                filters: vec![],
                traversal: vec![TraversalStep {
                    link_type: "owned_by".into(),
                    target_type: "team".into(),
                    reverse: false,
                }],
                limit: 10,
            })
            .await
            .expect("ClickHouse traversal can be queried");
        assert_eq!(traversed.len(), 1);
        assert_eq!(traversed[0].object_id, "team:platform");
        let edges = repository
            .edges_between(
                workspace_id,
                ontology_version_id,
                &["service:test".into(), "team:platform".into()],
                10,
            )
            .await
            .expect("edges can be queried from ClickHouse");
        assert_eq!(edges.len(), 1);
    }
}
