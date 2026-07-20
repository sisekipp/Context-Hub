# ContextHub

ContextHub is an ontology-driven property graph. Teams define their own object types, properties, links, and interfaces, map source data into that ontology, and explore the resulting graph in 2D, 3D, gRPC, or read-only MCP clients.

The repository is a greenfield V1 implementation inspired by the semantic model of Palantir Foundry and the Rust/ClickHouse architecture of GitLab Orbit. No Orbit source code is included.

## What is implemented

- A versioned ontology domain with object and link types, interfaces, value types, structs, shared and derived properties, and read-only functions.
- Validation for API names, identities, reusable type references, function boundaries, link targets, enums, and interface cycles.
- Optimistic draft revisions and immutable publish versions in the gRPC service.
- A declarative `MappingPlan` compiled into safe Apache DataFusion SQL expressions.
- Bounded, tenant-scoped graph query compilation into parameterized ClickHouse SQL.
- A unified ClickHouse schema for control-plane metadata and property-graph data.
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

The current browser vertical slice parses JSON, NDJSON, and CSV locally and passes the resulting objects and links directly to the explorer. The Rust Arrow/DataFusion and ClickHouse components are present, but durable upload/job/reload wiring remains a production integration; refreshing the browser clears an imported graph.

Each ontology currently keeps its own local editor draft, mapping draft, and in-session explorer graph. Shared-source persistence and durable per-ontology graph imports are the next integration step.
