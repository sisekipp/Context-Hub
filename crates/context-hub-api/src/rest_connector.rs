use std::{
    collections::{HashMap, HashSet},
    future::Future,
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr},
    time::Duration,
};

use futures::StreamExt;
use reqwest::{
    Client, StatusCode, Url,
    header::{HeaderMap, HeaderName, HeaderValue, LOCATION},
    redirect::Policy,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::{
    net::lookup_host,
    time::{sleep, timeout},
};

const DEFAULT_MAX_BYTES: usize = 32 * 1024 * 1024;
const MAX_ALLOWED_BYTES: usize = 64 * 1024 * 1024;
const DEFAULT_MAX_PAGES: usize = 100;
const MAX_ALLOWED_PAGES: usize = 1_000;
const MAX_RECORDS: usize = 1_000_000;
const MAX_REDIRECTS: usize = 5;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RestSourceConfiguration {
    pub url: String,
    #[serde(default)]
    pub headers: HashMap<String, String>,
    #[serde(default)]
    pub query: HashMap<String, String>,
    #[serde(default)]
    pub record_path: Option<String>,
    #[serde(default)]
    pub pagination: RestPagination,
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
pub enum RestPagination {
    #[default]
    None,
    Page {
        #[serde(default = "default_page_parameter")]
        parameter: String,
        #[serde(default = "default_start_page")]
        start: u64,
        #[serde(default)]
        page_size_parameter: Option<String>,
        #[serde(default)]
        page_size: Option<usize>,
        #[serde(default = "default_true")]
        stop_on_short_page: bool,
    },
    Cursor {
        query_parameter: String,
        next_cursor_path: String,
        #[serde(default)]
        initial_cursor: Option<String>,
    },
}

pub async fn fetch_rest_source(configuration: &RestSourceConfiguration) -> Result<Vec<u8>, String> {
    configuration.validate().await?;
    let headers = configuration.headers()?;
    let mut url = Url::parse(&configuration.url)
        .map_err(|error| format!("REST source URL is invalid: {error}"))?;
    for (name, value) in &configuration.query {
        set_query_parameter(&mut url, name, value);
    }
    collect_rest_pages(configuration, url, |request_url, remaining| {
        fetch_with_retries(configuration, request_url, &headers, remaining)
    })
    .await
}

async fn collect_rest_pages<F, Fut>(
    configuration: &RestSourceConfiguration,
    url: Url,
    mut fetch: F,
) -> Result<Vec<u8>, String>
where
    F: FnMut(Url, usize) -> Fut,
    Fut: Future<Output = Result<Vec<u8>, String>>,
{
    let mut page = match &configuration.pagination {
        RestPagination::Page { start, .. } => *start,
        _ => 0,
    };
    let mut cursor = match &configuration.pagination {
        RestPagination::Cursor { initial_cursor, .. } => initial_cursor.clone(),
        _ => None,
    };
    let mut seen_cursors = HashSet::new();
    let mut records = Vec::new();
    let mut bytes_read = 0;

    for page_index in 0..configuration.max_pages {
        let request_url = pagination_url(&url, &configuration.pagination, page, cursor.as_deref());
        let remaining = configuration.max_bytes.saturating_sub(bytes_read);
        if remaining == 0 {
            return Err("REST source exceeded its configured byte limit".into());
        }
        let body = fetch(request_url, remaining).await?;
        bytes_read += body.len();
        let response: Value = serde_json::from_slice(&body)
            .map_err(|error| format!("REST response is not valid JSON: {error}"))?;
        let page_records = extract_records(&response, configuration.record_path.as_deref())?;
        let page_record_count = page_records.len();
        if records.len() + page_record_count > MAX_RECORDS {
            return Err(format!(
                "REST source exceeds the limit of {MAX_RECORDS} records"
            ));
        }
        records.extend(page_records);

        if !advance_pagination(
            &configuration.pagination,
            &response,
            page_index,
            page_record_count,
            &mut page,
            &mut cursor,
            &mut seen_cursors,
        )? {
            let content = serde_json::to_vec(&records)
                .map_err(|error| format!("REST records could not be serialized: {error}"))?;
            if content.len() > configuration.max_bytes {
                return Err("REST records exceeded their configured byte limit".into());
            }
            return Ok(content);
        }
    }

    Err("REST source reached max_pages before pagination completed".into())
}

fn pagination_url(base: &Url, pagination: &RestPagination, page: u64, cursor: Option<&str>) -> Url {
    let mut url = base.clone();
    match pagination {
        RestPagination::None => {}
        RestPagination::Page {
            parameter,
            page_size_parameter,
            page_size,
            ..
        } => {
            set_query_parameter(&mut url, parameter, &page.to_string());
            if let (Some(parameter), Some(size)) = (page_size_parameter, page_size) {
                set_query_parameter(&mut url, parameter, &size.to_string());
            }
        }
        RestPagination::Cursor {
            query_parameter, ..
        } => {
            if let Some(value) = cursor {
                set_query_parameter(&mut url, query_parameter, value);
            }
        }
    }
    url
}

impl RestSourceConfiguration {
    pub async fn validate(&self) -> Result<(), String> {
        if self.max_pages == 0 || self.max_pages > MAX_ALLOWED_PAGES {
            return Err(format!(
                "REST max_pages must be between 1 and {MAX_ALLOWED_PAGES}"
            ));
        }
        if self.max_bytes == 0 || self.max_bytes > MAX_ALLOWED_BYTES {
            return Err(format!(
                "REST max_bytes must be between 1 and {MAX_ALLOWED_BYTES}"
            ));
        }
        if self.timeout_seconds == 0 || self.timeout_seconds > 60 {
            return Err("REST timeout_seconds must be between 1 and 60".into());
        }
        if self.retry_attempts > 5 {
            return Err("REST retry_attempts cannot exceed 5".into());
        }
        self.headers()?;
        let url = Url::parse(&self.url)
            .map_err(|error| format!("REST source URL is invalid: {error}"))?;
        validate_url(&url).await?;
        match &self.pagination {
            RestPagination::Page {
                parameter,
                page_size_parameter,
                page_size,
                ..
            } => {
                validate_parameter_name(parameter)?;
                if page_size_parameter.is_some() != page_size.is_some() {
                    return Err(
                        "REST page_size_parameter and page_size must be configured together".into(),
                    );
                }
                if let Some(parameter) = page_size_parameter {
                    validate_parameter_name(parameter)?;
                }
                if page_size == &Some(0) {
                    return Err("REST page_size must be greater than zero".into());
                }
            }
            RestPagination::Cursor {
                query_parameter,
                next_cursor_path,
                ..
            } => {
                validate_parameter_name(query_parameter)?;
                if next_cursor_path.trim().is_empty() {
                    return Err("REST next_cursor_path is required".into());
                }
            }
            RestPagination::None => {}
        }
        Ok(())
    }

    fn headers(&self) -> Result<HeaderMap, String> {
        validated_headers(&self.headers)
    }
}

async fn fetch_with_retries(
    configuration: &RestSourceConfiguration,
    url: Url,
    headers: &HeaderMap,
    byte_limit: usize,
) -> Result<Vec<u8>, String> {
    let mut last_error = String::new();
    for attempt in 0..=configuration.retry_attempts {
        match fetch_once(
            configuration.timeout_seconds,
            url.clone(),
            headers,
            byte_limit,
            None,
        )
        .await
        {
            Ok(body) => return Ok(body),
            Err(error) if error.retryable && attempt < configuration.retry_attempts => {
                last_error = error.message;
                let delay = 100_u64.saturating_mul(1_u64 << attempt.min(4));
                sleep(Duration::from_millis(delay)).await;
            }
            Err(error) => return Err(error.message),
        }
    }
    Err(last_error)
}

async fn fetch_once(
    timeout_seconds: u64,
    mut url: Url,
    headers: &HeaderMap,
    byte_limit: usize,
    json_body: Option<&Value>,
) -> Result<Vec<u8>, FetchError> {
    for _ in 0..=MAX_REDIRECTS {
        let client = pinned_client(&url, timeout_seconds)
            .await
            .map_err(FetchError::permanent)?;
        let request = json_body.map_or_else(
            || client.get(url.clone()),
            |body| client.post(url.clone()).json(body),
        );
        let response = request
            .headers(headers.clone())
            .send()
            .await
            .map_err(|error| FetchError::retryable(format!("remote request failed: {error}")))?;
        if response.status().is_redirection() {
            let location = response
                .headers()
                .get(LOCATION)
                .ok_or_else(|| FetchError::permanent("REST redirect has no Location header"))?
                .to_str()
                .map_err(|_| FetchError::permanent("REST redirect Location is invalid"))?;
            let redirect = url.join(location).map_err(|error| {
                FetchError::permanent(format!("REST redirect is invalid: {error}"))
            })?;
            if redirect.host_str() != url.host_str() {
                return Err(FetchError::permanent(
                    "REST redirects cannot change the destination host",
                ));
            }
            if url.scheme() == "https" && redirect.scheme() != "https" {
                return Err(FetchError::permanent(
                    "REST redirects cannot downgrade HTTPS to HTTP",
                ));
            }
            validate_url(&redirect)
                .await
                .map_err(FetchError::permanent)?;
            url = redirect;
            continue;
        }
        let status = response.status();
        if !status.is_success() {
            return Err(FetchError {
                message: format!("REST source returned HTTP {status}"),
                retryable: is_retryable_status(status),
            });
        }
        if response
            .content_length()
            .is_some_and(|length| length > byte_limit as u64)
        {
            return Err(FetchError::permanent(
                "REST response exceeds its configured byte limit",
            ));
        }
        let mut body = Vec::new();
        let mut stream = response.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|error| {
                FetchError::retryable(format!("REST response stream failed: {error}"))
            })?;
            if body.len() + chunk.len() > byte_limit {
                return Err(FetchError::permanent(
                    "REST response exceeds its configured byte limit",
                ));
            }
            body.extend_from_slice(&chunk);
        }
        return Ok(body);
    }
    Err(FetchError::permanent(format!(
        "REST source exceeded the limit of {MAX_REDIRECTS} redirects"
    )))
}

pub(crate) async fn fetch_secure_json_post(
    url: &str,
    headers: &HashMap<String, String>,
    body: &Value,
    timeout_seconds: u64,
    retry_attempts: usize,
    byte_limit: usize,
) -> Result<Vec<u8>, String> {
    let url = Url::parse(url).map_err(|error| format!("source URL is invalid: {error}"))?;
    validate_url(&url).await?;
    let headers = validated_headers(headers)?;
    let mut last_error = String::new();
    for attempt in 0..=retry_attempts {
        match fetch_once(
            timeout_seconds,
            url.clone(),
            &headers,
            byte_limit,
            Some(body),
        )
        .await
        {
            Ok(response) => return Ok(response),
            Err(error) if error.retryable && attempt < retry_attempts => {
                last_error = error.message;
                let delay = 100_u64.saturating_mul(1_u64 << attempt.min(4));
                sleep(Duration::from_millis(delay)).await;
            }
            Err(error) => return Err(error.message),
        }
    }
    Err(last_error)
}

pub(crate) async fn validate_secure_remote(
    url: &str,
    headers: &HashMap<String, String>,
) -> Result<(), String> {
    validated_headers(headers)?;
    let url = Url::parse(url).map_err(|error| format!("source URL is invalid: {error}"))?;
    validate_url(&url).await
}

fn validated_headers(values: &HashMap<String, String>) -> Result<HeaderMap, String> {
    let mut headers = HeaderMap::new();
    for (name, value) in values {
        let name = HeaderName::from_bytes(name.as_bytes())
            .map_err(|error| format!("header name is invalid: {error}"))?;
        let value = HeaderValue::from_str(value)
            .map_err(|error| format!("header value is invalid: {error}"))?;
        headers.insert(name, value);
    }
    Ok(headers)
}

async fn pinned_client(url: &Url, timeout_seconds: u64) -> Result<Client, String> {
    validate_url(url).await?;
    let host = url
        .host_str()
        .ok_or_else(|| "REST source URL requires a host".to_owned())?;
    let port = url
        .port_or_known_default()
        .ok_or_else(|| "REST source URL requires a known port".to_owned())?;
    let addresses = resolve_public_addresses(host, port).await?;
    let mut builder = Client::builder()
        .redirect(Policy::none())
        .timeout(Duration::from_secs(timeout_seconds))
        .no_proxy();
    if host.parse::<IpAddr>().is_err() {
        builder = builder.resolve_to_addrs(host, &addresses);
    }
    builder
        .build()
        .map_err(|error| format!("REST HTTP client could not be created: {error}"))
}

async fn validate_url(url: &Url) -> Result<(), String> {
    if !matches!(url.scheme(), "http" | "https") {
        return Err("REST source URL must use HTTP or HTTPS".into());
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err("REST source URL cannot contain credentials".into());
    }
    let host = url
        .host_str()
        .ok_or_else(|| "REST source URL requires a host".to_owned())?;
    let normalized = host.trim_end_matches('.').to_ascii_lowercase();
    let local_suffix = normalized
        .rsplit_once('.')
        .is_some_and(|(_, suffix)| matches!(suffix, "localhost" | "local"));
    if normalized == "localhost" || local_suffix {
        return Err("REST source URL resolves to a local host".into());
    }
    let port = url
        .port_or_known_default()
        .ok_or_else(|| "REST source URL requires a known port".to_owned())?;
    resolve_public_addresses(host, port).await.map(|_| ())
}

async fn resolve_public_addresses(host: &str, port: u16) -> Result<Vec<SocketAddr>, String> {
    let addresses = timeout(Duration::from_secs(5), lookup_host((host, port)))
        .await
        .map_err(|_| "REST source DNS resolution timed out".to_owned())?
        .map_err(|error| format!("REST source host could not be resolved: {error}"))?
        .collect::<Vec<_>>();
    if addresses.is_empty() {
        return Err("REST source host did not resolve to an address".into());
    }
    if addresses.iter().any(|address| !is_public_ip(address.ip())) {
        return Err("REST source host resolves to a private or reserved address".into());
    }
    Ok(addresses)
}

fn is_public_ip(address: IpAddr) -> bool {
    match address {
        IpAddr::V4(address) => is_public_ipv4(address),
        IpAddr::V6(address) => is_public_ipv6(address),
    }
}

fn is_public_ipv4(address: Ipv4Addr) -> bool {
    let [a, b, ..] = address.octets();
    !(address.is_private()
        || address.is_loopback()
        || address.is_link_local()
        || address.is_broadcast()
        || address.is_unspecified()
        || address.is_multicast()
        || a == 0
        || (a == 100 && (64..=127).contains(&b))
        || (a == 192 && b == 0)
        || (a == 198 && matches!(b, 18 | 19 | 51))
        || (a == 203 && b == 0)
        || a >= 240)
}

fn is_public_ipv6(address: Ipv6Addr) -> bool {
    let segments = address.segments();
    !(address.is_loopback()
        || address.is_unspecified()
        || address.is_multicast()
        || (segments[0] & 0xfe00) == 0xfc00
        || (segments[0] & 0xffc0) == 0xfe80
        || (segments[0] == 0x2001 && segments[1] == 0x0db8))
        && address.to_ipv4_mapped().is_none_or(is_public_ipv4)
}

fn extract_records(response: &Value, path: Option<&str>) -> Result<Vec<Value>, String> {
    let selected = path
        .filter(|path| !path.is_empty())
        .map_or(Some(response), |path| value_at_path(response, path))
        .ok_or_else(|| "REST record_path does not exist in the response".to_owned())?;
    match selected {
        Value::Array(records) => Ok(records.clone()),
        Value::Object(_) => Ok(vec![selected.clone()]),
        _ => Err("REST record_path must select an object or array".into()),
    }
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

#[allow(clippy::too_many_arguments)]
fn advance_pagination(
    pagination: &RestPagination,
    response: &Value,
    page_index: usize,
    record_count: usize,
    page: &mut u64,
    cursor: &mut Option<String>,
    seen_cursors: &mut HashSet<String>,
) -> Result<bool, String> {
    match pagination {
        RestPagination::None => Ok(false),
        RestPagination::Page {
            page_size,
            stop_on_short_page,
            ..
        } => {
            if record_count == 0
                || (*stop_on_short_page && page_size.is_some_and(|size| record_count < size))
            {
                return Ok(false);
            }
            *page = page
                .checked_add(1)
                .ok_or_else(|| "REST page number overflowed".to_owned())?;
            Ok(true)
        }
        RestPagination::Cursor {
            next_cursor_path, ..
        } => {
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
                    "REST cursor pagination repeated a cursor on page {}",
                    page_index + 1
                ));
            }
            *cursor = Some(next);
            Ok(true)
        }
    }
}

fn set_query_parameter(url: &mut Url, name: &str, value: &str) {
    let mut parameters = url
        .query_pairs()
        .filter(|(existing, _)| existing != name)
        .map(|(name, value)| (name.into_owned(), value.into_owned()))
        .collect::<Vec<_>>();
    parameters.push((name.to_owned(), value.to_owned()));
    url.query_pairs_mut().clear().extend_pairs(parameters);
}

fn validate_parameter_name(value: &str) -> Result<(), String> {
    if value.is_empty()
        || value.len() > 128
        || !value.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '_' | '-' | '.' | '[' | ']')
        })
    {
        return Err(format!("REST query parameter '{value}' is invalid"));
    }
    Ok(())
}

fn is_retryable_status(status: StatusCode) -> bool {
    status == StatusCode::TOO_MANY_REQUESTS || status.is_server_error()
}

#[derive(Debug)]
struct FetchError {
    message: String,
    retryable: bool,
}

impl FetchError {
    fn permanent(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            retryable: false,
        }
    }

    fn retryable(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            retryable: true,
        }
    }
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

fn default_page_parameter() -> String {
    "page".into()
}

const fn default_start_page() -> u64 {
    1
}

const fn default_true() -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        collections::VecDeque,
        sync::{Arc, Mutex},
    };

    #[test]
    fn rejects_private_and_reserved_addresses() {
        for address in [
            "127.0.0.1",
            "10.0.0.1",
            "169.254.1.1",
            "192.168.1.1",
            "100.64.0.1",
            "192.0.2.1",
            "::1",
            "fc00::1",
            "fe80::1",
            "2001:db8::1",
        ] {
            assert!(!is_public_ip(address.parse().unwrap()), "{address}");
        }
        assert!(is_public_ip("1.1.1.1".parse().unwrap()));
        assert!(is_public_ip("2606:4700:4700::1111".parse().unwrap()));
    }

    #[test]
    fn extracts_dot_and_json_pointer_record_paths() {
        let response = serde_json::json!({"data": {"items": [{"id": 1}, {"id": 2}]}});
        assert_eq!(
            extract_records(&response, Some("data.items"))
                .unwrap()
                .len(),
            2
        );
        assert_eq!(
            extract_records(&response, Some("/data/items"))
                .unwrap()
                .len(),
            2
        );
    }

    #[test]
    fn advances_page_and_cursor_pagination_safely() {
        let mut page = 1;
        let mut cursor = None;
        let mut seen = HashSet::new();
        let page_config = RestPagination::Page {
            parameter: "page".into(),
            start: 1,
            page_size_parameter: Some("limit".into()),
            page_size: Some(2),
            stop_on_short_page: true,
        };
        assert!(
            advance_pagination(
                &page_config,
                &Value::Null,
                0,
                2,
                &mut page,
                &mut cursor,
                &mut seen,
            )
            .unwrap()
        );
        assert_eq!(page, 2);
        assert!(
            !advance_pagination(
                &page_config,
                &Value::Null,
                1,
                1,
                &mut page,
                &mut cursor,
                &mut seen,
            )
            .unwrap()
        );

        let cursor_config = RestPagination::Cursor {
            query_parameter: "cursor".into(),
            next_cursor_path: "meta.next".into(),
            initial_cursor: None,
        };
        assert!(
            advance_pagination(
                &cursor_config,
                &serde_json::json!({"meta": {"next": "abc"}}),
                0,
                2,
                &mut page,
                &mut cursor,
                &mut seen,
            )
            .unwrap()
        );
        assert_eq!(cursor.as_deref(), Some("abc"));
        assert!(
            advance_pagination(
                &cursor_config,
                &serde_json::json!({"meta": {"next": "abc"}}),
                1,
                2,
                &mut page,
                &mut cursor,
                &mut seen,
            )
            .is_err()
        );
    }

    #[test]
    fn retries_only_rate_limits_and_server_errors() {
        assert!(is_retryable_status(StatusCode::TOO_MANY_REQUESTS));
        assert!(is_retryable_status(StatusCode::BAD_GATEWAY));
        assert!(!is_retryable_status(StatusCode::BAD_REQUEST));
        assert!(!is_retryable_status(StatusCode::UNAUTHORIZED));
    }

    #[test]
    fn replaces_existing_query_parameters() {
        let mut url = Url::parse("https://example.com/items?page=1&active=true").unwrap();
        set_query_parameter(&mut url, "page", "2");
        assert_eq!(url.as_str(), "https://example.com/items?active=true&page=2");
    }

    #[tokio::test]
    async fn collects_paginated_records_with_query_parameters() {
        let configuration = RestSourceConfiguration {
            url: "https://example.com/items?active=true".into(),
            headers: HashMap::new(),
            query: HashMap::new(),
            record_path: Some("data.items".into()),
            pagination: RestPagination::Page {
                parameter: "page".into(),
                start: 1,
                page_size_parameter: Some("limit".into()),
                page_size: Some(2),
                stop_on_short_page: true,
            },
            max_pages: 10,
            max_bytes: 10_000,
            timeout_seconds: 5,
            retry_attempts: 0,
        };
        let responses = Arc::new(Mutex::new(VecDeque::from([
            br#"{"data":{"items":[{"id":1},{"id":2}]}}"#.to_vec(),
            br#"{"data":{"items":[{"id":3}]}}"#.to_vec(),
        ])));
        let requested = Arc::new(Mutex::new(Vec::new()));
        let result = collect_rest_pages(&configuration, Url::parse(&configuration.url).unwrap(), {
            let responses = Arc::clone(&responses);
            let requested = Arc::clone(&requested);
            move |url, _| {
                requested.lock().unwrap().push(url);
                let response = responses.lock().unwrap().pop_front().unwrap();
                async move { Ok(response) }
            }
        })
        .await
        .unwrap();

        let records: Vec<Value> = serde_json::from_slice(&result).unwrap();
        assert_eq!(records.len(), 3);
        let requested = requested.lock().unwrap();
        assert_eq!(requested.len(), 2);
        assert!(requested[0].query().unwrap().contains("page=1"));
        assert!(requested[0].query().unwrap().contains("limit=2"));
        assert!(requested[1].query().unwrap().contains("page=2"));
    }
}
