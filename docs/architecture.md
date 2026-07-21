# Architecture

## Data flow

1. A user selects or creates an ontology in a workspace and edits its draft in the React Flow editor.
2. `OntologyService.Validate` applies the Rust domain validator.
3. `OntologyService.Publish` creates an immutable version with a SHA-256 checksum.
4. File, REST, or GraphQL records are normalized into Arrow `RecordBatch` streams.
5. A declarative mapping plan is compiled to DataFusion expressions and executed on those batches.
6. Identity fields produce stable object IDs; link mappings resolve target identities through bounded joins.
7. Nodes, edges, typed property indices, and provenance events are batch-written to ClickHouse.
8. `GraphService` validates a typed query against the active ontology and compiles a parameterized, workspace-scoped ClickHouse query.
9. The UI and read-only MCP surface receive only bounded graph results.

The UI uploads JSON, NDJSON, CSV, and Parquet sources through gRPC-Web into the workspace's MinIO bucket. Text formats are parsed locally for an immediate mapping preview. Parquet schema and records are decoded through Arrow on the backend and returned as a bounded JSON preview; ingestion reads the original Parquet bytes directly. The preview graph is handed to the selected ontology's 2D/3D explorer. Editor drafts, mapping drafts, and in-memory preview graphs are isolated by ontology ID.

The ontology editor loads both `definition_json` and `layout_json` from `OntologyService.GetDraft`. Its React Flow document serializes the complete supported ontology catalog and saves with an expected revision; unchanged documents are not written. Legacy local-storage drafts are retained as a failure fallback and removed after their first successful backend migration. `SaveDraft` accepts temporarily invalid but structurally decodable work-in-progress definitions, while `Validate` and `Publish` run the complete domain validator and immutable-version checks.

## Multi-ontology ownership

A workspace can contain many independent ontologies. Data sources belong to the workspace and can therefore be reused by every ontology in that workspace. The interpretation of a source never lives on the source itself: `ontology_data_mappings` associates one ontology with one shared data source and owns the mapping plan and its revision.

Published graph data is scoped by both `workspace_id` and `ontology_version_id`. Changing the active ontology changes the editor, mappings, ingestion jobs, and graph explorer together. Cross-ontology links are not created implicitly; an explicit future federation feature would be required for those.

`IngestionService.ImportGraph` is the bounded worker-to-storage boundary. It accepts at most 5,000 mapped nodes and 20,000 mapped edges per request, verifies that source, mapping, ontology, workspace, and immutable ontology version belong together, validates every object/link type against that version, and then writes JSONEachRow batches to ClickHouse. Repeated stable IDs are handled by the versioned `ReplacingMergeTree` tables.

`IngestionService.Start` runs the asynchronous worker path. It verifies immutable ontology-version and source/mapping ownership, streams uploaded objects through checksum-validating temporary storage, and converts JSON/NDJSON, CSV, or Parquet into bounded Arrow batches. Every restricted DataFusion object plan runs over those batches; nodes are merged globally before cross-plan links are resolved. Each replica must own an expiring fenced lease before writing. Node and edge offsets are persisted after every bounded ClickHouse batch, so a restarted or replacement worker resumes from the last checkpoint. Stable graph identifiers keep retries idempotent. Browser uploads larger than 32 MiB use resumable multipart sessions and support files up to 5 GiB.

## Storage

ClickHouse is the unified control and data plane. Revisioned `ReplacingMergeTree` tables store workspaces, drafts, versions, workspace-level data-source definitions, ontology-specific mapping plans, credential envelopes, and ingestion jobs. The API loads this control-plane snapshot on startup and persists every mutation through the same repository boundary. The same database stores versioned nodes, edges, typed property indices, and ingestion events. Graph sort keys begin with `workspace_id` and `ontology_version_id`. API callers never receive raw SQL access.

Uploads are addressed through Apache Arrow's `object_store` abstraction. MinIO is only the local S3-compatible implementation; production can use another supported object-store backend without changing mapping execution.

`DataSourceService.List` restores the workspace source registry from ClickHouse. `DataSourceService.GetUpload` is the bounded rehydration path for upload sources: it enforces workspace ownership, rejects non-upload connectors, reads the original MinIO object, and verifies its persisted size and SHA-256 checksum before returning at most the configured 32 MiB upload limit to the mapping UI.

## Security invariants

- Every graph query carries a workspace ID derived from authenticated context, not from an unrestricted client choice in production mode.
- Ontology and mapping identifiers follow a restricted snake-case grammar.
- Query values are bound parameters and are never interpolated into SQL.
- Traversals have a maximum depth of six.
- Default graph budgets are 5,000 nodes for 2D and 2,000 for 3D.
- Remote connectors reject local/private destinations, revalidate redirects, pin validated DNS answers, and enforce response, page, and time limits.
- MCP annotations and server behavior are read-only.

## Remaining production integrations

The gRPC runtime uses ClickHouse repositories for graph and control-plane reads/writes, and MinIO plus DataFusion for uploaded-file ingestion. Arrow supplies native streamed record batches for schema detection and mapping. REST and GraphQL sources share DNS/IP validation, DNS pinning, manually revalidated redirects, streaming byte limits, retries, and AES-256-GCM envelopes for sensitive headers. A bounded `DataSourceService.Preview` call gives the visual mapper normalized records without exposing binary parsing or secrets to the browser. The frontend drives publish/save/start/poll as one import workflow and hydrates the active graph through bounded `GraphService` queries. Before production deployment, add the configured production JWT validator and an operational credential-key rotation procedure.
