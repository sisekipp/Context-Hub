use std::{io::Cursor, sync::Arc};

use datafusion::{
    arrow::{
        csv::reader::{Format as CsvFormat, ReaderBuilder as CsvReaderBuilder},
        datatypes::SchemaRef,
        error::ArrowError,
        json::{
            LineDelimitedWriter, ReaderBuilder as JsonReaderBuilder, reader::infer_json_schema,
        },
        record_batch::RecordBatch,
    },
    datasource::MemTable,
    prelude::SessionContext,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
pub struct MappingPlan {
    pub id: Uuid,
    pub object_type: String,
    pub identity_fields: Vec<String>,
    pub fields: Vec<FieldMapping>,
    #[serde(default)]
    pub links: Vec<LinkMapping>,
    #[serde(default)]
    pub row_filter: Option<Predicate>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
pub struct MappingBundle {
    pub id: Uuid,
    pub plans: Vec<MappingPlan>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(untagged)]
pub enum MappingDocument {
    Bundle(MappingBundle),
    Plan(MappingPlan),
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SourceFormat {
    Json,
    Ndjson,
    Csv,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MappedNode {
    pub object_type: String,
    pub object_id: String,
    pub properties_json: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MappedEdge {
    pub link_type: String,
    pub source_object_type: String,
    pub source_id: String,
    pub target_object_type: String,
    pub target_id: String,
    pub properties_json: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MappedGraphBatch {
    pub rows_read: u64,
    pub rows_rejected: u64,
    pub nodes: Vec<MappedNode>,
    pub edges: Vec<MappedEdge>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
pub struct FieldMapping {
    pub source: String,
    pub target: String,
    #[serde(default)]
    pub transforms: Vec<Transform>,
    #[serde(default)]
    pub on_error: ErrorStrategy,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
pub struct LinkMapping {
    pub link_type: String,
    pub target_object_type: String,
    pub source_fields: Vec<String>,
    pub target_identity_fields: Vec<String>,
    #[serde(default)]
    pub missing_target: MissingTargetStrategy,
}

#[derive(Debug, Clone, Default, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MissingTargetStrategy {
    #[default]
    Create,
    Skip,
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Transform {
    Cast {
        target: CastType,
    },
    Trim,
    Lowercase,
    Uppercase,
    Replace {
        from: String,
        to: String,
    },
    RegexReplace {
        pattern: String,
        replacement: String,
    },
    Default {
        value: serde_json::Value,
    },
    Coalesce {
        fields: Vec<String>,
    },
    Concat {
        fields: Vec<String>,
        separator: String,
    },
    Add {
        value: f64,
    },
    Multiply {
        value: f64,
    },
    ParseDate {
        format: String,
    },
    ParseTimestamp {
        format: String,
    },
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CastType {
    String,
    Boolean,
    Int64,
    Float64,
    Decimal,
    Date,
    Timestamp,
}

#[derive(Debug, Clone, Default, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ErrorStrategy {
    #[default]
    RejectRow,
    UseNull,
    AbortJob,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
pub struct Predicate {
    pub field: String,
    pub operator: PredicateOperator,
    pub value: serde_json::Value,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PredicateOperator {
    Equal,
    NotEqual,
    GreaterThan,
    LessThan,
    Contains,
}

#[derive(Debug, Error)]
pub enum MappingError {
    #[error("mapping plan needs at least one identity field")]
    MissingIdentity,
    #[error("mapping plan contains invalid identifier '{0}'")]
    InvalidIdentifier(String),
    #[error("mapping plan contains duplicate target '{0}'")]
    DuplicateTarget(String),
    #[error("mapping bundle needs at least one object plan")]
    EmptyBundle,
    #[error("mapping bundle contains duplicate object plan '{0}'")]
    DuplicateObjectPlan(String),
    #[error("mapping execution failed: {0}")]
    Execution(#[from] datafusion::error::DataFusionError),
    #[error("Arrow source processing failed: {0}")]
    Arrow(#[from] ArrowError),
    #[error("source JSON is invalid: {0}")]
    Json(#[from] serde_json::Error),
    #[error("source must contain object records")]
    InvalidSource,
    #[error("link '{link_type}' targets missing object '{target_id}'")]
    MissingLinkTarget {
        link_type: String,
        target_id: String,
    },
}

struct PendingEdge {
    edge: MappedEdge,
    missing_target: MissingTargetStrategy,
    target_properties_json: String,
}

impl MappingPlan {
    /// Checks identifiers, identity configuration, transform arguments, and link join keys.
    ///
    /// # Errors
    ///
    /// Returns [`MappingError`] when the plan cannot be compiled safely.
    pub fn validate(&self) -> Result<(), MappingError> {
        validate_identifier(&self.object_type)?;
        if self.identity_fields.is_empty() {
            return Err(MappingError::MissingIdentity);
        }
        for field in &self.identity_fields {
            validate_identifier(field)?;
        }
        let mut targets = std::collections::HashSet::new();
        for field in &self.fields {
            validate_identifier(&field.source)?;
            validate_identifier(&field.target)?;
            if !targets.insert(field.target.as_str()) {
                return Err(MappingError::DuplicateTarget(field.target.clone()));
            }
            for transform in &field.transforms {
                validate_transform(transform)?;
            }
        }
        for link in &self.links {
            validate_identifier(&link.link_type)?;
            validate_identifier(&link.target_object_type)?;
            for field in link
                .source_fields
                .iter()
                .chain(&link.target_identity_fields)
            {
                validate_identifier(field)?;
            }
            if link.source_fields.len() != link.target_identity_fields.len() {
                return Err(MappingError::InvalidIdentifier(format!(
                    "link '{}' has mismatched join keys",
                    link.link_type
                )));
            }
        }
        if let Some(predicate) = &self.row_filter {
            validate_identifier(&predicate.field)?;
        }
        Ok(())
    }

    /// Compiles this validated plan into the restricted SQL subset consumed by `DataFusion`.
    ///
    /// # Errors
    ///
    /// Returns [`MappingError`] when validation or expression compilation fails.
    pub fn compile_datafusion_sql(&self) -> Result<String, MappingError> {
        self.validate()?;
        let projection = self
            .fields
            .iter()
            .map(|field| {
                let mut expression = quote_identifier(&field.source);
                for transform in &field.transforms {
                    expression = compile_transform(expression, transform)?;
                }
                Ok(format!(
                    "{expression} AS {}",
                    quote_identifier(&field.target)
                ))
            })
            .collect::<Result<Vec<_>, MappingError>>()?;

        let filter = self
            .row_filter
            .as_ref()
            .map(compile_predicate)
            .transpose()?;
        Ok(match filter {
            Some(filter) => format!(
                "SELECT {} FROM source WHERE {filter}",
                projection.join(", ")
            ),
            None => format!("SELECT {} FROM source", projection.join(", ")),
        })
    }
}

impl MappingBundle {
    /// Validates every object plan and prevents ambiguous duplicate producers.
    ///
    /// # Errors
    ///
    /// Returns [`MappingError`] when the bundle is empty, contains duplicate object types, or one
    /// of its plans is invalid.
    pub fn validate(&self) -> Result<(), MappingError> {
        if self.plans.is_empty() {
            return Err(MappingError::EmptyBundle);
        }
        let mut object_types = std::collections::HashSet::new();
        for plan in &self.plans {
            plan.validate()?;
            if !object_types.insert(plan.object_type.as_str()) {
                return Err(MappingError::DuplicateObjectPlan(plan.object_type.clone()));
            }
        }
        Ok(())
    }
}

impl MappingDocument {
    #[must_use]
    pub fn plans(&self) -> &[MappingPlan] {
        match self {
            Self::Bundle(bundle) => &bundle.plans,
            Self::Plan(plan) => std::slice::from_ref(plan),
        }
    }

    /// Validates either a legacy single plan or a multi-object bundle.
    ///
    /// # Errors
    ///
    /// Returns [`MappingError`] when the contained mapping is invalid.
    pub fn validate(&self) -> Result<(), MappingError> {
        match self {
            Self::Bundle(bundle) => bundle.validate(),
            Self::Plan(plan) => plan.validate(),
        }
    }
}

/// Executes a mapping plan over in-memory Arrow record batches.
///
/// # Errors
///
/// Returns [`MappingError`] when the plan is invalid or `DataFusion` cannot execute it.
pub async fn execute_mapping(
    plan: &MappingPlan,
    schema: SchemaRef,
    batches: Vec<RecordBatch>,
) -> Result<Vec<RecordBatch>, MappingError> {
    let context = SessionContext::new();
    let table = MemTable::try_new(schema, vec![batches])?;
    context.register_table("source", Arc::new(table))?;
    let frame = context.sql(&plan.compile_datafusion_sql()?).await?;
    Ok(frame.collect().await?)
}

/// Loads a bounded source into Arrow, executes the restricted `DataFusion` mapping, and converts
/// the result into stable property-graph records for the storage worker.
///
/// # Errors
///
/// Returns [`MappingError`] when parsing, schema inference, mapping execution, or graph identity
/// construction fails.
pub async fn execute_source_mapping(
    plan: &MappingPlan,
    format: SourceFormat,
    content: &[u8],
) -> Result<MappedGraphBatch, MappingError> {
    execute_source_mapping_bundle(std::slice::from_ref(plan), format, content).await
}

/// Executes multiple object plans over one shared Arrow source and resolves their links globally.
///
/// # Errors
///
/// Returns [`MappingError`] when a plan is invalid, the source cannot be read, or a link configured
/// with [`MissingTargetStrategy::Error`] cannot be resolved by any plan in the bundle.
pub async fn execute_source_mapping_bundle(
    plans: &[MappingPlan],
    format: SourceFormat,
    content: &[u8],
) -> Result<MappedGraphBatch, MappingError> {
    validate_plan_set(plans)?;
    let batches = read_source_batches(format, content)?;
    let rows_read = batches.iter().map(RecordBatch::num_rows).sum::<usize>();
    if batches.is_empty() {
        return Ok(MappedGraphBatch::default());
    }
    let mut result = MappedGraphBatch {
        rows_read: rows_read as u64,
        ..MappedGraphBatch::default()
    };
    let mut node_positions = std::collections::HashMap::new();
    let mut pending_edges = Vec::new();
    for plan in plans {
        let (nodes, rows_rejected, edges) = map_plan_batches(plan, &batches).await?;
        result.rows_rejected += rows_rejected;
        for node in nodes {
            merge_node(&mut result.nodes, &mut node_positions, node)?;
        }
        pending_edges.extend(edges);
    }
    resolve_pending_edges(&mut result, pending_edges)?;
    let mut edge_keys = std::collections::HashSet::new();
    result.edges.retain(|edge| {
        edge_keys.insert((
            edge.source_object_type.clone(),
            edge.source_id.clone(),
            edge.link_type.clone(),
            edge.target_object_type.clone(),
            edge.target_id.clone(),
        ))
    });
    Ok(result)
}

fn validate_plan_set(plans: &[MappingPlan]) -> Result<(), MappingError> {
    if plans.is_empty() {
        return Err(MappingError::EmptyBundle);
    }
    let mut object_types = std::collections::HashSet::new();
    for plan in plans {
        plan.validate()?;
        if !object_types.insert(plan.object_type.as_str()) {
            return Err(MappingError::DuplicateObjectPlan(plan.object_type.clone()));
        }
    }
    Ok(())
}

async fn map_plan_batches(
    plan: &MappingPlan,
    batches: &[RecordBatch],
) -> Result<(Vec<MappedNode>, u64, Vec<PendingEdge>), MappingError> {
    let schema = batches[0].schema();
    let worker_plan = worker_projection(plan);
    let mapped = execute_mapping(&worker_plan, schema, batches.to_vec()).await?;
    let rows = record_batches_to_json(&mapped)?;
    let mut nodes = Vec::new();
    let mut rows_rejected = 0;
    let mut pending_edges = Vec::new();
    for row in rows {
        let Some(object) = row.as_object() else {
            rows_rejected += 1;
            continue;
        };
        let identity = (0..plan.identity_fields.len())
            .map(|index| object.get(&format!("__identity_{index}")))
            .collect::<Option<Vec<_>>>();
        let Some(identity) = identity.filter(|values| values.iter().all(|value| !value.is_null()))
        else {
            rows_rejected += 1;
            continue;
        };
        let object_id = stable_object_id(&plan.object_type, &identity);
        let properties = plan
            .fields
            .iter()
            .filter_map(|field| {
                object
                    .get(&field.target)
                    .cloned()
                    .map(|value| (field.target.clone(), value))
            })
            .collect::<serde_json::Map<_, _>>();
        nodes.push(MappedNode {
            object_type: plan.object_type.clone(),
            object_id: object_id.clone(),
            properties_json: serde_json::to_string(&properties)?,
        });
        for (link_index, link) in plan.links.iter().enumerate() {
            let target_values = (0..link.source_fields.len())
                .map(|field_index| object.get(&format!("__link_{link_index}_{field_index}")))
                .collect::<Option<Vec<_>>>();
            let Some(target_values) =
                target_values.filter(|values| values.iter().all(|value| !value.is_null()))
            else {
                continue;
            };
            for expanded in expand_link_values(&target_values) {
                let expanded = expanded.iter().collect::<Vec<_>>();
                let target_id = stable_object_id(&link.target_object_type, &expanded);
                let target_properties = link
                    .target_identity_fields
                    .iter()
                    .cloned()
                    .zip(expanded.iter().map(|value| (*value).clone()))
                    .collect::<serde_json::Map<_, _>>();
                pending_edges.push(PendingEdge {
                    edge: MappedEdge {
                        link_type: link.link_type.clone(),
                        source_object_type: plan.object_type.clone(),
                        source_id: object_id.clone(),
                        target_object_type: link.target_object_type.clone(),
                        target_id,
                        properties_json: "{}".into(),
                    },
                    missing_target: link.missing_target,
                    target_properties_json: serde_json::to_string(&target_properties)?,
                });
            }
        }
    }
    Ok((nodes, rows_rejected, pending_edges))
}

fn merge_node(
    nodes: &mut Vec<MappedNode>,
    positions: &mut std::collections::HashMap<(String, String), usize>,
    node: MappedNode,
) -> Result<(), MappingError> {
    let key = (node.object_type.clone(), node.object_id.clone());
    if let Some(index) = positions.get(&key).copied() {
        let existing = serde_json::from_str::<serde_json::Map<String, serde_json::Value>>(
            &nodes[index].properties_json,
        )?;
        let incoming = serde_json::from_str::<serde_json::Map<String, serde_json::Value>>(
            &node.properties_json,
        )?;
        let mut merged = existing;
        merged.extend(incoming);
        nodes[index].properties_json = serde_json::to_string(&merged)?;
    } else {
        positions.insert(key, nodes.len());
        nodes.push(node);
    }
    Ok(())
}

fn resolve_pending_edges(
    result: &mut MappedGraphBatch,
    pending_edges: Vec<PendingEdge>,
) -> Result<(), MappingError> {
    let mut known_nodes = result
        .nodes
        .iter()
        .map(|node| (node.object_type.clone(), node.object_id.clone()))
        .collect::<std::collections::HashSet<_>>();
    for pending in pending_edges {
        let target_key = (
            pending.edge.target_object_type.clone(),
            pending.edge.target_id.clone(),
        );
        if !known_nodes.contains(&target_key) {
            match pending.missing_target {
                MissingTargetStrategy::Create => {
                    known_nodes.insert(target_key);
                    result.nodes.push(MappedNode {
                        object_type: pending.edge.target_object_type.clone(),
                        object_id: pending.edge.target_id.clone(),
                        properties_json: pending.target_properties_json,
                    });
                }
                MissingTargetStrategy::Skip => continue,
                MissingTargetStrategy::Error => {
                    return Err(MappingError::MissingLinkTarget {
                        link_type: pending.edge.link_type,
                        target_id: pending.edge.target_id,
                    });
                }
            }
        }
        result.edges.push(pending.edge);
    }
    Ok(())
}

fn worker_projection(plan: &MappingPlan) -> MappingPlan {
    let mut worker = plan.clone();
    for (index, source) in plan.identity_fields.iter().enumerate() {
        let transforms = plan
            .fields
            .iter()
            .find(|field| field.source == *source)
            .map(|field| field.transforms.clone())
            .unwrap_or_default();
        worker.fields.push(FieldMapping {
            source: source.clone(),
            target: format!("__identity_{index}"),
            transforms,
            on_error: ErrorStrategy::RejectRow,
        });
    }
    for (link_index, link) in plan.links.iter().enumerate() {
        for (field_index, source) in link.source_fields.iter().enumerate() {
            worker.fields.push(FieldMapping {
                source: source.clone(),
                target: format!("__link_{link_index}_{field_index}"),
                transforms: vec![],
                on_error: ErrorStrategy::RejectRow,
            });
        }
    }
    worker
}

fn read_source_batches(
    format: SourceFormat,
    content: &[u8],
) -> Result<Vec<RecordBatch>, MappingError> {
    match format {
        SourceFormat::Json | SourceFormat::Ndjson => {
            let data = if format == SourceFormat::Json {
                normalize_json_records(content)?
            } else {
                content.to_vec()
            };
            let mut inference = Cursor::new(&data);
            let (schema, _) = infer_json_schema(&mut inference, None)?;
            let reader = JsonReaderBuilder::new(Arc::new(schema))
                .with_batch_size(8_192)
                .build(Cursor::new(data))?;
            reader.collect::<Result<Vec<_>, _>>().map_err(Into::into)
        }
        SourceFormat::Csv => {
            let format = CsvFormat::default().with_header(true);
            let mut inference = Cursor::new(content);
            let (schema, _) = format.infer_schema(&mut inference, Some(1_000))?;
            let reader = CsvReaderBuilder::new(Arc::new(schema))
                .with_format(format)
                .with_batch_size(8_192)
                .build(Cursor::new(content))?;
            reader.collect::<Result<Vec<_>, _>>().map_err(Into::into)
        }
    }
}

fn normalize_json_records(content: &[u8]) -> Result<Vec<u8>, MappingError> {
    let value: serde_json::Value = serde_json::from_slice(content)?;
    let records = match value {
        serde_json::Value::Array(records) => records,
        serde_json::Value::Object(mut object) => ["records", "data", "items", "results"]
            .into_iter()
            .find_map(|key| {
                object
                    .remove(key)
                    .and_then(|value| value.as_array().cloned())
            })
            .unwrap_or_else(|| vec![serde_json::Value::Object(object)]),
        _ => return Err(MappingError::InvalidSource),
    };
    if records.iter().any(|record| !record.is_object()) {
        return Err(MappingError::InvalidSource);
    }
    let mut output = Vec::new();
    for record in records {
        serde_json::to_writer(&mut output, &record)?;
        output.push(b'\n');
    }
    Ok(output)
}

fn record_batches_to_json(batches: &[RecordBatch]) -> Result<Vec<serde_json::Value>, MappingError> {
    let mut output = Vec::new();
    {
        let mut writer = LineDelimitedWriter::new(&mut output);
        for batch in batches {
            writer.write(batch)?;
        }
        writer.finish()?;
    }
    output
        .split(|byte| *byte == b'\n')
        .filter(|line| !line.is_empty())
        .map(serde_json::from_slice)
        .collect::<Result<Vec<_>, _>>()
        .map_err(Into::into)
}

fn stable_object_id(object_type: &str, values: &[&serde_json::Value]) -> String {
    let identity = values
        .iter()
        .map(|value| match value {
            serde_json::Value::String(value) => value.clone(),
            other => other.to_string(),
        })
        .collect::<Vec<_>>()
        .join("|");
    format!("{object_type}:{identity}")
}

fn expand_link_values(values: &[&serde_json::Value]) -> Vec<Vec<serde_json::Value>> {
    let array_length = values
        .iter()
        .filter_map(|value| value.as_array().map(Vec::len))
        .max();
    let length = array_length.unwrap_or(1);
    (0..length)
        .filter_map(|index| {
            values
                .iter()
                .map(|value| match value {
                    serde_json::Value::Array(items) => items.get(index).cloned(),
                    scalar => Some((*scalar).clone()),
                })
                .collect::<Option<Vec<_>>>()
                .filter(|expanded| expanded.iter().all(|value| !value.is_null()))
        })
        .collect()
}

fn compile_transform(input: String, transform: &Transform) -> Result<String, MappingError> {
    let sql = match transform {
        Transform::Cast { target } => format!("CAST({input} AS {})", cast_type(*target)),
        Transform::Trim => format!("btrim({input})"),
        Transform::Lowercase => format!("lower({input})"),
        Transform::Uppercase => format!("upper({input})"),
        Transform::Replace { from, to } => {
            format!("replace({input}, {}, {})", sql_string(from), sql_string(to))
        }
        Transform::RegexReplace {
            pattern,
            replacement,
        } => format!(
            "regexp_replace({input}, {}, {}, 'g')",
            sql_string(pattern),
            sql_string(replacement)
        ),
        Transform::Default { value } => format!("coalesce({input}, {})", sql_literal(value)),
        Transform::Coalesce { fields } => {
            for field in fields {
                validate_identifier(field)?;
            }
            let others = fields
                .iter()
                .map(|field| quote_identifier(field))
                .collect::<Vec<_>>()
                .join(", ");
            if others.is_empty() {
                input
            } else {
                format!("coalesce({input}, {others})")
            }
        }
        Transform::Concat { fields, separator } => {
            for field in fields {
                validate_identifier(field)?;
            }
            let mut values = vec![input];
            values.extend(fields.iter().map(|field| quote_identifier(field)));
            format!(
                "array_join(make_array({}), {})",
                values.join(", "),
                sql_string(separator)
            )
        }
        Transform::Add { value } => format!("({input} + {value})"),
        Transform::Multiply { value } => format!("({input} * {value})"),
        Transform::ParseDate { format } => {
            format!("to_date(to_timestamp({input}, {}))", sql_string(format))
        }
        Transform::ParseTimestamp { format } => {
            format!("to_timestamp({input}, {})", sql_string(format))
        }
    };
    Ok(sql)
}

fn compile_predicate(predicate: &Predicate) -> Result<String, MappingError> {
    validate_identifier(&predicate.field)?;
    let field = quote_identifier(&predicate.field);
    let value = sql_literal(&predicate.value);
    Ok(match predicate.operator {
        PredicateOperator::Equal => format!("{field} = {value}"),
        PredicateOperator::NotEqual => format!("{field} <> {value}"),
        PredicateOperator::GreaterThan => format!("{field} > {value}"),
        PredicateOperator::LessThan => format!("{field} < {value}"),
        PredicateOperator::Contains => {
            format!("strpos(CAST({field} AS VARCHAR), CAST({value} AS VARCHAR)) > 0")
        }
    })
}

fn validate_transform(transform: &Transform) -> Result<(), MappingError> {
    match transform {
        Transform::Coalesce { fields } | Transform::Concat { fields, .. } => {
            for field in fields {
                validate_identifier(field)?;
            }
        }
        _ => {}
    }
    Ok(())
}

fn validate_identifier(value: &str) -> Result<(), MappingError> {
    let valid = !value.is_empty()
        && value.len() <= 128
        && value
            .chars()
            .next()
            .is_some_and(|character| character.is_ascii_alphabetic() || character == '_')
        && value
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || character == '_');
    if valid {
        Ok(())
    } else {
        Err(MappingError::InvalidIdentifier(value.to_owned()))
    }
}

fn quote_identifier(value: &str) -> String {
    format!("\"{}\"", value.replace('"', "\"\""))
}
fn sql_string(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

fn sql_literal(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::Null => "NULL".into(),
        serde_json::Value::Bool(value) => value.to_string().to_uppercase(),
        serde_json::Value::Number(value) => value.to_string(),
        serde_json::Value::String(value) => sql_string(value),
        other => sql_string(&other.to_string()),
    }
}

const fn cast_type(value: CastType) -> &'static str {
    match value {
        CastType::String => "VARCHAR",
        CastType::Boolean => "BOOLEAN",
        CastType::Int64 => "BIGINT",
        CastType::Float64 => "DOUBLE",
        CastType::Decimal => "DECIMAL(38, 9)",
        CastType::Date => "DATE",
        CastType::Timestamp => "TIMESTAMP",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compiles_a_safe_projection() {
        let plan = MappingPlan {
            id: Uuid::nil(),
            object_type: "service".into(),
            identity_fields: vec!["service_id".into()],
            fields: vec![FieldMapping {
                source: "service_name".into(),
                target: "name".into(),
                transforms: vec![Transform::Trim, Transform::Lowercase],
                on_error: ErrorStrategy::RejectRow,
            }],
            links: vec![],
            row_filter: Some(Predicate {
                field: "active".into(),
                operator: PredicateOperator::Equal,
                value: true.into(),
            }),
        };
        assert_eq!(
            plan.compile_datafusion_sql().unwrap(),
            "SELECT lower(btrim(\"service_name\")) AS \"name\" FROM source WHERE \"active\" = TRUE"
        );
    }

    #[test]
    fn rejects_identifier_injection() {
        let plan = MappingPlan {
            id: Uuid::nil(),
            object_type: "service; drop table".into(),
            identity_fields: vec!["id".into()],
            fields: vec![],
            links: vec![],
            row_filter: None,
        };
        assert!(matches!(
            plan.validate(),
            Err(MappingError::InvalidIdentifier(_))
        ));
    }

    #[tokio::test]
    async fn maps_json_records_to_nodes_and_links() {
        let mut plan = MappingPlan {
            id: Uuid::nil(),
            object_type: "service".into(),
            identity_fields: vec!["service_id".into()],
            fields: vec![
                FieldMapping {
                    source: "service_id".into(),
                    target: "id".into(),
                    transforms: vec![Transform::Trim],
                    on_error: ErrorStrategy::RejectRow,
                },
                FieldMapping {
                    source: "service_name".into(),
                    target: "name".into(),
                    transforms: vec![Transform::Trim],
                    on_error: ErrorStrategy::RejectRow,
                },
            ],
            links: vec![LinkMapping {
                link_type: "owned_by".into(),
                target_object_type: "team".into(),
                source_fields: vec!["team_ids".into()],
                target_identity_fields: vec!["id".into()],
                missing_target: MissingTargetStrategy::Create,
            }],
            row_filter: None,
        };
        let mapped = execute_source_mapping(
            &plan,
            SourceFormat::Json,
            br#"[{"service_id":" billing ","service_name":" Billing API ","team_ids":["payments","platform"]}]"#,
        )
        .await
        .expect("JSON records can be mapped through DataFusion");

        assert_eq!(mapped.rows_read, 1);
        assert_eq!(mapped.rows_rejected, 0);
        assert_eq!(mapped.nodes[0].object_id, "service:billing");
        assert_eq!(mapped.nodes.len(), 3);
        assert!(mapped.nodes[0].properties_json.contains("Billing API"));
        assert_eq!(mapped.edges.len(), 2);
        assert_eq!(mapped.edges[0].source_id, "service:billing");
        assert_eq!(mapped.edges[0].target_id, "team:payments");
        assert_eq!(mapped.edges[1].target_id, "team:platform");
        assert_eq!(mapped.nodes[1].object_id, "team:payments");
        assert_eq!(mapped.nodes[2].object_id, "team:platform");

        plan.links[0].missing_target = MissingTargetStrategy::Skip;
        let skipped = execute_source_mapping(
            &plan,
            SourceFormat::Json,
            br#"[{"service_id":"billing","service_name":"Billing API","team_ids":["payments"]}]"#,
        )
        .await
        .expect("missing targets can be skipped");
        assert_eq!(skipped.nodes.len(), 1);
        assert!(skipped.edges.is_empty());

        plan.links[0].missing_target = MissingTargetStrategy::Error;
        let error = execute_source_mapping(
            &plan,
            SourceFormat::Json,
            br#"[{"service_id":"billing","service_name":"Billing API","team_ids":["payments"]}]"#,
        )
        .await
        .expect_err("missing targets can fail the mapping");
        assert!(matches!(error, MappingError::MissingLinkTarget { .. }));
    }

    #[tokio::test]
    async fn maps_multiple_object_types_and_resolves_links_globally() {
        let service_plan = MappingPlan {
            id: Uuid::new_v4(),
            object_type: "service".into(),
            identity_fields: vec!["service_id".into()],
            fields: vec![
                FieldMapping {
                    source: "service_id".into(),
                    target: "id".into(),
                    transforms: vec![],
                    on_error: ErrorStrategy::RejectRow,
                },
                FieldMapping {
                    source: "service_name".into(),
                    target: "name".into(),
                    transforms: vec![Transform::Trim],
                    on_error: ErrorStrategy::RejectRow,
                },
            ],
            links: vec![LinkMapping {
                link_type: "owned_by".into(),
                target_object_type: "team".into(),
                source_fields: vec!["team_id".into()],
                target_identity_fields: vec!["id".into()],
                missing_target: MissingTargetStrategy::Error,
            }],
            row_filter: None,
        };
        let team_plan = MappingPlan {
            id: Uuid::new_v4(),
            object_type: "team".into(),
            identity_fields: vec!["team_id".into()],
            fields: vec![
                FieldMapping {
                    source: "team_id".into(),
                    target: "id".into(),
                    transforms: vec![],
                    on_error: ErrorStrategy::RejectRow,
                },
                FieldMapping {
                    source: "team_name".into(),
                    target: "name".into(),
                    transforms: vec![Transform::Trim],
                    on_error: ErrorStrategy::RejectRow,
                },
            ],
            links: vec![],
            row_filter: None,
        };
        let plans = vec![service_plan, team_plan];

        let mapped = execute_source_mapping_bundle(
            &plans,
            SourceFormat::Json,
            br#"[
                {"service_id":"billing","service_name":" Billing API ","team_id":"payments","team_name":"Payments"},
                {"service_id":"search","service_name":"Search API","team_id":"platform","team_name":"Platform"}
            ]"#,
        )
        .await
        .expect("a target produced by another plan satisfies the strict link strategy");

        assert_eq!(
            mapped.rows_read, 2,
            "the shared source is counted only once"
        );
        assert_eq!(mapped.rows_rejected, 0);
        assert_eq!(mapped.nodes.len(), 4);
        assert_eq!(mapped.edges.len(), 2);
        assert!(
            mapped
                .edges
                .iter()
                .all(|edge| edge.source_object_type == "service")
        );
        let payments = mapped
            .nodes
            .iter()
            .find(|node| node.object_id == "team:payments")
            .expect("the team plan creates the link target");
        assert!(payments.properties_json.contains("Payments"));
    }

    #[test]
    fn reads_legacy_plans_and_mapping_bundles() {
        let plan = MappingPlan {
            id: Uuid::new_v4(),
            object_type: "service".into(),
            identity_fields: vec!["id".into()],
            fields: vec![],
            links: vec![],
            row_filter: None,
        };
        let legacy: MappingDocument =
            serde_json::from_value(serde_json::to_value(&plan).unwrap()).unwrap();
        assert_eq!(legacy.plans(), std::slice::from_ref(&plan));

        let bundle = MappingBundle {
            id: Uuid::new_v4(),
            plans: vec![plan],
        };
        let document: MappingDocument =
            serde_json::from_value(serde_json::to_value(&bundle).unwrap()).unwrap();
        assert_eq!(document.plans(), bundle.plans);
    }
}
