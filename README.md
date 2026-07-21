# ContextHub

ContextHub is an ontology-driven property graph. Teams define their own object types, properties, links, and interfaces, map source data into that ontology, and explore the resulting graph in 2D, 3D, gRPC, or read-only MCP clients.

The repository is a greenfield V1 implementation inspired by the semantic model of Palantir Foundry and the Rust/ClickHouse architecture of GitLab Orbit. No Orbit source code is included.

## What is implemented

- A versioned ontology domain with object and link types, interfaces, value types, structs, shared and derived properties, and read-only functions.
- Validation for API names, identities, reusable type references, function boundaries, link targets, enums, and interface cycles.
- Optimistic backend draft revisions and immutable publish versions in the gRPC service. Work-in-progress drafts may be temporarily invalid; validation remains mandatory at publish time.
- A declarative `MappingPlan` compiled into safe Apache DataFusion SQL expressions, with an ordered visual transformation pipeline and per-field Skip row, Use null, or Abort import strategy.
- JSON, NDJSON, CSV, and Parquet source uploads through gRPC-Web into MinIO, plus secured REST and GraphQL sources with bounded backend previews.
- A background ingestion path that reads uploaded objects as Arrow batches, executes the ontology-specific mapping through DataFusion, and writes nodes and links to ClickHouse.
- Bounded, tenant-scoped graph query compilation into parameterized ClickHouse SQL.
- Ontology-version-scoped graph batch ingestion with stable node/edge IDs and persistent ClickHouse storage.
- A working `GraphService` for bounded node queries, traversals, edge results, and single-object lookup.
- A unified ClickHouse schema for control-plane metadata and property-graph data.
- Durable ClickHouse control-plane repositories for ontology drafts/versions, data-source metadata, ontology-specific mappings, and ingestion jobs, including runtime reload after restart.
- Ontology-scoped import history with durable field/row events, safe retries, and source-field provenance in the Explorer property inspector.
- gRPC and gRPC-Web contracts for workspaces, ontologies, data sources, ingestion, and graph queries.
- Published, type-checked read-only Function execution through publish-validated controlled expressions, testable allowlisted external gRPC providers, or managed, fuel- and memory-limited WASM modules. Function executions and failures are retained in ClickHouse.
- Read-only MCP tools for schema discovery and graph access.
- A Next.js frontend with a backend-persisted React Flow ontology editor, revisioned ontology-bound JSON/NDJSON/CSV/Parquet object and link mappings, and a connected 2D/3D graph explorer.
- A workspace ontology switcher: users can create multiple isolated ontologies, while data-source definitions remain reusable at workspace level and mappings remain ontology-specific.
- A Devcontainer with ClickHouse and MinIO. Local development uses `AUTH_MODE=dev`; no authentication service is started.

## Quick start

The preferred path is **Dev Containers: Reopen in Container**. Docker Compose starts ClickHouse and MinIO, while the post-create hook installs JavaScript and Rust dependencies and generates the TypeScript Protobuf files.

Inside the container:

```bash
mise run dev:api
mise run dev:mcp
mise run dev:web
```

Open:

- Web UI: <http://localhost:3000>
- gRPC/gRPC-Web: `localhost:50051`
- MCP: <http://localhost:8080/mcp>
- MinIO console: <http://localhost:9001>
- ClickHouse HTTP: <http://localhost:8123>

Run all checks with `mise run check`.

## Repository map

```text
apps/web/                    Next.js UI
crates/context-hub-domain/  Ontology types and validation
crates/context-hub-mapping/ Mapping plans and DataFusion execution
crates/context-hub-storage/ ClickHouse adapters and graph query compiler
crates/context-hub-api/     gRPC and gRPC-Web server
crates/context-hub-mcp/     Read-only MCP HTTP server
proto/                      Public API contracts
infra/                      ClickHouse bootstrap schema
.devcontainer/              Reproducible developer environment
```

## V1 boundaries

Actions, scenarios, GeoPoint/GeoShape, Attachment/MediaReference, status/render metadata, write-capable MCP tools, direct database connectors, and arbitrary user SQL are deliberately excluded. Functions are included as read-only expression, external gRPC, or WASM definitions. ConnectorX is reserved for a later direct-database connector milestone.

The browser uploads selected JSON, NDJSON, CSV, and Parquet files to the backend and stores them durably in MinIO. Text formats are parsed locally for an immediate mapping preview. Parquet is decoded by the backend into a bounded JSON preview while its original columnar bytes remain Arrow-native for DataFusion ingestion. REST and GraphQL sources are fetched and normalized by the backend so browser CORS rules do not affect the mapping workflow. `IngestionService.Start` loads the shared source through its connector, maps it with Arrow/DataFusion, and persists the ontology-version-scoped graph in ClickHouse. See [Parquet imports](docs/parquet-imports.md).

The **Data sources** workspace view lists every saved source and the ontology mappings that use it. Sources can be tested and renamed there; REST and GraphQL configurations can also be edited. Deletion is allowed only when no ontology mapping references the source, and deleting an upload removes its MinIO object. See [Data source management](docs/data-source-management.md).

The Explorer includes a visual graph-query builder for ontology-validated filters, property projections, sorting, bounded aggregations, and directed traversals. Selected nodes can load their one-hop neighborhood from ClickHouse without replacing the current 2D/3D graph. See [Graph query builder](docs/graph-query-builder.md).

The **Imports** view shows durable job history, counters, timestamps, field/row errors, source details, and a retry action that creates a separate audit record. Selecting a graph node resolves each directly mapped property back to its shared source, source field, mapping, and successful ingestion job. See [Import history and provenance](docs/import-history-provenance.md).

The ontology editor publishes Function definitions with typed inputs and outputs and can execute the published version from its inspector. Function nodes can be duplicated or deleted. Expressions use a fixed language rather than SQL or shell access and are validated during publish. External providers implement the `ExternalFunctionService` envelope and can be tested before publishing; WASM artifacts are uploaded and managed in the inspector, read from MinIO, and execute without WASI or host imports. Execution results and detailed failures are available in the Function history. See [Function execution](docs/function-execution.md).

Ontology canvas state is stored with the definition in the revisioned ClickHouse draft. The editor reconstructs older definitions that have no compatible layout, migrates legacy browser-local drafts once, and autosaves only documents that differ from the last backend-confirmed state. Object types, links, interfaces, value types, structs, shared and derived properties, and Functions survive reloads without being flattened. Node/property deletion and bounded Undo/Redo are available before publish.

The browser import action now executes the complete backend sequence: publish the selected ontology, save its source-specific mapping, start DataFusion ingestion, poll the durable job, and open the local graph preview after success. Ontologies, versions, source metadata, mappings, job results, and graph data survive restarts. Workspace upload sources are listed from ClickHouse after a browser refresh; selecting one checksum-validates and restores its original MinIO object into the mapping assistant. The explorer reloads the active-version graph through `GraphService` in 2,000-object keyset pages. Users can incrementally merge additional pages into the current 2D/3D view without reloading already completed query branches.

The worker executes every Object Mapping in one mapping bundle over shared Arrow batches. It merges nodes across plans before resolving links, so references can target objects produced by another plan in the same job. List-valued references expand into individual edges, and the visual missing-target strategies Create, Skip, and Error are enforced by the worker. Each property can use an ordered pipeline of casts, string operations, defaults, field combinations, arithmetic, and date parsing; the browser preview applies the same ordering before the plan is compiled for DataFusion. Legacy single-transform browser drafts and single-plan backend mappings remain readable. On API startup, persisted queued or interrupted running jobs are resumed idempotently; jobs whose source, mapping, or ontology version disappeared are finalized as failed. Properties marked `Indexed` are derived from the published ontology and written to ClickHouse's typed string, number, boolean, and timestamp indexes; list values produce one index row per value. Graph filters automatically use these indexes with ontology-aware value validation, while unindexed properties retain the parameterized JSON fallback. The REST and GraphQL source forms load secured backend previews and feed imports into the same DataFusion worker; see [REST connectors](docs/rest-connectors.md) and [GraphQL connectors](docs/graphql-connectors.md). Multipart uploads beyond 32 MiB, encrypted connector credentials, and production authentication remain future work.
