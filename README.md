# ContextHub

ContextHub is an ontology-driven property graph. Teams define their own object types, properties, links, and interfaces, map source data into that ontology, and explore the resulting graph in 2D, 3D, gRPC, or read-only MCP clients.

The repository is a greenfield V1 implementation inspired by the semantic model of Palantir Foundry and the Rust/ClickHouse architecture of GitLab Orbit. No Orbit source code is included.

## What is implemented

- A versioned ontology domain with object and link types, interfaces, value types, structs, shared and derived properties, and read-only functions.
- Validation for API names, identities, reusable type references, function boundaries, link targets, enums, and interface cycles.
- Optimistic draft revisions and immutable publish versions in the gRPC service.
- A declarative `MappingPlan` compiled into safe Apache DataFusion SQL expressions.
- JSON, NDJSON, and CSV source uploads through gRPC-Web into MinIO, with a 32 MiB per-request development limit.
- A background ingestion path that reads uploaded objects as Arrow batches, executes the ontology-specific mapping through DataFusion, and writes nodes and links to ClickHouse.
- Bounded, tenant-scoped graph query compilation into parameterized ClickHouse SQL.
- Ontology-version-scoped graph batch ingestion with stable node/edge IDs and persistent ClickHouse storage.
- A working `GraphService` for bounded node queries, traversals, edge results, and single-object lookup.
- A unified ClickHouse schema for control-plane metadata and property-graph data.
- Durable ClickHouse control-plane repositories for ontology drafts/versions, data-source metadata, ontology-specific mappings, and ingestion jobs, including runtime reload after restart.
- gRPC and gRPC-Web contracts for workspaces, ontologies, data sources, ingestion, and graph queries.
- Read-only MCP tools for schema discovery and graph access.
- A Next.js frontend with an editable React Flow ontology/link editor, revisioned ontology-bound JSON/NDJSON/CSV object and link mappings, and a connected 2D/3D graph explorer.
- A workspace ontology switcher: users can create multiple isolated ontologies, while data-source definitions remain reusable at workspace level and mappings remain ontology-specific.
- A Devcontainer with ClickHouse and MinIO. Local development uses `AUTH_MODE=dev`; no authentication service is started.

## Quick start

The preferred path is **Dev Containers: Reopen in Container**. The post-create hook installs JavaScript and Rust dependencies and generates the TypeScript Protobuf files.

Inside the container:

```bash
mise run db:up
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

The browser uploads selected JSON, NDJSON, and CSV files to the backend and stores them durably in MinIO. It still also parses those files locally to provide an immediate mapping preview and in-session 2D/3D result. The backend can independently execute a saved mapping with `IngestionService.Start`: it reads the shared source from MinIO, maps it with Arrow/DataFusion, and persists the ontology-version-scoped graph in ClickHouse.

The browser import action now executes the complete backend sequence: publish the selected ontology, save its source-specific mapping, start DataFusion ingestion, poll the durable job, and open the local graph preview after success. Ontologies, versions, source metadata, mappings, job results, and graph data survive restarts. After a browser refresh, the explorer reloads a bounded active-version graph through `GraphService`; the current hydration budget is 5,000 objects per ontology query set.

The worker executes every Object Mapping in one mapping bundle over shared Arrow batches. It merges nodes across plans before resolving links, so references can target objects produced by another plan in the same job. List-valued references expand into individual edges, and the visual missing-target strategies Create, Skip, and Error are enforced by the worker. Legacy single-plan mappings remain readable. REST and GraphQL connectors, multipart uploads beyond 32 MiB, restartable queued/running workers, cursor-based explorer expansion, typed property-index writes, and production authentication remain future work.
