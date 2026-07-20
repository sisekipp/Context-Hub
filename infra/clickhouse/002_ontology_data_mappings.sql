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
