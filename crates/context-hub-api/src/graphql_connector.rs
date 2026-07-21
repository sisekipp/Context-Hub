use std::{
    collections::{HashMap, HashSet},
    future::Future,
};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::rest_connector::{fetch_secure_json_post, validate_secure_remote};

const DEFAULT_MAX_BYTES: usize = 32 * 1024 * 1024;
const MAX_ALLOWED_BYTES: usize = 64 * 1024 * 1024;
const DEFAULT_MAX_PAGES: usize = 100;
const MAX_ALLOWED_PAGES: usize = 1_000;
const MAX_RECORDS: usize = 1_000_000;
const MAX_QUERY_BYTES: usize = 256 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphqlSourceConfiguration {
    pub url: String,
    pub query: String,
    #[serde(default = "empty_object")]
    pub variables: Value,
    #[serde(default)]
    pub headers: HashMap<String, String>,
    pub record_path: String,
    #[serde(default)]
    pub pagination: GraphqlPagination,
    #[serde(default = "default_max_pages")]
    pub max_pages: usize,
    #[serde(default = "default_max_bytes")]
    pub max_bytes: usize,
    #[serde(default = "default_timeout_seconds")]
    pub timeout_seconds: u64,
    #[serde(default = "default_retry_attempts")]
    pub retry_attempts: usize,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum GraphqlPagination {
    #[default]
    None,
    Cursor {
        variable: String,
        next_cursor_path: String,
        #[serde(default)]
        initial_cursor: Option<String>,
    },
}

pub async fn fetch_graphql_source(
    configuration: &GraphqlSourceConfiguration,
) -> Result<Vec<u8>, String> {
    configuration.validate().await?;
    let url = configuration.url.as_str();
    let headers = &configuration.headers;
    let timeout_seconds = configuration.timeout_seconds;
    let retry_attempts = configuration.retry_attempts;
    collect_graphql_pages(configuration, |body, remaining| async move {
        fetch_secure_json_post(
            url,
            headers,
            &body,
            timeout_seconds,
            retry_attempts,
            remaining,
        )
        .await
    })
    .await
}

impl GraphqlSourceConfiguration {
    pub async fn validate(&self) -> Result<(), String> {
        if self.query.trim().is_empty() || self.query.len() > MAX_QUERY_BYTES {
            return Err(format!(
                "GraphQL query must contain between 1 and {MAX_QUERY_BYTES} bytes"
            ));
        }
        if !self.variables.is_object() {
            return Err("GraphQL variables must be a JSON object".into());
        }
        if self.record_path.trim().is_empty() {
            return Err("GraphQL record_path is required".into());
        }
        if self.max_pages == 0 || self.max_pages > MAX_ALLOWED_PAGES {
            return Err(format!(
                "GraphQL max_pages must be between 1 and {MAX_ALLOWED_PAGES}"
            ));
        }
        if self.max_bytes == 0 || self.max_bytes > MAX_ALLOWED_BYTES {
            return Err(format!(
                "GraphQL max_bytes must be between 1 and {MAX_ALLOWED_BYTES}"
            ));
        }
        if self.timeout_seconds == 0 || self.timeout_seconds > 60 {
            return Err("GraphQL timeout_seconds must be between 1 and 60".into());
        }
        if self.retry_attempts > 5 {
            return Err("GraphQL retry_attempts cannot exceed 5".into());
        }
        if let GraphqlPagination::Cursor {
            variable,
            next_cursor_path,
            ..
        } = &self.pagination
        {
            validate_graphql_name(variable)?;
            if next_cursor_path.trim().is_empty() {
                return Err("GraphQL next_cursor_path is required".into());
            }
        }
        validate_secure_remote(&self.url, &self.headers).await
    }
}

async fn collect_graphql_pages<F, Fut>(
    configuration: &GraphqlSourceConfiguration,
    mut fetch: F,
) -> Result<Vec<u8>, String>
where
    F: FnMut(Value, usize) -> Fut,
    Fut: Future<Output = Result<Vec<u8>, String>>,
{
    let mut cursor = match &configuration.pagination {
        GraphqlPagination::Cursor { initial_cursor, .. } => initial_cursor.clone(),
        GraphqlPagination::None => None,
    };
    let mut seen_cursors = HashSet::new();
    let mut records = Vec::new();
    let mut bytes_read = 0;
    for page_index in 0..configuration.max_pages {
        let body = request_body(configuration, cursor.as_deref())?;
        let remaining = configuration.max_bytes.saturating_sub(bytes_read);
        if remaining == 0 {
            return Err("GraphQL source exceeded its configured byte limit".into());
        }
        let content = fetch(body, remaining).await?;
        bytes_read += content.len();
        let response: Value = serde_json::from_slice(&content)
            .map_err(|error| format!("GraphQL response is not valid JSON: {error}"))?;
        validate_graphql_response(&response)?;
        let page_records = extract_records(&response, &configuration.record_path)?;
        if records.len() + page_records.len() > MAX_RECORDS {
            return Err(format!(
                "GraphQL source exceeds the limit of {MAX_RECORDS} records"
            ));
        }
        records.extend(page_records);
        if !advance_cursor(
            &configuration.pagination,
            &response,
            page_index,
            &mut cursor,
            &mut seen_cursors,
        )? {
            let content = serde_json::to_vec(&records)
                .map_err(|error| format!("GraphQL records could not be serialized: {error}"))?;
            if content.len() > configuration.max_bytes {
                return Err("GraphQL records exceeded their configured byte limit".into());
            }
            return Ok(content);
        }
    }
    Err("GraphQL source reached max_pages before pagination completed".into())
}

fn request_body(
    configuration: &GraphqlSourceConfiguration,
    cursor: Option<&str>,
) -> Result<Value, String> {
    let mut variables = configuration
        .variables
        .as_object()
        .cloned()
        .ok_or_else(|| "GraphQL variables must be a JSON object".to_owned())?;
    if let GraphqlPagination::Cursor { variable, .. } = &configuration.pagination {
        variables.insert(
            variable.clone(),
            cursor.map_or(Value::Null, |value| Value::String(value.to_owned())),
        );
    }
    Ok(json!({ "query": configuration.query, "variables": variables }))
}

fn validate_graphql_response(response: &Value) -> Result<(), String> {
    let Some(errors) = response.get("errors").and_then(Value::as_array) else {
        return Ok(());
    };
    if errors.is_empty() {
        return Ok(());
    }
    let messages = errors
        .iter()
        .take(5)
        .filter_map(|error| error.get("message").and_then(Value::as_str))
        .collect::<Vec<_>>();
    Err(if messages.is_empty() {
        "GraphQL endpoint returned errors".into()
    } else {
        format!("GraphQL endpoint returned errors: {}", messages.join("; "))
    })
}

fn extract_records(response: &Value, path: &str) -> Result<Vec<Value>, String> {
    let selected = value_at_path(response, path)
        .ok_or_else(|| "GraphQL record_path does not exist in the response".to_owned())?;
    match selected {
        Value::Array(records) => Ok(records.clone()),
        Value::Object(_) => Ok(vec![selected.clone()]),
        _ => Err("GraphQL record_path must select an object or array".into()),
    }
}

fn advance_cursor(
    pagination: &GraphqlPagination,
    response: &Value,
    page_index: usize,
    cursor: &mut Option<String>,
    seen_cursors: &mut HashSet<String>,
) -> Result<bool, String> {
    let GraphqlPagination::Cursor {
        next_cursor_path, ..
    } = pagination
    else {
        return Ok(false);
    };
    let next = value_at_path(response, next_cursor_path)
        .and_then(|value| match value {
            Value::String(value) => Some(value.clone()),
            Value::Number(value) => Some(value.to_string()),
            _ => None,
        })
        .filter(|value| !value.is_empty());
    let Some(next) = next else {
        return Ok(false);
    };
    if !seen_cursors.insert(next.clone()) || cursor.as_ref() == Some(&next) {
        return Err(format!(
            "GraphQL cursor pagination repeated a cursor on page {}",
            page_index + 1
        ));
    }
    *cursor = Some(next);
    Ok(true)
}

fn value_at_path<'a>(value: &'a Value, path: &str) -> Option<&'a Value> {
    if path.starts_with('/') {
        return value.pointer(path);
    }
    path.split('.').try_fold(value, |current, segment| {
        current
            .as_object()
            .and_then(|object| object.get(segment))
            .or_else(|| {
                segment
                    .parse::<usize>()
                    .ok()
                    .and_then(|index| current.as_array()?.get(index))
            })
    })
}

fn validate_graphql_name(value: &str) -> Result<(), String> {
    let mut characters = value.chars();
    let valid_start = characters
        .next()
        .is_some_and(|character| character == '_' || character.is_ascii_alphabetic());
    if !valid_start
        || value.len() > 128
        || !characters.all(|character| character == '_' || character.is_ascii_alphanumeric())
    {
        return Err(format!("GraphQL variable name '{value}' is invalid"));
    }
    Ok(())
}

fn empty_object() -> Value {
    Value::Object(Map::new())
}

const fn default_max_pages() -> usize {
    DEFAULT_MAX_PAGES
}
const fn default_max_bytes() -> usize {
    DEFAULT_MAX_BYTES
}
const fn default_timeout_seconds() -> u64 {
    30
}
const fn default_retry_attempts() -> usize {
    2
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        collections::VecDeque,
        sync::{Arc, Mutex},
    };

    fn configuration() -> GraphqlSourceConfiguration {
        GraphqlSourceConfiguration {
            url: "https://example.com/graphql".into(),
            query: "query Services($after: String) { services(after: $after) { nodes { id name } pageInfo { endCursor } } }".into(),
            variables: json!({ "limit": 2 }),
            headers: HashMap::new(),
            record_path: "data.services.nodes".into(),
            pagination: GraphqlPagination::Cursor {
                variable: "after".into(),
                next_cursor_path: "data.services.pageInfo.endCursor".into(),
                initial_cursor: None,
            },
            max_pages: 10,
            max_bytes: 10_000,
            timeout_seconds: 5,
            retry_attempts: 0,
        }
    }

    #[tokio::test]
    async fn merges_cursor_pages_and_updates_variables() {
        let responses = Arc::new(Mutex::new(VecDeque::from([
            br#"{"data":{"services":{"nodes":[{"id":"1"},{"id":"2"}],"pageInfo":{"endCursor":"abc"}}}}"#.to_vec(),
            br#"{"data":{"services":{"nodes":[{"id":"3"}],"pageInfo":{"endCursor":null}}}}"#.to_vec(),
        ])));
        let requests = Arc::new(Mutex::new(Vec::new()));
        let content = collect_graphql_pages(&configuration(), {
            let responses = Arc::clone(&responses);
            let requests = Arc::clone(&requests);
            move |body, _| {
                requests.lock().unwrap().push(body);
                let response = responses.lock().unwrap().pop_front().unwrap();
                async move { Ok(response) }
            }
        })
        .await
        .unwrap();
        let records: Vec<Value> = serde_json::from_slice(&content).unwrap();
        assert_eq!(records.len(), 3);
        let requests = requests.lock().unwrap();
        assert!(requests[0]["variables"]["after"].is_null());
        assert_eq!(requests[1]["variables"]["after"], "abc");
    }

    #[test]
    fn surfaces_graphql_errors_without_extensions() {
        let error = validate_graphql_response(&json!({
            "data": null,
            "errors": [{"message": "Access denied", "extensions": {"secret": "hidden"}}]
        }))
        .unwrap_err();
        assert_eq!(error, "GraphQL endpoint returned errors: Access denied");
        assert!(!error.contains("secret"));
    }

    #[test]
    fn rejects_invalid_variable_names() {
        assert!(validate_graphql_name("after").is_ok());
        assert!(validate_graphql_name("1after").is_err());
        assert!(validate_graphql_name("after.value").is_err());
    }
}
