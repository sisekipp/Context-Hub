# Architecture

## Data flow

1. A user edits an ontology draft in the React Flow editor.
2. `OntologyService.Validate` applies the Rust domain validator.
3. `OntologyService.Publish` creates an immutable version with a SHA-256 checksum.
4. File, REST, or GraphQL records are normalized into Arrow `RecordBatch` streams.
5. A declarative mapping plan is compiled to DataFusion expressions and executed on those batches.
6. Identity fields produce stable object IDs; link mappings resolve target identities through bounded joins.
7. Nodes, edges, typed property indices, and provenance events are batch-written to ClickHouse.
8. `GraphService` validates a typed query against the active ontology and compiles a parameterized, workspace-scoped ClickHouse query.
9. The UI and read-only MCP surface receive only bounded graph results.

The current UI vertical slice also offers an immediate local file path: JSON, NDJSON, and CSV records are parsed in the browser, mapped into the shared graph model, and handed to the 2D/3D explorer. This removes demo counts and demo nodes while the durable worker-to-ClickHouse ingestion loop is still being connected.

## Storage

ClickHouse is the unified control and data plane. Revisioned `ReplacingMergeTree` tables store workspaces, drafts, versions, data-source definitions, mapping plans, credential envelopes, and ingestion jobs. The same database stores versioned nodes, edges, typed property indices, and ingestion events. Graph sort keys begin with `workspace_id` and `ontology_version_id`. API callers never receive raw SQL access.

Uploads are addressed through Apache Arrow's `object_store` abstraction. MinIO is only the local S3-compatible implementation.

## Security invariants

- Every graph query carries a workspace ID derived from authenticated context, not from an unrestricted client choice in production mode.
- Ontology and mapping identifiers follow a restricted snake-case grammar.
- Query values are bound parameters and are never interpolated into SQL.
- Traversals have a maximum depth of six.
- Default graph budgets are 5,000 nodes for 2D and 2,000 for 3D.
- Connectors must reject local/private destinations, revalidate redirects, and enforce response, page, and time limits before production enablement.
- MCP annotations and server behavior are read-only.

## Remaining production integrations

The vertical slice intentionally keeps persistence wiring replaceable. Before production deployment, connect the gRPC runtime to `ClickHouseOntologyRepository` and `ClickHouseGraphRepository`, implement the ingestion worker's job claim loop, add credential-envelope encryption, and enable the configured production JWT validator.
