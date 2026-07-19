use std::sync::Arc;

use datafusion::{
    arrow::{datatypes::SchemaRef, record_batch::RecordBatch},
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
    #[error("mapping execution failed: {0}")]
    Execution(#[from] datafusion::error::DataFusionError),
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
}
