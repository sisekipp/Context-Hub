-- Existing development volumes predate ontology-scoped ingestion jobs.
ALTER TABLE context_hub.ingestion_jobs
  ADD COLUMN IF NOT EXISTS ontology_mapping_id UUID AFTER data_source_id;

ALTER TABLE context_hub.ingestion_jobs
  ADD COLUMN IF NOT EXISTS ontology_version_id UUID AFTER ontology_mapping_id;
