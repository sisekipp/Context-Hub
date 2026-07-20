# REST connectors

REST sources use `DataSource.kind = DATA_SOURCE_KIND_REST`. Their `configuration_json` follows this shape:

```json
{
  "url": "https://api.example.com/services",
  "headers": {
    "accept": "application/json"
  },
  "query": {
    "active": "true"
  },
  "record_path": "data.items",
  "pagination": {
    "mode": "page",
    "parameter": "page",
    "start": 1,
    "page_size_parameter": "limit",
    "page_size": 250,
    "stop_on_short_page": true
  },
  "max_pages": 100,
  "max_bytes": 33554432,
  "timeout_seconds": 30,
  "retry_attempts": 2
}
```

`record_path` accepts dotted paths such as `data.items` or JSON Pointer paths such as `/data/items`. The selected value must be an object or an array. Objects become one record; page arrays are merged and passed as one JSON source to the existing DataFusion mapping pipeline.

Cursor pagination uses this alternative:

```json
{
  "mode": "cursor",
  "query_parameter": "cursor",
  "next_cursor_path": "meta.next_cursor",
  "initial_cursor": null
}
```

The connector only performs GET requests. It rejects URL credentials, localhost, private, link-local and reserved IP ranges, mixed public/private DNS answers, cross-host redirects, HTTPS downgrades, and responses outside the configured limits. Redirects are resolved and validated one step at a time. DNS answers are checked and pinned into the request client to limit DNS-rebinding exposure.

`Authorization`, `Cookie`, `Proxy-Authorization`, and `X-API-Key` cannot be stored in `configuration_json`. They remain disabled until encrypted credential envelopes are implemented.
