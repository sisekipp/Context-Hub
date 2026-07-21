# GraphQL connectors

GraphQL sources use `DataSource.kind = DATA_SOURCE_KIND_GRAPHQL`. In the web app, open **Data mapping → GraphQL** to configure a workspace-wide source and request a normalized preview for the active ontology's mapping.

`configuration_json` follows this shape:

```json
{
  "url": "https://api.example.com/graphql",
  "query": "query Services($after: String) { services(after: $after) { nodes { id name teamId } pageInfo { endCursor } } }",
  "variables": {
    "limit": 250
  },
  "headers": {
    "accept": "application/json"
  },
  "record_path": "data.services.nodes",
  "pagination": {
    "mode": "cursor",
    "variable": "after",
    "next_cursor_path": "data.services.pageInfo.endCursor",
    "initial_cursor": null
  },
  "max_pages": 100,
  "max_bytes": 33554432,
  "timeout_seconds": 30,
  "retry_attempts": 2
}
```

`variables` must be a JSON object. `record_path` and `next_cursor_path` accept dotted paths or JSON Pointer paths. The value selected by `record_path` must be an object or array; page arrays are merged before they enter the existing Arrow/DataFusion mapping pipeline. Cursor pagination writes the cursor from `next_cursor_path` into the configured GraphQL variable for the next request and rejects repeating cursors.

The mapping preview is capped at 20 pages, 8 MiB, and 10,000 returned records. An ingestion job uses the source's configured production limits. GraphQL response errors are reduced to their message fields; response extensions are not returned to clients.

The connector shares the REST connector's outbound-network protection: it rejects URL credentials, localhost, private, link-local and reserved IP ranges, mixed public/private DNS answers, cross-host redirects, HTTPS downgrades, and oversized responses. DNS answers are validated and pinned into the request client.

`Authorization`, `Cookie`, `Proxy-Authorization`, and `X-API-Key` values are stored in an AES-256-GCM credential envelope separate from the public configuration. The API and UI return only a masked placeholder and preserve an unchanged secret when the source is edited.
