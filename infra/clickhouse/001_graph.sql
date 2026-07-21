CREATE DATABASE IF NOT EXISTS context_hub;

CREATE TABLE IF NOT EXISTS context_hub.workspaces (
  id UUID,
  name String,
  slug String,
  revision UInt64,
  deleted Bool DEFAULT false,
  created_at DateTime64(6, 'UTC') DEFAULT now64(6),
  updated_at DateTime64(6, 'UTC') DEFAULT now64(6)
) ENGINE = ReplacingMergeTree(revision, deleted)
ORDER BY id;

CREATE TABLE IF NOT EXISTS context_hub.workspace_members (
  workspace_id UUID,
  subject String,
  role Enum8('owner' = 1, 'editor' = 2, 'viewer' = 3),
  revision UInt64,
  deleted Bool DEFAULT false,
  updated_at DateTime64(6, 'UTC') DEFAULT now64(6)
) ENGINE = ReplacingMergeTree(revision, deleted)
ORDER BY (workspace_id, subject);

CREATE TABLE IF NOT EXISTS context_hub.ontologies (
  id UUID,
  workspace_id UUID,
  name String,
  slug String,
  active_version_id Nullable(UUID),
  revision UInt64,
  deleted Bool DEFAULT false,
  created_at DateTime64(6, 'UTC') DEFAULT now64(6),
  updated_at DateTime64(6, 'UTC') DEFAULT now64(6)
) ENGINE = ReplacingMergeTree(revision, deleted)
ORDER BY (workspace_id, id);

CREATE TABLE IF NOT EXISTS context_hub.ontology_drafts (
  id UUID,
  workspace_id UUID,
  revision UInt64,
  definition_json String,
  layout_json String DEFAULT '{}',
  updated_at DateTime64(6, 'UTC') DEFAULT now64(6),
  deleted Bool DEFAULT false
) ENGINE = ReplacingMergeTree(revision, deleted)
ORDER BY (workspace_id, id);

CREATE TABLE IF NOT EXISTS context_hub.ontology_versions (
  id UUID,
  workspace_id UUID,
  ontology_id UUID,
  version UInt64,
  definition_json String,
  checksum FixedString(64),
  published_at DateTime64(6, 'UTC') DEFAULT now64(6),
  active Bool DEFAULT false
) ENGINE = ReplacingMergeTree(published_at)
ORDER BY (workspace_id, ontology_id, version);

CREATE TABLE IF NOT EXISTS context_hub.data_sources (
  id UUID,
  workspace_id UUID,
  name String,
  kind Enum8('upload' = 1, 'rest' = 2, 'graphql' = 3),
  configuration_json String,
  credential_envelope String DEFAULT '',
  mapping_plan_json String DEFAULT '',
  revision UInt64,
  deleted Bool DEFAULT false,
  created_at DateTime64(6, 'UTC') DEFAULT now64(6),
  updated_at DateTime64(6, 'UTC') DEFAULT now64(6)
) ENGINE = ReplacingMergeTree(revision, deleted)
ORDER BY (workspace_id, id);

-- Data sources are shared within a workspace. Their interpretation belongs to an ontology,
-- therefore mapping plans live in this association table rather than on data_sources.
CREATE TABLE IF NOT EXISTS context_hub.ontology_data_mappings (
  id UUID,
  workspace_id UUID,
  ontology_id UUID,
  data_source_id UUID,
  name String,
  mapping_plan_json String,
  revision UInt64,
  deleted Bool DEFAULT false,
  created_at DateTime64(6, 'UTC') DEFAULT now64(6),
  updated_at DateTime64(6, 'UTC') DEFAULT now64(6)
) ENGINE = ReplacingMergeTree(revision, deleted)
ORDER BY (workspace_id, ontology_id, data_source_id, id);

CREATE TABLE IF NOT EXISTS context_hub.ingestion_jobs (
  id UUID,
  workspace_id UUID,
  data_source_id UUID,
  ontology_mapping_id UUID,
  ontology_version_id UUID,
  state Enum8('queued' = 1, 'running' = 2, 'succeeded' = 3, 'failed' = 4, 'cancelled' = 5),
  stats_json String DEFAULT '{}',
  error String DEFAULT '',
  revision UInt64,
  created_at DateTime64(6, 'UTC') DEFAULT now64(6),
  started_at Nullable(DateTime64(6, 'UTC')),
  completed_at Nullable(DateTime64(6, 'UTC')),
  updated_at DateTime64(6, 'UTC') DEFAULT now64(6)
) ENGINE = ReplacingMergeTree(revision)
ORDER BY (workspace_id, id);

CREATE TABLE IF NOT EXISTS context_hub.graph_nodes (
  workspace_id UUID,
  ontology_version_id UUID,
  object_type LowCardinality(String),
  object_id String,
  source_id UUID,
  external_id String,
  properties JSON(max_dynamic_paths = 1024),
  version UInt64,
  deleted Bool DEFAULT false,
  source_updated_at Nullable(DateTime64(6, 'UTC')),
  ingested_at DateTime64(6, 'UTC') DEFAULT now64(6)
) ENGINE = ReplacingMergeTree(version, deleted)
ORDER BY (workspace_id, ontology_version_id, object_type, object_id);

CREATE TABLE IF NOT EXISTS context_hub.graph_edges (
  workspace_id UUID,
  ontology_version_id UUID,
  link_type LowCardinality(String),
  edge_id String,
  source_type LowCardinality(String),
  source_id String,
  target_type LowCardinality(String),
  target_id String,
  data_source_id UUID,
  properties JSON(max_dynamic_paths = 256),
  version UInt64,
  deleted Bool DEFAULT false,
  ingested_at DateTime64(6, 'UTC') DEFAULT now64(6)
) ENGINE = ReplacingMergeTree(version, deleted)
ORDER BY (workspace_id, ontology_version_id, link_type, source_id, target_id, edge_id);

CREATE TABLE IF NOT EXISTS context_hub.property_string_index (
  workspace_id UUID, ontology_version_id UUID, object_type LowCardinality(String), property LowCardinality(String), value String, object_id String, version UInt64, deleted Bool
) ENGINE = ReplacingMergeTree(version, deleted)
ORDER BY (workspace_id, ontology_version_id, object_type, property, value, object_id);

CREATE TABLE IF NOT EXISTS context_hub.property_number_index (
  workspace_id UUID, ontology_version_id UUID, object_type LowCardinality(String), property LowCardinality(String), value Decimal128(12), object_id String, version UInt64, deleted Bool
) ENGINE = ReplacingMergeTree(version, deleted)
ORDER BY (workspace_id, ontology_version_id, object_type, property, value, object_id);

CREATE TABLE IF NOT EXISTS context_hub.property_boolean_index (
  workspace_id UUID, ontology_version_id UUID, object_type LowCardinality(String), property LowCardinality(String), value Bool, object_id String, version UInt64, deleted Bool
) ENGINE = ReplacingMergeTree(version, deleted)
ORDER BY (workspace_id, ontology_version_id, object_type, property, value, object_id);

CREATE TABLE IF NOT EXISTS context_hub.property_timestamp_index (
  workspace_id UUID, ontology_version_id UUID, object_type LowCardinality(String), property LowCardinality(String), value DateTime64(6, 'UTC'), object_id String, version UInt64, deleted Bool
) ENGINE = ReplacingMergeTree(version, deleted)
ORDER BY (workspace_id, ontology_version_id, object_type, property, value, object_id);

CREATE TABLE IF NOT EXISTS context_hub.ingestion_events (
  workspace_id UUID,
  job_id UUID,
  source_id UUID,
  event_type LowCardinality(String),
  row_number UInt64,
  object_type String,
  external_id String,
  message String,
  details JSON(max_dynamic_paths = 128),
  occurred_at DateTime64(6, 'UTC') DEFAULT now64(6)
) ENGINE = MergeTree
ORDER BY (workspace_id, job_id, occurred_at, row_number);

CREATE TABLE IF NOT EXISTS context_hub.function_artifacts (
  id UUID,
  workspace_id UUID,
  name String,
  file_name String,
  object_key String,
  size_bytes UInt64,
  sha256 FixedString(64),
  revision UInt64,
  deleted Bool DEFAULT false,
  created_at DateTime64(6, 'UTC') DEFAULT now64(6)
) ENGINE = ReplacingMergeTree(revision, deleted)
ORDER BY (workspace_id, id);

CREATE TABLE IF NOT EXISTS context_hub.function_executions (
  id UUID,
  workspace_id UUID,
  ontology_version_id UUID,
  function_api_name LowCardinality(String),
  executor LowCardinality(String),
  state Enum8('succeeded' = 1, 'failed' = 2),
  arguments_json String,
  result_json String,
  error String,
  duration_millis UInt64,
  executed_at DateTime64(6, 'UTC') DEFAULT now64(6)
) ENGINE = MergeTree
ORDER BY (workspace_id, ontology_version_id, function_api_name, executed_at, id);

INSERT INTO context_hub.workspaces (id, name, slug, revision)
SELECT toUUID('00000000-0000-0000-0000-000000000001'), 'Development', 'development', 1
WHERE NOT EXISTS (
  SELECT 1 FROM context_hub.workspaces FINAL
  WHERE id = toUUID('00000000-0000-0000-0000-000000000001') AND deleted = false
);

INSERT INTO context_hub.workspace_members (workspace_id, subject, role, revision)
SELECT toUUID('00000000-0000-0000-0000-000000000001'), 'dev-user', 'owner', 1
WHERE NOT EXISTS (
  SELECT 1 FROM context_hub.workspace_members FINAL
  WHERE workspace_id = toUUID('00000000-0000-0000-0000-000000000001')
    AND subject = 'dev-user' AND deleted = false
);
