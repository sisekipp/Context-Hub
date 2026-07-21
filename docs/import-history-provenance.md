# Import history and provenance

The **Imports** workspace view is scoped to the active ontology. It lists durable ingestion jobs newest first while resolving their data-source names from the shared workspace catalog. Selecting a job shows its source, immutable ontology version, mapping ID, timestamps, counters, terminal error, and detailed execution events. **Retry** creates a new job with the same source, mapping, and ontology version; it never overwrites the original audit record.

`IngestionService.ListJobs`, `Retry`, and `ListEvents` expose the same workflow to gRPC and gRPC-Web clients. Job state and timestamps are stored in `ingestion_jobs`. `started`, `completed`, `failed`, `row_rejected`, and `field_null` records are appended to `ingestion_events`. Field-level events include the one-based source row, target object type, selected error strategy, target field, and safe error message. Connector secrets and complete rejected source records are deliberately not copied into the event log.

The Explorer requests property origins only after a user selects a node. `GraphService.GetObjectProvenance` combines the node's stored source ID with the matching ontology mapping and latest successful job for that immutable ontology version. Each mapped property can therefore display:

- shared data-source name and ID;
- original source field;
- ontology mapping name and ID;
- ingestion job and completion time.

This V1 lineage is deterministic for directly mapped properties. It describes the mapping that produced the current node version, but does not yet store cell-level transformation steps or a copy of the original value. Links retain their source ID in ClickHouse, but the current endpoint returns node-property lineage only.
