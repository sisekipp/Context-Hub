use std::fmt::Write as _;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use context_hub_domain::OntologyDraft;
use serde::{Deserialize, Serialize};
use thiserror::Error;
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
        let (from, to) = if step.reverse {
            ("target_id", "source_id")
        } else {
            ("source_id", "target_id")
        };
        write!(&mut joins, " JOIN graph_edges FINAL AS {edge} ON {edge}.workspace_id = {current}.workspace_id AND {edge}.ontology_version_id = {current}.ontology_version_id AND {edge}.{from} = {current}.object_id AND {edge}.link_type = ? AND {edge}.deleted = false")
            .expect("writing to a String cannot fail");
        write!(&mut joins, " JOIN graph_nodes FINAL AS {next} ON {next}.workspace_id = {edge}.workspace_id AND {next}.ontology_version_id = {edge}.ontology_version_id AND {next}.object_id = {edge}.{to} AND {next}.object_type = ? AND {next}.deleted = false")
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
        "SELECT {current}.object_id, {current}.object_type, toJSONString({current}.properties) FROM graph_nodes FINAL AS n0{joins} WHERE n0.workspace_id = ? AND n0.ontology_version_id = ? AND n0.object_type = ? AND n0.deleted = false{filters} ORDER BY {current}.object_id LIMIT {limit}"
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
    pub async fn get_node(
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
}
