use std::net::SocketAddr;

use axum::{
    Json, Router,
    routing::{get, post},
};
use serde::Deserialize;
use serde_json::{Value, json};
use tower_http::trace::TraceLayer;
use tracing_subscriber::EnvFilter;

#[derive(Debug, Deserialize)]
struct JsonRpcRequest {
    #[allow(dead_code)]
    jsonrpc: Option<String>,
    id: Option<Value>,
    method: String,
    #[serde(default)]
    params: Value,
}

async fn health() -> &'static str {
    "ok"
}

async fn mcp(Json(request): Json<JsonRpcRequest>) -> Json<Value> {
    let id = request.id.unwrap_or(Value::Null);
    let result = match request.method.as_str() {
        "initialize" => json!({
            "protocolVersion": "2025-06-18",
            "capabilities": { "tools": {} },
            "serverInfo": { "name": "context-hub", "version": env!("CARGO_PKG_VERSION") }
        }),
        "tools/list" => json!({ "tools": tool_definitions() }),
        "tools/call" => call_tool(&request.params),
        "notifications/initialized" => return Json(json!({})),
        _ => {
            return Json(
                json!({ "jsonrpc": "2.0", "id": id, "error": { "code": -32601, "message": "Method not found" } }),
            );
        }
    };
    Json(json!({ "jsonrpc": "2.0", "id": id, "result": result }))
}

fn tool_definitions() -> Vec<Value> {
    vec![
        tool(
            "get_ontology_schema",
            "Return the active ontology schema for a workspace.",
            &json!({ "type": "object", "properties": { "workspace_id": { "type": "string" } }, "required": ["workspace_id"] }),
        ),
        tool(
            "search_objects",
            "Search ontology objects by type and text.",
            &json!({ "type": "object", "properties": { "workspace_id": { "type": "string" }, "object_type": { "type": "string" }, "query": { "type": "string" }, "limit": { "type": "integer", "maximum": 100 } }, "required": ["workspace_id", "object_type", "query"] }),
        ),
        tool(
            "get_object",
            "Get one object and its properties.",
            &json!({ "type": "object", "properties": { "workspace_id": { "type": "string" }, "object_type": { "type": "string" }, "id": { "type": "string" } }, "required": ["workspace_id", "object_type", "id"] }),
        ),
        tool(
            "query_graph",
            "Run a bounded, read-only graph traversal.",
            &json!({ "type": "object", "properties": { "workspace_id": { "type": "string" }, "root_type": { "type": "string" }, "traversal": { "type": "array", "maxItems": 6 }, "limit": { "type": "integer", "maximum": 500 } }, "required": ["workspace_id", "root_type"] }),
        ),
    ]
}

fn tool(name: &str, description: &str, input_schema: &Value) -> Value {
    json!({ "name": name, "description": description, "inputSchema": input_schema, "annotations": { "readOnlyHint": true } })
}

fn call_tool(params: &Value) -> Value {
    let name = params
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if ![
        "get_ontology_schema",
        "search_objects",
        "get_object",
        "query_graph",
    ]
    .contains(&name)
    {
        return json!({ "isError": true, "content": [{ "type": "text", "text": "Unknown tool" }] });
    }
    json!({
        "content": [{ "type": "text", "text": serde_json::to_string(&json!({ "tool": name, "data": [], "note": "The read-only MCP surface is ready; connect the graph repository to return persisted data." })).unwrap_or_default() }]
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
    let app = Router::new()
        .route("/health", get(health))
        .route("/mcp", post(mcp))
        .layer(TraceLayer::new_for_http());
    tracing::info!(%address, "starting ContextHub MCP server");
    let listener = tokio::net::TcpListener::bind(address).await?;
    axum::serve(listener, app).await?;
    Ok(())
}
