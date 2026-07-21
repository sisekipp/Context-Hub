use std::net::SocketAddr;

use axum::{
    Json, Router,
    extract::State,
    routing::{get, post},
};
use proto::{
    ExpandGraphRequest, FilterOperator, GetObjectRequest, GraphAggregation, GraphFilter,
    GraphQuery, GraphSort, ListOntologiesRequest, ListOntologyVersionsRequest, SortDirection,
    TraversalStep, graph_service_client::GraphServiceClient,
    ontology_service_client::OntologyServiceClient,
};
use serde::Deserialize;
use serde_json::{Map, Value, json};
use tonic::transport::{Channel, Endpoint};
use tower_http::trace::TraceLayer;
use tracing_subscriber::EnvFilter;

mod proto {
    #![allow(clippy::all, clippy::pedantic)]

    // The generated tonic/prost surface intentionally follows generator conventions.
    tonic::include_proto!("context_hub.v1");
}

#[derive(Clone)]
struct AppState {
    channel: Channel,
}

#[derive(Debug, Deserialize)]
struct JsonRpcRequest {
    #[allow(dead_code)]
    jsonrpc: Option<String>,
    id: Option<Value>,
    method: String,
    #[serde(default)]
    params: Value,
}

#[derive(Debug, Deserialize)]
struct ToolCall {
    name: String,
    #[serde(default)]
    arguments: Value,
}

#[derive(Debug, Deserialize)]
struct OntologyArguments {
    workspace_id: String,
    #[serde(default)]
    ontology_id: Option<String>,
}

#[derive(Debug, Deserialize)]
struct SearchArguments {
    workspace_id: String,
    ontology_id: String,
    object_type: String,
    query: String,
    #[serde(default = "default_search_property")]
    property: String,
    #[serde(default = "default_search_limit")]
    limit: u32,
    #[serde(default)]
    cursor: String,
}

#[derive(Debug, Deserialize)]
struct GetObjectArguments {
    workspace_id: String,
    ontology_id: String,
    object_type: String,
    id: String,
    #[serde(default)]
    include_related: bool,
    #[serde(default)]
    link_types: Vec<String>,
    #[serde(default = "default_related_limit")]
    related_limit: u32,
}

#[derive(Debug, Deserialize)]
struct QueryArguments {
    workspace_id: String,
    ontology_id: String,
    root_type: String,
    #[serde(default)]
    filters: Vec<FilterArguments>,
    #[serde(default)]
    traversal: Vec<TraversalArguments>,
    #[serde(default)]
    projection: Vec<String>,
    #[serde(default)]
    sort: Option<SortArguments>,
    #[serde(default)]
    aggregations: Vec<AggregationArguments>,
    #[serde(default = "default_query_limit")]
    limit: u32,
    #[serde(default)]
    cursor: String,
}

#[derive(Debug, Deserialize)]
struct FilterArguments {
    property: String,
    operator: String,
    values: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct TraversalArguments {
    link_type: String,
    target_type: String,
    #[serde(default)]
    reverse: bool,
}

#[derive(Debug, Deserialize)]
struct SortArguments {
    property: String,
    #[serde(default = "default_sort_direction")]
    direction: String,
}

#[derive(Debug, Deserialize)]
struct AggregationArguments {
    property: String,
    function: String,
    alias: String,
}

fn default_search_property() -> String {
    "name".into()
}

fn default_search_limit() -> u32 {
    25
}

fn default_related_limit() -> u32 {
    50
}

fn default_query_limit() -> u32 {
    100
}

fn default_sort_direction() -> String {
    "ascending".into()
}

async fn health() -> &'static str {
    "ok"
}

async fn mcp(State(state): State<AppState>, Json(request): Json<JsonRpcRequest>) -> Json<Value> {
    let id = request.id.unwrap_or(Value::Null);
    let result = match request.method.as_str() {
        "initialize" => json!({
            "protocolVersion": "2025-06-18",
            "capabilities": { "tools": { "listChanged": false } },
            "serverInfo": { "name": "context-hub", "version": env!("CARGO_PKG_VERSION") }
        }),
        "tools/list" => json!({ "tools": tool_definitions() }),
        "tools/call" => call_tool(&state, request.params).await,
        "notifications/initialized" => return Json(json!({})),
        _ => {
            return Json(json!({
                "jsonrpc": "2.0",
                "id": id,
                "error": { "code": -32601, "message": "Method not found" }
            }));
        }
    };
    Json(json!({ "jsonrpc": "2.0", "id": id, "result": result }))
}

fn ontology_properties() -> Map<String, Value> {
    Map::from_iter([
        (
            "workspace_id".into(),
            json!({ "type": "string", "description": "Workspace UUID." }),
        ),
        (
            "ontology_id".into(),
            json!({ "type": "string", "description": "Ontology UUID. Data never crosses ontology boundaries." }),
        ),
    ])
}

fn tool_definitions() -> Vec<Value> {
    let schema_properties = ontology_properties();
    let mut search_properties = ontology_properties();
    search_properties.extend(Map::from_iter([
        ("object_type".into(), json!({ "type": "string" })),
        ("query".into(), json!({ "type": "string" })),
        (
            "property".into(),
            json!({ "type": "string", "default": "name" }),
        ),
        (
            "limit".into(),
            json!({ "type": "integer", "minimum": 1, "maximum": 100, "default": 25 }),
        ),
        ("cursor".into(), json!({ "type": "string" })),
    ]));
    let mut object_properties = ontology_properties();
    object_properties.extend(Map::from_iter([
        ("object_type".into(), json!({ "type": "string" })),
        ("id".into(), json!({ "type": "string" })),
        (
            "include_related".into(),
            json!({ "type": "boolean", "default": false }),
        ),
        (
            "link_types".into(),
            json!({ "type": "array", "items": { "type": "string" }, "maxItems": 20 }),
        ),
        (
            "related_limit".into(),
            json!({ "type": "integer", "minimum": 1, "maximum": 100, "default": 50 }),
        ),
    ]));
    let mut query_properties = ontology_properties();
    query_properties.extend(Map::from_iter([
        ("root_type".into(), json!({ "type": "string" })),
        ("filters".into(), json!({ "type": "array", "maxItems": 20, "items": { "type": "object", "properties": { "property": { "type": "string" }, "operator": { "type": "string", "enum": ["equal", "not_equal", "contains", "greater_than", "less_than", "in"] }, "values": { "type": "array", "items": { "type": "string" }, "minItems": 1, "maxItems": 100 } }, "required": ["property", "operator", "values"], "additionalProperties": false } })),
        ("traversal".into(), json!({ "type": "array", "maxItems": 6, "items": { "type": "object", "properties": { "link_type": { "type": "string" }, "target_type": { "type": "string" }, "reverse": { "type": "boolean", "default": false } }, "required": ["link_type", "target_type"], "additionalProperties": false } })),
        ("projection".into(), json!({ "type": "array", "items": { "type": "string" }, "maxItems": 100 })),
        ("sort".into(), json!({ "type": "object", "properties": { "property": { "type": "string" }, "direction": { "type": "string", "enum": ["ascending", "descending"], "default": "ascending" } }, "required": ["property"], "additionalProperties": false })),
        ("aggregations".into(), json!({ "type": "array", "maxItems": 20, "items": { "type": "object", "properties": { "property": { "type": "string" }, "function": { "type": "string", "enum": ["count", "distinct_count", "sum", "average", "minimum", "maximum"] }, "alias": { "type": "string" } }, "required": ["property", "function", "alias"], "additionalProperties": false } })),
        ("limit".into(), json!({ "type": "integer", "minimum": 1, "maximum": 500, "default": 100 })),
        ("cursor".into(), json!({ "type": "string" })),
    ]));
    vec![
        tool(
            "get_ontology_schema",
            "Discover a workspace's ontologies and return their active, published schemas. Pass ontology_id to select one ontology.",
            &json!({ "type": "object", "properties": schema_properties, "required": ["workspace_id"], "additionalProperties": false }),
        ),
        tool(
            "search_objects",
            "Search persisted objects inside one ontology by a validated property filter.",
            &json!({ "type": "object", "properties": search_properties, "required": ["workspace_id", "ontology_id", "object_type", "query"], "additionalProperties": false }),
        ),
        tool(
            "get_object",
            "Get one persisted object and optionally its directly connected objects.",
            &json!({ "type": "object", "properties": object_properties, "required": ["workspace_id", "ontology_id", "object_type", "id"], "additionalProperties": false }),
        ),
        tool(
            "query_graph",
            "Run a bounded, validated, read-only graph query with filters, projections, sorting, aggregations and traversals.",
            &json!({ "type": "object", "properties": query_properties, "required": ["workspace_id", "ontology_id", "root_type"], "additionalProperties": false }),
        ),
    ]
}

fn tool(name: &str, description: &str, input_schema: &Value) -> Value {
    json!({
        "name": name,
        "description": description,
        "inputSchema": input_schema,
        "annotations": { "readOnlyHint": true, "destructiveHint": false, "idempotentHint": true }
    })
}

async fn call_tool(state: &AppState, params: Value) -> Value {
    let call = match serde_json::from_value::<ToolCall>(params) {
        Ok(call) => call,
        Err(error) => return tool_error(format!("Invalid tools/call request: {error}")),
    };
    let result = match call.name.as_str() {
        "get_ontology_schema" => get_ontology_schema(state, call.arguments).await,
        "search_objects" => search_objects(state, call.arguments).await,
        "get_object" => get_object(state, call.arguments).await,
        "query_graph" => query_graph(state, call.arguments).await,
        _ => Err("Unknown tool".into()),
    };
    match result {
        Ok(value) => tool_result(value),
        Err(error) => tool_error(error),
    }
}

async fn active_ontology(
    state: &AppState,
    workspace_id: &str,
    ontology_id: &str,
) -> Result<(proto::Ontology, proto::OntologyVersion), String> {
    let mut client = OntologyServiceClient::new(state.channel.clone());
    let ontologies = client
        .list(ListOntologiesRequest {
            workspace_id: workspace_id.into(),
        })
        .await
        .map_err(grpc_error)?
        .into_inner()
        .ontologies;
    let ontology = ontologies
        .into_iter()
        .find(|ontology| ontology.id == ontology_id)
        .ok_or_else(|| "Ontology was not found in this workspace".to_string())?;
    if ontology.active_version_id.is_empty() {
        return Err("Ontology has no active published version".into());
    }
    let versions = client
        .list_versions(ListOntologyVersionsRequest {
            ontology_id: ontology.id.clone(),
        })
        .await
        .map_err(grpc_error)?
        .into_inner()
        .versions;
    let version = versions
        .into_iter()
        .find(|version| version.id == ontology.active_version_id && version.active)
        .ok_or_else(|| "Active ontology version could not be loaded".to_string())?;
    Ok((ontology, version))
}

async fn get_ontology_schema(state: &AppState, arguments: Value) -> Result<Value, String> {
    let arguments: OntologyArguments = parse_arguments(arguments)?;
    if let Some(ontology_id) = arguments.ontology_id {
        let (ontology, version) =
            active_ontology(state, &arguments.workspace_id, &ontology_id).await?;
        return ontology_schema_value(&ontology, &version);
    }
    let mut client = OntologyServiceClient::new(state.channel.clone());
    let ontologies = client
        .list(ListOntologiesRequest {
            workspace_id: arguments.workspace_id,
        })
        .await
        .map_err(grpc_error)?
        .into_inner()
        .ontologies;
    let mut schemas = Vec::new();
    for ontology in ontologies {
        if ontology.active_version_id.is_empty() {
            continue;
        }
        let versions = client
            .list_versions(ListOntologyVersionsRequest {
                ontology_id: ontology.id.clone(),
            })
            .await
            .map_err(grpc_error)?
            .into_inner()
            .versions;
        if let Some(version) = versions
            .into_iter()
            .find(|version| version.id == ontology.active_version_id && version.active)
        {
            schemas.push(ontology_schema_value(&ontology, &version)?);
        }
    }
    Ok(json!({ "ontologies": schemas }))
}

fn ontology_schema_value(
    ontology: &proto::Ontology,
    version: &proto::OntologyVersion,
) -> Result<Value, String> {
    let definition: Value = serde_json::from_str(&version.definition_json)
        .map_err(|error| format!("Stored ontology schema is invalid: {error}"))?;
    Ok(json!({
        "ontology": {
            "id": ontology.id,
            "name": ontology.name,
            "slug": ontology.slug,
            "active_version_id": version.id,
            "version": version.version,
            "checksum": version.checksum
        },
        "schema": definition
    }))
}

async fn search_objects(state: &AppState, arguments: Value) -> Result<Value, String> {
    let arguments: SearchArguments = parse_arguments(arguments)?;
    ensure_limit(arguments.limit, 100, "search limit")?;
    let (_, version) =
        active_ontology(state, &arguments.workspace_id, &arguments.ontology_id).await?;
    let query = GraphQuery {
        workspace_id: arguments.workspace_id,
        ontology_version_id: version.id,
        root_type: arguments.object_type,
        filters: vec![GraphFilter {
            property: arguments.property,
            operator: FilterOperator::Contains as i32,
            values: vec![arguments.query],
        }],
        traversal: vec![],
        projection: vec![],
        limit: arguments.limit,
        cursor: arguments.cursor,
        sort: None,
        aggregations: vec![],
    };
    let response = GraphServiceClient::new(state.channel.clone())
        .query(query)
        .await
        .map_err(grpc_error)?
        .into_inner();
    graph_response(response)
}

async fn get_object(state: &AppState, arguments: Value) -> Result<Value, String> {
    let arguments: GetObjectArguments = parse_arguments(arguments)?;
    ensure_limit(arguments.related_limit, 100, "related limit")?;
    if arguments.link_types.len() > 20 {
        return Err("link_types exceeds 20 items".into());
    }
    let (_, version) =
        active_ontology(state, &arguments.workspace_id, &arguments.ontology_id).await?;
    let mut client = GraphServiceClient::new(state.channel.clone());
    let object = client
        .get_object(GetObjectRequest {
            workspace_id: arguments.workspace_id.clone(),
            ontology_version_id: version.id.clone(),
            object_type: arguments.object_type.clone(),
            id: arguments.id.clone(),
        })
        .await
        .map_err(grpc_error)?
        .into_inner();
    let mut result = json!({ "object": node_value(&object)? });
    if arguments.include_related {
        let neighborhood = client
            .expand(ExpandGraphRequest {
                workspace_id: arguments.workspace_id,
                ontology_version_id: version.id,
                object_type: arguments.object_type,
                object_id: arguments.id,
                link_types: arguments.link_types,
                limit: arguments.related_limit,
            })
            .await
            .map_err(grpc_error)?
            .into_inner();
        result["neighborhood"] = graph_response(neighborhood)?;
    }
    Ok(result)
}

async fn query_graph(state: &AppState, arguments: Value) -> Result<Value, String> {
    let arguments: QueryArguments = parse_arguments(arguments)?;
    ensure_limit(arguments.limit, 500, "query limit")?;
    if arguments.traversal.len() > 6 {
        return Err("traversal exceeds 6 steps".into());
    }
    let (_, version) =
        active_ontology(state, &arguments.workspace_id, &arguments.ontology_id).await?;
    let filters = arguments
        .filters
        .into_iter()
        .map(|filter| {
            Ok(GraphFilter {
                property: filter.property,
                operator: filter_operator(&filter.operator)?,
                values: filter.values,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    let sort = arguments
        .sort
        .map(|sort| -> Result<GraphSort, String> {
            Ok(GraphSort {
                property: sort.property,
                direction: sort_direction(&sort.direction)?,
            })
        })
        .transpose()?;
    let aggregations = arguments
        .aggregations
        .into_iter()
        .map(|aggregation| {
            Ok(GraphAggregation {
                property: aggregation.property,
                function: aggregation_function(&aggregation.function)?,
                alias: aggregation.alias,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    let query = GraphQuery {
        workspace_id: arguments.workspace_id,
        ontology_version_id: version.id,
        root_type: arguments.root_type,
        filters,
        traversal: arguments
            .traversal
            .into_iter()
            .map(|step| TraversalStep {
                link_type: step.link_type,
                target_type: step.target_type,
                reverse: step.reverse,
            })
            .collect(),
        projection: arguments.projection,
        limit: arguments.limit,
        cursor: arguments.cursor,
        sort,
        aggregations,
    };
    let response = GraphServiceClient::new(state.channel.clone())
        .query(query)
        .await
        .map_err(grpc_error)?
        .into_inner();
    graph_response(response)
}

fn graph_response(response: proto::QueryGraphResponse) -> Result<Value, String> {
    let nodes = response
        .nodes
        .iter()
        .map(node_value)
        .collect::<Result<Vec<_>, _>>()?;
    let edges = response
        .edges
        .into_iter()
        .map(|edge| {
            let properties = parse_json(&edge.properties_json, "edge properties")?;
            Ok(json!({
                "id": edge.id,
                "link_type": edge.link_type,
                "source_id": edge.source_id,
                "target_id": edge.target_id,
                "properties": properties
            }))
        })
        .collect::<Result<Vec<_>, String>>()?;
    let aggregations = response
        .aggregation_results
        .into_iter()
        .map(|result| {
            Ok(json!({
                "alias": result.alias,
                "value": parse_json(&result.value_json, "aggregation value")?
            }))
        })
        .collect::<Result<Vec<_>, String>>()?;
    Ok(json!({
        "nodes": nodes,
        "edges": edges,
        "aggregations": aggregations,
        "next_cursor": response.next_cursor,
        "truncated": response.truncated
    }))
}

fn node_value(node: &proto::GraphNode) -> Result<Value, String> {
    Ok(json!({
        "id": node.id,
        "object_type": node.object_type,
        "properties": parse_json(&node.properties_json, "object properties")?
    }))
}

fn parse_json(value: &str, label: &str) -> Result<Value, String> {
    serde_json::from_str(value).map_err(|error| format!("Stored {label} are invalid: {error}"))
}

fn parse_arguments<T: for<'de> Deserialize<'de>>(arguments: Value) -> Result<T, String> {
    serde_json::from_value(arguments).map_err(|error| format!("Invalid tool arguments: {error}"))
}

fn ensure_limit(limit: u32, maximum: u32, label: &str) -> Result<(), String> {
    if limit == 0 || limit > maximum {
        Err(format!("{label} must be between 1 and {maximum}"))
    } else {
        Ok(())
    }
}

fn filter_operator(operator: &str) -> Result<i32, String> {
    let operator = match operator {
        "equal" => FilterOperator::Equal,
        "not_equal" => FilterOperator::NotEqual,
        "contains" => FilterOperator::Contains,
        "greater_than" => FilterOperator::GreaterThan,
        "less_than" => FilterOperator::LessThan,
        "in" => FilterOperator::In,
        _ => return Err(format!("Unknown filter operator: {operator}")),
    };
    Ok(operator as i32)
}

fn sort_direction(direction: &str) -> Result<i32, String> {
    let direction = match direction {
        "ascending" => SortDirection::Ascending,
        "descending" => SortDirection::Descending,
        _ => return Err(format!("Unknown sort direction: {direction}")),
    };
    Ok(direction as i32)
}

fn aggregation_function(function: &str) -> Result<i32, String> {
    use proto::AggregationFunction;
    let function = match function {
        "count" => AggregationFunction::Count,
        "distinct_count" => AggregationFunction::DistinctCount,
        "sum" => AggregationFunction::Sum,
        "average" => AggregationFunction::Average,
        "minimum" => AggregationFunction::Minimum,
        "maximum" => AggregationFunction::Maximum,
        _ => return Err(format!("Unknown aggregation function: {function}")),
    };
    Ok(function as i32)
}

#[allow(clippy::needless_pass_by_value)]
fn grpc_error(status: tonic::Status) -> String {
    format!(
        "ContextHub API error ({}): {}",
        status.code(),
        status.message()
    )
}

#[allow(clippy::needless_pass_by_value)]
fn tool_result(value: Value) -> Value {
    json!({
        "content": [{ "type": "text", "text": serde_json::to_string_pretty(&value).unwrap_or_default() }],
        "structuredContent": value,
        "isError": false
    })
}

fn tool_error(error: impl Into<String>) -> Value {
    json!({
        "content": [{ "type": "text", "text": error.into() }],
        "isError": true
    })
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("context_hub=info")),
        )
        .init();
    let address: SocketAddr = std::env::var("HTTP_ADDR")
        .unwrap_or_else(|_| "0.0.0.0:8080".into())
        .parse()?;
    let grpc_endpoint = std::env::var("CONTEXT_HUB_GRPC_ENDPOINT")
        .unwrap_or_else(|_| "http://127.0.0.1:50051".into());
    let channel = Endpoint::from_shared(grpc_endpoint.clone())?.connect_lazy();
    let app = Router::new()
        .route("/health", get(health))
        .route("/mcp", post(mcp))
        .layer(TraceLayer::new_for_http())
        .with_state(AppState { channel });
    tracing::info!(%address, %grpc_endpoint, "starting ContextHub MCP server");
    let listener = tokio::net::TcpListener::bind(address).await?;
    axum::serve(listener, app).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tools_are_read_only_and_data_tools_require_an_ontology() {
        let tools = tool_definitions();
        assert_eq!(tools.len(), 4);
        for tool in tools {
            assert_eq!(tool["annotations"]["readOnlyHint"], true);
            assert_eq!(tool["annotations"]["destructiveHint"], false);
            let required = tool["inputSchema"]["required"].as_array().unwrap();
            assert!(required.contains(&json!("workspace_id")));
            if tool["name"] == "get_ontology_schema" {
                assert!(!required.contains(&json!("ontology_id")));
            } else {
                assert!(required.contains(&json!("ontology_id")));
            }
        }
    }

    #[test]
    fn query_enums_reject_unknown_values() {
        assert!(filter_operator("contains").is_ok());
        assert!(filter_operator("sql").is_err());
        assert!(sort_direction("descending").is_ok());
        assert!(sort_direction("random").is_err());
        assert!(aggregation_function("average").is_ok());
        assert!(aggregation_function("eval").is_err());
    }

    #[test]
    fn limits_are_bounded() {
        assert!(ensure_limit(1, 100, "limit").is_ok());
        assert!(ensure_limit(100, 100, "limit").is_ok());
        assert!(ensure_limit(0, 100, "limit").is_err());
        assert!(ensure_limit(101, 100, "limit").is_err());
    }
}
