use std::collections::{HashMap, HashSet};

use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ScalarType {
    String,
    Boolean,
    Int64,
    Float64,
    Decimal,
    Date,
    Timestamp,
    Uuid,
    Enum { values: Vec<String> },
    Json,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub struct ValueType {
    pub scalar: ScalarType,
    #[serde(default)]
    pub list: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
pub struct ValueTypeDefinition {
    pub api_name: String,
    pub display_name: String,
    pub description: Option<String>,
    pub base_type: ScalarType,
    pub unit: Option<String>,
    pub minimum: Option<f64>,
    pub maximum: Option<f64>,
    pub pattern: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TypeReference {
    Scalar { value_type: ValueType },
    ValueType { api_name: String, list: bool },
    Struct { api_name: String, list: bool },
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub struct StructFieldDefinition {
    pub api_name: String,
    pub display_name: String,
    pub value_type: TypeReference,
    #[serde(default)]
    pub required: bool,
    pub description: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub struct StructTypeDefinition {
    pub api_name: String,
    pub display_name: String,
    pub description: Option<String>,
    #[serde(default)]
    pub fields: Vec<StructFieldDefinition>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[allow(clippy::struct_excessive_bools)]
pub struct PropertyDefinition {
    pub api_name: String,
    pub display_name: String,
    pub value_type: ValueType,
    #[serde(default)]
    pub required: bool,
    #[serde(default)]
    pub unique: bool,
    #[serde(default)]
    pub identity: bool,
    #[serde(default)]
    pub indexed: bool,
    pub description: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub struct SharedPropertyDefinition {
    pub api_name: String,
    pub display_name: String,
    pub value_type: TypeReference,
    pub description: Option<String>,
    #[serde(default)]
    pub required: bool,
    #[serde(default)]
    pub indexed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub struct DerivedPropertyDefinition {
    pub api_name: String,
    pub display_name: String,
    pub value_type: TypeReference,
    /// A controlled expression compiled by the backend. This is not arbitrary SQL.
    pub expression: String,
    pub description: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub struct ObjectTypeDefinition {
    pub api_name: String,
    pub display_name: String,
    pub description: Option<String>,
    #[serde(default)]
    pub properties: Vec<PropertyDefinition>,
    #[serde(default)]
    pub shared_properties: Vec<String>,
    #[serde(default)]
    pub derived_properties: Vec<DerivedPropertyDefinition>,
    #[serde(default)]
    pub implements: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub struct InterfaceDefinition {
    pub api_name: String,
    pub display_name: String,
    pub description: Option<String>,
    #[serde(default)]
    pub properties: Vec<PropertyDefinition>,
    #[serde(default)]
    pub shared_properties: Vec<String>,
    #[serde(default)]
    pub extends: Vec<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Cardinality {
    One,
    Many,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub struct LinkTypeDefinition {
    pub api_name: String,
    pub display_name: String,
    pub source_type: String,
    pub target_type: String,
    pub source_cardinality: Cardinality,
    pub target_cardinality: Cardinality,
    #[serde(default)]
    pub required: bool,
    #[serde(default)]
    pub properties: Vec<PropertyDefinition>,
    pub description: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub struct FunctionParameterDefinition {
    pub api_name: String,
    pub display_name: String,
    pub value_type: TypeReference,
    #[serde(default)]
    pub required: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum FunctionImplementation {
    Expression {
        expression: String,
    },
    ExternalGrpc {
        endpoint: String,
        method: String,
    },
    Wasm {
        artifact_uri: String,
        entrypoint: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub struct FunctionDefinition {
    pub api_name: String,
    pub display_name: String,
    pub description: Option<String>,
    #[serde(default)]
    pub inputs: Vec<FunctionParameterDefinition>,
    pub output: TypeReference,
    pub implementation: FunctionImplementation,
    /// V1 functions are query-only and cannot mutate ontology data.
    #[serde(default = "default_true")]
    pub read_only: bool,
}

const fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
pub struct OntologyDefinition {
    pub api_name: String,
    pub display_name: String,
    pub description: Option<String>,
    #[serde(default)]
    pub object_types: Vec<ObjectTypeDefinition>,
    #[serde(default)]
    pub link_types: Vec<LinkTypeDefinition>,
    #[serde(default)]
    pub interfaces: Vec<InterfaceDefinition>,
    #[serde(default)]
    pub value_types: Vec<ValueTypeDefinition>,
    #[serde(default)]
    pub struct_types: Vec<StructTypeDefinition>,
    #[serde(default)]
    pub shared_properties: Vec<SharedPropertyDefinition>,
    #[serde(default)]
    pub functions: Vec<FunctionDefinition>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct OntologyDraft {
    pub id: Uuid,
    pub workspace_id: Uuid,
    pub revision: u64,
    pub definition: OntologyDefinition,
    #[serde(default)]
    pub layout: serde_json::Value,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub struct ValidationIssue {
    pub path: String,
    pub code: String,
    pub message: String,
}

#[derive(Debug, Error)]
pub enum DraftError {
    #[error("draft revision conflict: expected {expected}, current {current}")]
    RevisionConflict { expected: u64, current: u64 },
    #[error("ontology validation failed with {0} issue(s)")]
    ValidationFailed(usize),
}

impl OntologyDraft {
    /// Replaces the editable definition when the caller still owns the current revision.
    ///
    /// # Errors
    ///
    /// Returns [`DraftError::RevisionConflict`] when another writer saved the draft first.
    pub fn update(
        &mut self,
        expected_revision: u64,
        definition: OntologyDefinition,
        layout: serde_json::Value,
    ) -> Result<(), DraftError> {
        if expected_revision != self.revision {
            return Err(DraftError::RevisionConflict {
                expected: expected_revision,
                current: self.revision,
            });
        }
        self.definition = definition;
        self.layout = layout;
        self.revision += 1;
        self.updated_at = Utc::now();
        Ok(())
    }
}

impl OntologyDefinition {
    pub fn validate(&self) -> Vec<ValidationIssue> {
        let mut issues = Vec::new();
        validate_api_name("api_name", &self.api_name, &mut issues);

        let object_names: HashSet<_> = self
            .object_types
            .iter()
            .map(|item| item.api_name.as_str())
            .collect();
        let interface_names: HashSet<_> = self
            .interfaces
            .iter()
            .map(|item| item.api_name.as_str())
            .collect();
        let value_type_names: HashSet<_> = self
            .value_types
            .iter()
            .map(|item| item.api_name.as_str())
            .collect();
        let struct_type_names: HashSet<_> = self
            .struct_types
            .iter()
            .map(|item| item.api_name.as_str())
            .collect();
        let shared_property_names: HashSet<_> = self
            .shared_properties
            .iter()
            .map(|item| item.api_name.as_str())
            .collect();
        let mut all_type_names = HashSet::new();

        for (index, value_type) in self.value_types.iter().enumerate() {
            let path = format!("value_types[{index}]");
            validate_api_name(
                &format!("{path}.api_name"),
                &value_type.api_name,
                &mut issues,
            );
            if !all_type_names.insert(value_type.api_name.as_str()) {
                duplicate(
                    &format!("{path}.api_name"),
                    &value_type.api_name,
                    &mut issues,
                );
            }
            if value_type
                .minimum
                .zip(value_type.maximum)
                .is_some_and(|(min, max)| min > max)
            {
                issues.push(issue(
                    path,
                    "invalid_range",
                    "minimum must not exceed maximum",
                ));
            }
        }

        for (index, struct_type) in self.struct_types.iter().enumerate() {
            let path = format!("struct_types[{index}]");
            validate_api_name(
                &format!("{path}.api_name"),
                &struct_type.api_name,
                &mut issues,
            );
            if !all_type_names.insert(struct_type.api_name.as_str()) {
                duplicate(
                    &format!("{path}.api_name"),
                    &struct_type.api_name,
                    &mut issues,
                );
            }
            let mut field_names = HashSet::new();
            for (field_index, field) in struct_type.fields.iter().enumerate() {
                let field_path = format!("{path}.fields[{field_index}]");
                validate_api_name(
                    &format!("{field_path}.api_name"),
                    &field.api_name,
                    &mut issues,
                );
                if !field_names.insert(field.api_name.as_str()) {
                    duplicate(
                        &format!("{field_path}.api_name"),
                        &field.api_name,
                        &mut issues,
                    );
                }
                validate_type_reference(
                    &field_path,
                    &field.value_type,
                    &value_type_names,
                    &struct_type_names,
                    &mut issues,
                );
            }
        }

        let mut shared_names = HashSet::new();
        for (index, property) in self.shared_properties.iter().enumerate() {
            let path = format!("shared_properties[{index}]");
            validate_api_name(&format!("{path}.api_name"), &property.api_name, &mut issues);
            if !shared_names.insert(property.api_name.as_str()) {
                duplicate(&format!("{path}.api_name"), &property.api_name, &mut issues);
            }
            validate_type_reference(
                &path,
                &property.value_type,
                &value_type_names,
                &struct_type_names,
                &mut issues,
            );
        }

        for (index, object) in self.object_types.iter().enumerate() {
            let path = format!("object_types[{index}]");
            validate_api_name(&format!("{path}.api_name"), &object.api_name, &mut issues);
            if !all_type_names.insert(object.api_name.as_str()) {
                duplicate(&format!("{path}.api_name"), &object.api_name, &mut issues);
            }
            validate_properties(&path, &object.properties, true, &mut issues);
            validate_shared_property_refs(
                &path,
                &object.shared_properties,
                &shared_property_names,
                &mut issues,
            );
            for (derived_index, derived) in object.derived_properties.iter().enumerate() {
                let derived_path = format!("{path}.derived_properties[{derived_index}]");
                validate_api_name(
                    &format!("{derived_path}.api_name"),
                    &derived.api_name,
                    &mut issues,
                );
                validate_type_reference(
                    &derived_path,
                    &derived.value_type,
                    &value_type_names,
                    &struct_type_names,
                    &mut issues,
                );
                if derived.expression.trim().is_empty() {
                    issues.push(issue(
                        format!("{derived_path}.expression"),
                        "empty_expression",
                        "derived properties need an expression",
                    ));
                }
            }
            for interface in &object.implements {
                if !interface_names.contains(interface.as_str()) {
                    issues.push(issue(
                        format!("{path}.implements"),
                        "unknown_interface",
                        format!("interface '{interface}' does not exist"),
                    ));
                }
            }
        }

        let mut interface_graph = HashMap::new();
        for (index, interface) in self.interfaces.iter().enumerate() {
            let path = format!("interfaces[{index}]");
            validate_api_name(
                &format!("{path}.api_name"),
                &interface.api_name,
                &mut issues,
            );
            if !all_type_names.insert(interface.api_name.as_str()) {
                duplicate(
                    &format!("{path}.api_name"),
                    &interface.api_name,
                    &mut issues,
                );
            }
            validate_properties(&path, &interface.properties, false, &mut issues);
            validate_shared_property_refs(
                &path,
                &interface.shared_properties,
                &shared_property_names,
                &mut issues,
            );
            for parent in &interface.extends {
                if !interface_names.contains(parent.as_str()) {
                    issues.push(issue(
                        format!("{path}.extends"),
                        "unknown_interface",
                        format!("interface '{parent}' does not exist"),
                    ));
                }
            }
            interface_graph.insert(
                interface.api_name.as_str(),
                interface
                    .extends
                    .iter()
                    .map(String::as_str)
                    .collect::<Vec<_>>(),
            );
        }
        detect_interface_cycles(&interface_graph, &mut issues);

        let mut link_names = HashSet::new();
        for (index, link) in self.link_types.iter().enumerate() {
            let path = format!("link_types[{index}]");
            validate_api_name(&format!("{path}.api_name"), &link.api_name, &mut issues);
            if !link_names.insert(link.api_name.as_str()) {
                duplicate(&format!("{path}.api_name"), &link.api_name, &mut issues);
            }
            if !object_names.contains(link.source_type.as_str())
                && !interface_names.contains(link.source_type.as_str())
            {
                issues.push(issue(
                    format!("{path}.source_type"),
                    "unknown_type",
                    format!("type '{}' does not exist", link.source_type),
                ));
            }
            if !object_names.contains(link.target_type.as_str())
                && !interface_names.contains(link.target_type.as_str())
            {
                issues.push(issue(
                    format!("{path}.target_type"),
                    "unknown_type",
                    format!("type '{}' does not exist", link.target_type),
                ));
            }
            validate_properties(&path, &link.properties, false, &mut issues);
        }

        let mut function_names = HashSet::new();
        for (index, function) in self.functions.iter().enumerate() {
            let path = format!("functions[{index}]");
            validate_api_name(&format!("{path}.api_name"), &function.api_name, &mut issues);
            if !function_names.insert(function.api_name.as_str()) {
                duplicate(&format!("{path}.api_name"), &function.api_name, &mut issues);
            }
            if !function.read_only {
                issues.push(issue(
                    format!("{path}.read_only"),
                    "actions_not_supported",
                    "V1 functions must be read-only",
                ));
            }
            let mut parameter_names = HashSet::new();
            for (parameter_index, parameter) in function.inputs.iter().enumerate() {
                let parameter_path = format!("{path}.inputs[{parameter_index}]");
                validate_api_name(
                    &format!("{parameter_path}.api_name"),
                    &parameter.api_name,
                    &mut issues,
                );
                if !parameter_names.insert(parameter.api_name.as_str()) {
                    duplicate(
                        &format!("{parameter_path}.api_name"),
                        &parameter.api_name,
                        &mut issues,
                    );
                }
                validate_type_reference(
                    &parameter_path,
                    &parameter.value_type,
                    &value_type_names,
                    &struct_type_names,
                    &mut issues,
                );
            }
            validate_type_reference(
                &format!("{path}.output"),
                &function.output,
                &value_type_names,
                &struct_type_names,
                &mut issues,
            );
        }
        issues
    }
}

fn validate_shared_property_refs(
    path: &str,
    references: &[String],
    known: &HashSet<&str>,
    issues: &mut Vec<ValidationIssue>,
) {
    for reference in references {
        if !known.contains(reference.as_str()) {
            issues.push(issue(
                format!("{path}.shared_properties"),
                "unknown_shared_property",
                format!("shared property '{reference}' does not exist"),
            ));
        }
    }
}

fn validate_type_reference(
    path: &str,
    value: &TypeReference,
    value_types: &HashSet<&str>,
    struct_types: &HashSet<&str>,
    issues: &mut Vec<ValidationIssue>,
) {
    match value {
        TypeReference::Scalar { value_type } => {
            if let ScalarType::Enum { values } = &value_type.scalar {
                if values.is_empty() {
                    issues.push(issue(
                        path,
                        "empty_enum",
                        "enum types need at least one value",
                    ));
                }
            }
        }
        TypeReference::ValueType { api_name, .. } if !value_types.contains(api_name.as_str()) => {
            issues.push(issue(
                path,
                "unknown_value_type",
                format!("value type '{api_name}' does not exist"),
            ));
        }
        TypeReference::Struct { api_name, .. } if !struct_types.contains(api_name.as_str()) => {
            issues.push(issue(
                path,
                "unknown_struct_type",
                format!("struct type '{api_name}' does not exist"),
            ));
        }
        _ => {}
    }
}

fn validate_properties(
    path: &str,
    properties: &[PropertyDefinition],
    require_identity: bool,
    issues: &mut Vec<ValidationIssue>,
) {
    let mut names = HashSet::new();
    let mut identity_count = 0;
    for (index, property) in properties.iter().enumerate() {
        let property_path = format!("{path}.properties[{index}]");
        validate_api_name(
            &format!("{property_path}.api_name"),
            &property.api_name,
            issues,
        );
        if !names.insert(property.api_name.as_str()) {
            duplicate(
                &format!("{property_path}.api_name"),
                &property.api_name,
                issues,
            );
        }
        if property.identity {
            identity_count += 1;
            if !property.required || property.value_type.list {
                issues.push(issue(
                    property_path.clone(),
                    "invalid_identity",
                    "identity properties must be required scalar values",
                ));
            }
        }
        if let ScalarType::Enum { values } = &property.value_type.scalar {
            if values.is_empty() {
                issues.push(issue(
                    format!("{property_path}.value_type"),
                    "empty_enum",
                    "enum properties need at least one value",
                ));
            }
            if values.iter().collect::<HashSet<_>>().len() != values.len() {
                issues.push(issue(
                    format!("{property_path}.value_type"),
                    "duplicate_enum_value",
                    "enum values must be unique",
                ));
            }
        }
    }
    if require_identity && identity_count == 0 {
        issues.push(issue(
            format!("{path}.properties"),
            "missing_identity",
            "object types need at least one identity property",
        ));
    }
}

fn validate_api_name(path: &str, value: &str, issues: &mut Vec<ValidationIssue>) {
    let valid = !value.is_empty()
        && value.len() <= 64
        && value
            .chars()
            .next()
            .is_some_and(|ch| ch.is_ascii_lowercase())
        && value
            .chars()
            .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '_');
    if !valid {
        issues.push(issue(
            path,
            "invalid_api_name",
            "API names must be snake_case, start with a letter, and be at most 64 characters",
        ));
    }
}

fn duplicate(path: &str, value: &str, issues: &mut Vec<ValidationIssue>) {
    issues.push(issue(
        path,
        "duplicate_api_name",
        format!("API name '{value}' is duplicated"),
    ));
}

fn issue(
    path: impl Into<String>,
    code: impl Into<String>,
    message: impl Into<String>,
) -> ValidationIssue {
    ValidationIssue {
        path: path.into(),
        code: code.into(),
        message: message.into(),
    }
}

fn detect_interface_cycles(graph: &HashMap<&str, Vec<&str>>, issues: &mut Vec<ValidationIssue>) {
    fn visit<'a>(
        node: &'a str,
        graph: &HashMap<&'a str, Vec<&'a str>>,
        visiting: &mut HashSet<&'a str>,
        visited: &mut HashSet<&'a str>,
    ) -> bool {
        if visiting.contains(node) {
            return true;
        }
        if visited.contains(node) {
            return false;
        }
        visiting.insert(node);
        let cyclic = graph.get(node).is_some_and(|parents| {
            parents
                .iter()
                .any(|parent| visit(parent, graph, visiting, visited))
        });
        visiting.remove(node);
        visited.insert(node);
        cyclic
    }
    let mut visited = HashSet::new();
    for node in graph.keys() {
        if visit(node, graph, &mut HashSet::new(), &mut visited) {
            issues.push(issue(
                "interfaces",
                "interface_cycle",
                format!("interface inheritance cycle includes '{node}'"),
            ));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn identity_property() -> PropertyDefinition {
        PropertyDefinition {
            api_name: "id".into(),
            display_name: "ID".into(),
            value_type: ValueType {
                scalar: ScalarType::Uuid,
                list: false,
            },
            required: true,
            unique: true,
            identity: true,
            indexed: true,
            description: None,
        }
    }

    #[test]
    fn validates_a_minimal_ontology() {
        let ontology = OntologyDefinition {
            api_name: "service_map".into(),
            display_name: "Service map".into(),
            description: None,
            object_types: vec![ObjectTypeDefinition {
                api_name: "service".into(),
                display_name: "Service".into(),
                description: None,
                properties: vec![identity_property()],
                shared_properties: vec![],
                derived_properties: vec![],
                implements: vec![],
            }],
            link_types: vec![],
            interfaces: vec![],
            value_types: vec![],
            struct_types: vec![],
            shared_properties: vec![],
            functions: vec![],
        };
        assert!(ontology.validate().is_empty());
    }

    #[test]
    fn rejects_unknown_link_targets_and_missing_identity() {
        let ontology = OntologyDefinition {
            api_name: "service_map".into(),
            display_name: "Service map".into(),
            description: None,
            object_types: vec![ObjectTypeDefinition {
                api_name: "service".into(),
                display_name: "Service".into(),
                description: None,
                properties: vec![],
                shared_properties: vec![],
                derived_properties: vec![],
                implements: vec![],
            }],
            link_types: vec![LinkTypeDefinition {
                api_name: "depends_on".into(),
                display_name: "Depends on".into(),
                source_type: "service".into(),
                target_type: "missing".into(),
                source_cardinality: Cardinality::Many,
                target_cardinality: Cardinality::Many,
                required: false,
                properties: vec![],
                description: None,
            }],
            interfaces: vec![],
            value_types: vec![],
            struct_types: vec![],
            shared_properties: vec![],
            functions: vec![],
        };
        let codes: HashSet<_> = ontology
            .validate()
            .into_iter()
            .map(|item| item.code)
            .collect();
        assert!(codes.contains("missing_identity"));
        assert!(codes.contains("unknown_type"));
    }

    #[test]
    fn validates_reusable_types_and_read_only_functions() {
        let ontology = OntologyDefinition {
            api_name: "service_map".into(),
            display_name: "Service map".into(),
            description: None,
            object_types: vec![ObjectTypeDefinition {
                api_name: "service".into(),
                display_name: "Service".into(),
                description: None,
                properties: vec![identity_property()],
                shared_properties: vec!["lifecycle".into()],
                derived_properties: vec![],
                implements: vec![],
            }],
            link_types: vec![],
            interfaces: vec![],
            value_types: vec![ValueTypeDefinition {
                api_name: "score".into(),
                display_name: "Score".into(),
                description: None,
                base_type: ScalarType::Float64,
                unit: None,
                minimum: Some(0.0),
                maximum: Some(100.0),
                pattern: None,
            }],
            struct_types: vec![],
            shared_properties: vec![SharedPropertyDefinition {
                api_name: "lifecycle".into(),
                display_name: "Lifecycle".into(),
                value_type: TypeReference::Scalar {
                    value_type: ValueType {
                        scalar: ScalarType::String,
                        list: false,
                    },
                },
                description: None,
                required: false,
                indexed: true,
            }],
            functions: vec![FunctionDefinition {
                api_name: "service_score".into(),
                display_name: "Service score".into(),
                description: None,
                inputs: vec![],
                output: TypeReference::ValueType {
                    api_name: "score".into(),
                    list: false,
                },
                implementation: FunctionImplementation::Expression {
                    expression: "42".into(),
                },
                read_only: true,
            }],
        };
        assert!(ontology.validate().is_empty());
    }

    #[test]
    fn rejects_mutating_functions_and_unknown_references() {
        let mut ontology = OntologyDefinition {
            api_name: "service_map".into(),
            display_name: "Service map".into(),
            description: None,
            object_types: vec![],
            link_types: vec![],
            interfaces: vec![],
            value_types: vec![],
            struct_types: vec![],
            shared_properties: vec![],
            functions: vec![],
        };
        ontology.functions.push(FunctionDefinition {
            api_name: "mutate".into(),
            display_name: "Mutate".into(),
            description: None,
            inputs: vec![],
            output: TypeReference::Struct {
                api_name: "missing".into(),
                list: false,
            },
            implementation: FunctionImplementation::Expression {
                expression: "value".into(),
            },
            read_only: false,
        });
        let codes: HashSet<_> = ontology.validate().into_iter().map(|issue| issue.code).collect();
        assert!(codes.contains("actions_not_supported"));
        assert!(codes.contains("unknown_struct_type"));
    }
}
