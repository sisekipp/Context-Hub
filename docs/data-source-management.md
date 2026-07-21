# Data source management

Data sources belong to the workspace and can be reused by multiple ontologies. The **Data sources** view shows their type, backing file or connector name, and every ontology mapping that currently references them.

The view supports these operations:

- Test any source through the bounded `DataSourceService.Preview` path.
- Rename file, REST, and GraphQL sources.
- Edit REST and GraphQL connector configuration without changing source identity.
- Open a source directly in the active ontology's mapping assistant.
- Delete an unused source.

`DataSourceService.GetUsage` returns mapping and ontology names for a source. `DataSourceService.Delete` repeats the usage check server-side and returns `FAILED_PRECONDITION` while mappings exist, so the UI cannot bypass referential protection. Upload configuration and source kind are immutable; replacing a file creates a new source. When an unused upload is deleted, its control-plane row is tombstoned in ClickHouse and the corresponding MinIO object is removed.

Deleting a source does not delete imported graph nodes. Graph data remains scoped to the immutable ontology version and can be managed independently when provenance and retention controls are added.
