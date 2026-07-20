use std::{collections::HashMap, fmt::Write as _, net::SocketAddr, sync::Arc};

use chrono::{DateTime, NaiveDate, Utc};
use context_hub_api::context_hub::v1::{
    CreateOntologyRequest, DataSource, GetIngestionJobRequest, GetObjectRequest,
    GetOntologyDraftRequest, GetUploadDataSourceRequest, GetUploadDataSourceResponse,
    GetWorkspaceRequest, GraphEdge, GraphNode, GraphQuery, ImportGraphRequest, IngestionJob,
    IngestionState, ListDataSourcesRequest, ListDataSourcesResponse, ListOntologiesRequest,
    ListOntologiesResponse, ListOntologyDataMappingsRequest, ListOntologyDataMappingsResponse,
    ListOntologyVersionsRequest, ListOntologyVersionsResponse, ListWorkspacesResponse, Ontology,
    OntologyDataMapping, OntologyDraft, OntologyVersion, PublishOntologyRequest,
    QueryGraphResponse, SaveDataSourceRequest, SaveOntologyDataMappingRequest,
    SaveOntologyDraftRequest, StartIngestionRequest, UploadDataSourceRequest,
    UploadDataSourceResponse, ValidateOntologyRequest, ValidateOntologyResponse, ValidationIssue,
    Workspace,
    data_source_service_server::{DataSourceService, DataSourceServiceServer},
    graph_service_server::{GraphService, GraphServiceServer},
    ingestion_service_server::{IngestionService, IngestionServiceServer},
    ontology_service_server::{OntologyService, OntologyServiceServer},
    workspace_service_server::{WorkspaceService, WorkspaceServiceServer},
};
use context_hub_domain::{
    ObjectTypeDefinition, OntologyDefinition, PropertyDefinition, ScalarType, ValueType,
};
use context_hub_mapping::{
    MappingDocument, MappingPlan, SourceFormat, execute_source_mapping_bundle,
};
use context_hub_storage::{
    ClickHouseControlPlaneRepository, ClickHouseGraphRepository, ControlPlaneRepository,
    ControlPlaneSnapshot, FilterOperator as StorageFilterOperator, GraphEdgeWrite,
    GraphFilter as StorageGraphFilter, GraphNodeWrite, GraphQuery as StorageGraphQuery,
    GraphRepository, ObjectStoreSourceRepository, PropertyIndexValue, PropertyIndexWrite,
    SourceObjectStore, StorageError, StoredDataSource, StoredIngestionJob, StoredOntology,
    StoredOntologyDraft, StoredOntologyMapping, StoredOntologyVersion,
    TraversalStep as StorageTraversalStep,
};
#[cfg(test)]
use context_hub_storage::{MemoryGraphRepository, MemorySourceObjectStore};
use prost_types::Timestamp;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::sync::RwLock;
use tonic::{Request, Response, Status, transport::Server};
use tower_http::cors::{Any, CorsLayer};
use tracing_subscriber::EnvFilter;
use uuid::Uuid;

#[derive(Clone)]
struct Runtime {
    ontologies: Arc<RwLock<HashMap<String, Ontology>>>,
    drafts: Arc<RwLock<HashMap<String, OntologyDraft>>>,
    versions: Arc<RwLock<HashMap<String, Vec<OntologyVersion>>>>,
    data_sources: Arc<RwLock<HashMap<String, DataSource>>>,
    mappings: Arc<RwLock<HashMap<String, OntologyDataMapping>>>,
    jobs: Arc<RwLock<HashMap<String, IngestionJob>>>,
    graph: Arc<dyn GraphRepository>,
    source_store: Arc<dyn SourceObjectStore>,
    control_plane: Option<Arc<dyn ControlPlaneRepository>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct UploadSourceConfiguration {
    object_key: String,
    file_name: String,
    format: SourceFormat,
    size_bytes: u64,
    sha256: String,
}

const MAX_UPLOAD_BYTES: usize = 32 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct GraphCursor {
    version: u8,
    positions: Vec<GraphCursorPosition>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct GraphCursorPosition {
    after_object_id: Option<String>,
    done: bool,
}

impl Runtime {
    #[cfg(test)]
    async fn seeded() -> Self {
        Self::seeded_with_stores(
            Arc::new(MemoryGraphRepository::default()),
            Arc::new(MemorySourceObjectStore::default()),
            None,
        )
        .await
        .expect("memory runtime can be seeded")
    }

    #[allow(clippy::too_many_lines)]
    async fn seeded_with_stores(
        graph: Arc<dyn GraphRepository>,
        source_store: Arc<dyn SourceObjectStore>,
        control_plane: Option<Arc<dyn ControlPlaneRepository>>,
    ) -> Result<Self, StorageError> {
        let runtime = Self {
            ontologies: Arc::default(),
            drafts: Arc::default(),
            versions: Arc::default(),
            data_sources: Arc::default(),
            mappings: Arc::default(),
            jobs: Arc::default(),
            graph,
            source_store,
            control_plane,
        };
        if let Some(repository) = &runtime.control_plane {
            let snapshot = repository
                .load(parse_uuid_storage(&dev_workspace_id())?)
                .await?;
            if !snapshot.ontologies.is_empty() {
                runtime.load_control_plane(snapshot).await?;
                runtime.resume_ingestion_jobs().await;
                return Ok(runtime);
            }
        }
        let ontology_id =
            Uuid::new_v5(&Uuid::NAMESPACE_URL, b"context-hub/dev/service-map").to_string();
        let definition = OntologyDefinition {
            api_name: "service_map".into(),
            display_name: "Service Map".into(),
            description: Some("Development ontology".into()),
            object_types: vec![ObjectTypeDefinition {
                api_name: "service".into(),
                display_name: "Service".into(),
                description: None,
                properties: vec![PropertyDefinition {
                    api_name: "id".into(),
                    display_name: "ID".into(),
                    value_type: ValueType {
                        scalar: ScalarType::String,
                        list: false,
                    },
                    required: true,
                    unique: true,
                    identity: true,
                    indexed: true,
                    description: None,
                }],
                shared_properties: vec![],
                derived_properties: vec![],
                implements: vec![],
            }],
            link_types: vec![],
            interfaces: vec![],
            value_types: vec![],
            struct_types: vec![],
            shared_properties: vec![],
            functions: vec![],
        };
        let now = Utc::now();
        runtime.ontologies.write().await.insert(
            ontology_id.clone(),
            Ontology {
                id: ontology_id.clone(),
                workspace_id: dev_workspace_id(),
                name: definition.display_name.clone(),
                slug: definition.api_name.clone(),
                active_version_id: String::new(),
                revision: 0,
            },
        );
        runtime.drafts.write().await.insert(
            ontology_id.clone(),
            OntologyDraft {
                id: ontology_id,
                workspace_id: dev_workspace_id(),
                name: definition.display_name.clone(),
                slug: definition.api_name.clone(),
                revision: 0,
                definition_json: serde_json::to_string_pretty(&definition)
                    .expect("seed definition serializes"),
                layout_json: "{}".into(),
                updated_at: Some(timestamp(now)),
            },
        );
        if let Some(repository) = &runtime.control_plane {
            let ontology = runtime
                .ontologies
                .read()
                .await
                .values()
                .next()
                .cloned()
                .expect("seed ontology exists");
            let draft = runtime
                .drafts
                .read()
                .await
                .values()
                .next()
                .cloned()
                .expect("seed draft exists");
            repository
                .save_ontology(&stored_ontology(&ontology)?)
                .await?;
            repository.save_draft(&stored_draft(&draft)?).await?;
        }
        Ok(runtime)
    }

    async fn load_control_plane(&self, snapshot: ControlPlaneSnapshot) -> Result<(), StorageError> {
        let active_versions = snapshot
            .ontologies
            .iter()
            .filter_map(|ontology| ontology.active_version_id.map(|id| (ontology.id, id)))
            .collect::<HashMap<_, _>>();
        self.ontologies.write().await.extend(
            snapshot
                .ontologies
                .into_iter()
                .map(proto_ontology)
                .map(|ontology| (ontology.id.clone(), ontology)),
        );
        self.drafts.write().await.extend(
            snapshot
                .drafts
                .into_iter()
                .map(proto_draft)
                .map(|value| value.map(|draft| (draft.id.clone(), draft)))
                .collect::<Result<HashMap<_, _>, _>>()?,
        );
        let mut versions = HashMap::<String, Vec<OntologyVersion>>::new();
        for stored in snapshot.versions {
            let active = active_versions.get(&stored.ontology_id) == Some(&stored.id);
            let version = proto_version(stored, active)?;
            versions
                .entry(version.ontology_id.clone())
                .or_default()
                .push(version);
        }
        self.versions.write().await.extend(versions);
        self.data_sources.write().await.extend(
            snapshot
                .data_sources
                .into_iter()
                .map(proto_data_source)
                .map(|source| (source.id.clone(), source)),
        );
        self.mappings.write().await.extend(
            snapshot
                .mappings
                .into_iter()
                .map(proto_mapping)
                .map(|mapping| (mapping.id.clone(), mapping)),
        );
        self.jobs.write().await.extend(
            snapshot
                .jobs
                .into_iter()
                .map(proto_job)
                .map(|value| value.map(|job| (job.id.clone(), job)))
                .collect::<Result<HashMap<_, _>, _>>()?,
        );
        Ok(())
    }

    async fn persist_ontology(&self, value: &Ontology) -> Result<(), Status> {
        if let Some(repository) = &self.control_plane {
            repository
                .save_ontology(&stored_ontology(value).map_err(storage_status)?)
                .await
                .map_err(storage_status)?;
        }
        Ok(())
    }

    async fn persist_draft(&self, value: &OntologyDraft) -> Result<(), Status> {
        if let Some(repository) = &self.control_plane {
            repository
                .save_draft(&stored_draft(value).map_err(storage_status)?)
                .await
                .map_err(storage_status)?;
        }
        Ok(())
    }

    async fn persist_version(
        &self,
        value: &OntologyVersion,
        workspace_id: &str,
    ) -> Result<(), Status> {
        if let Some(repository) = &self.control_plane {
            repository
                .save_version(&stored_version(value, workspace_id).map_err(storage_status)?)
                .await
                .map_err(storage_status)?;
        }
        Ok(())
    }

    async fn persist_data_source(&self, value: &DataSource) -> Result<(), Status> {
        if let Some(repository) = &self.control_plane {
            repository
                .save_data_source(
                    &stored_data_source(value, persistence_revision()).map_err(storage_status)?,
                )
                .await
                .map_err(storage_status)?;
        }
        Ok(())
    }

    async fn persist_mapping(&self, value: &OntologyDataMapping) -> Result<(), Status> {
        if let Some(repository) = &self.control_plane {
            repository
                .save_mapping(&stored_mapping(value).map_err(storage_status)?)
                .await
                .map_err(storage_status)?;
        }
        Ok(())
    }

    async fn persist_job(&self, value: &IngestionJob) -> Result<(), Status> {
        if let Some(repository) = &self.control_plane {
            repository
                .save_job(&stored_job(value, persistence_revision()).map_err(storage_status)?)
                .await
                .map_err(storage_status)?;
        }
        Ok(())
    }

    async fn ingestion_scope(
        &self,
        data_source_id: &str,
        mapping_id: &str,
        ontology_version_id: &str,
    ) -> Result<(DataSource, OntologyDataMapping, OntologyVersion), Status> {
        let source = self
            .data_sources
            .read()
            .await
            .get(data_source_id)
            .cloned()
            .ok_or_else(|| Status::not_found("data source not found"))?;
        let mapping = self
            .mappings
            .read()
            .await
            .get(mapping_id)
            .cloned()
            .ok_or_else(|| Status::not_found("ontology mapping not found"))?;
        if mapping.data_source_id != source.id {
            return Err(Status::invalid_argument(
                "mapping does not belong to the selected data source",
            ));
        }
        if mapping.workspace_id != source.workspace_id || source.workspace_id != dev_workspace_id()
        {
            return Err(Status::permission_denied("workspace is not accessible"));
        }
        let version = self
            .versions
            .read()
            .await
            .get(&mapping.ontology_id)
            .and_then(|versions| {
                versions
                    .iter()
                    .find(|version| version.id == ontology_version_id)
            })
            .cloned()
            .ok_or_else(|| {
                Status::failed_precondition(
                    "ontology version does not belong to the mapping ontology",
                )
            })?;
        Ok((source, mapping, version))
    }

    async fn ontology_version(
        &self,
        workspace_id: &str,
        version_id: &str,
    ) -> Result<OntologyVersion, Status> {
        if workspace_id != dev_workspace_id() {
            return Err(Status::permission_denied("workspace is not accessible"));
        }
        let ontologies = self.ontologies.read().await;
        let versions = self.versions.read().await;
        versions
            .iter()
            .find_map(|(ontology_id, candidates)| {
                let ontology = ontologies.get(ontology_id)?;
                (ontology.workspace_id == workspace_id)
                    .then(|| candidates.iter().find(|version| version.id == version_id))
                    .flatten()
                    .cloned()
            })
            .ok_or_else(|| Status::not_found("ontology version not found"))
    }

    async fn run_ingestion_job(
        &self,
        job_id: String,
        source: DataSource,
        mapping: OntologyDataMapping,
        version: OntologyVersion,
    ) {
        let running = {
            let mut jobs = self.jobs.write().await;
            jobs.get_mut(&job_id).map(|job| {
                job.state = IngestionState::Running as i32;
                job.clone()
            })
        };
        if let Some(running) = running
            && let Err(error) = self.persist_job(&running).await
        {
            tracing::error!(%error, %job_id, "failed to persist running ingestion job");
        }
        let result = self
            .ingest_uploaded_source(&source, &mapping, &version)
            .await;
        let completed = {
            let mut jobs = self.jobs.write().await;
            let Some(job) = jobs.get_mut(&job_id) else {
                return;
            };
            match result {
                Ok(mapped) => {
                    job.state = IngestionState::Succeeded as i32;
                    job.rows_read = mapped.rows_read;
                    job.rows_rejected = mapped.rows_rejected;
                    job.nodes_written = mapped.nodes.len() as u64;
                    job.edges_written = mapped.edges.len() as u64;
                }
                Err(error) => {
                    job.state = IngestionState::Failed as i32;
                    job.error = error;
                }
            }
            job.clone()
        };
        if let Err(error) = self.persist_job(&completed).await {
            tracing::error!(%error, %job_id, "failed to persist completed ingestion job");
        }
    }

    async fn resume_ingestion_jobs(&self) {
        let recoverable = self
            .jobs
            .read()
            .await
            .values()
            .filter(|job| {
                job.state == IngestionState::Queued as i32
                    || job.state == IngestionState::Running as i32
            })
            .cloned()
            .collect::<Vec<_>>();
        for job in recoverable {
            match self
                .ingestion_scope(
                    &job.data_source_id,
                    &job.ontology_mapping_id,
                    &job.ontology_version_id,
                )
                .await
            {
                Ok((source, mapping, version)) => {
                    let runtime = self.clone();
                    tokio::spawn(async move {
                        runtime
                            .run_ingestion_job(job.id, source, mapping, version)
                            .await;
                    });
                }
                Err(error) => self.fail_unrecoverable_job(job, error.message()).await,
            }
        }
    }

    async fn fail_unrecoverable_job(&self, mut job: IngestionJob, reason: &str) {
        job.state = IngestionState::Failed as i32;
        job.error = format!("ingestion job could not be resumed: {reason}");
        self.jobs.write().await.insert(job.id.clone(), job.clone());
        if let Err(error) = self.persist_job(&job).await {
            tracing::error!(%error, job_id = %job.id, "failed to persist unrecoverable ingestion job");
        }
    }

    async fn ingest_uploaded_source(
        &self,
        source: &DataSource,
        mapping: &OntologyDataMapping,
        version: &OntologyVersion,
    ) -> Result<context_hub_mapping::MappedGraphBatch, String> {
        let (configuration, content) = self.read_uploaded_source(source).await?;
        let document: MappingDocument = serde_json::from_str(&mapping.mapping_plan_json)
            .map_err(|error| format!("mapping document is invalid: {error}"))?;
        document
            .validate()
            .map_err(|error| format!("mapping document is invalid: {error}"))?;
        let definition: OntologyDefinition = serde_json::from_str(&version.definition_json)
            .map_err(|error| format!("stored ontology version is invalid: {error}"))?;
        for plan in document.plans() {
            validate_worker_plan(plan, &definition)?;
        }
        let mapped =
            execute_source_mapping_bundle(document.plans(), configuration.format, &content)
                .await
                .map_err(|error| error.to_string())?;
        self.write_mapped_graph(source, version, &mapped).await?;
        Ok(mapped)
    }

    async fn read_uploaded_source(
        &self,
        source: &DataSource,
    ) -> Result<(UploadSourceConfiguration, Vec<u8>), String> {
        let configuration: UploadSourceConfiguration =
            serde_json::from_str(&source.configuration_json)
                .map_err(|error| format!("source is not an uploaded object: {error}"))?;
        let content = self
            .source_store
            .get(&configuration.object_key)
            .await
            .map_err(|error| error.to_string())?;
        if content.len() as u64 != configuration.size_bytes {
            return Err("uploaded object size no longer matches its source definition".into());
        }
        if content.len() > MAX_UPLOAD_BYTES {
            return Err("uploaded object exceeds the 32 MiB development limit".into());
        }
        let checksum =
            Sha256::digest(&content)
                .iter()
                .fold(String::with_capacity(64), |mut output, byte| {
                    write!(&mut output, "{byte:02x}").expect("writing to a String cannot fail");
                    output
                });
        if checksum != configuration.sha256 {
            return Err("uploaded object checksum mismatch".into());
        }
        Ok((configuration, content))
    }

    async fn write_mapped_graph(
        &self,
        source: &DataSource,
        version: &OntologyVersion,
        mapped: &context_hub_mapping::MappedGraphBatch,
    ) -> Result<(), String> {
        let workspace_id = Uuid::parse_str(&source.workspace_id)
            .map_err(|error| format!("workspace id is invalid: {error}"))?;
        let ontology_version_id = Uuid::parse_str(&version.id)
            .map_err(|error| format!("ontology version id is invalid: {error}"))?;
        let data_source_id = Uuid::parse_str(&source.id)
            .map_err(|error| format!("data source id is invalid: {error}"))?;
        let write_version = u64::try_from(Utc::now().timestamp_micros())
            .map_err(|_| "system clock predates the Unix epoch".to_owned())?;
        let definition: OntologyDefinition = serde_json::from_str(&version.definition_json)
            .map_err(|error| format!("stored ontology is invalid: {error}"))?;
        let nodes = mapped
            .nodes
            .iter()
            .map(|node| {
                Ok(GraphNodeWrite {
                    workspace_id,
                    ontology_version_id,
                    object_type: node.object_type.clone(),
                    object_id: node.object_id.clone(),
                    source_id: data_source_id,
                    external_id: node.object_id.clone(),
                    property_indexes: property_indexes(
                        &node.object_type,
                        &node.properties_json,
                        &definition,
                    )?,
                    properties_json: node.properties_json.clone(),
                    version: write_version,
                })
            })
            .collect::<Result<Vec<_>, String>>()?;
        let edges = mapped
            .edges
            .iter()
            .map(|edge| GraphEdgeWrite {
                workspace_id,
                ontology_version_id,
                link_type: edge.link_type.clone(),
                edge_id: Uuid::new_v5(
                    &Uuid::NAMESPACE_URL,
                    format!(
                        "{}/{}/{}/{}/{}",
                        version.id, edge.link_type, edge.source_id, edge.target_id, source.id
                    )
                    .as_bytes(),
                )
                .to_string(),
                source_type: edge.source_object_type.clone(),
                source_id: edge.source_id.clone(),
                target_type: edge.target_object_type.clone(),
                target_id: edge.target_id.clone(),
                data_source_id,
                properties_json: edge.properties_json.clone(),
                version: write_version,
            })
            .collect::<Vec<_>>();
        for chunk in nodes.chunks(5_000) {
            self.graph
                .write_graph(chunk, &[])
                .await
                .map_err(|error| error.to_string())?;
        }
        for chunk in edges.chunks(20_000) {
            self.graph
                .write_graph(&[], chunk)
                .await
                .map_err(|error| error.to_string())?;
        }
        Ok(())
    }
}

#[tonic::async_trait]
impl WorkspaceService for Runtime {
    async fn get_workspace(
        &self,
        request: Request<GetWorkspaceRequest>,
    ) -> Result<Response<Workspace>, Status> {
        if request.into_inner().id != dev_workspace_id() {
            return Err(Status::not_found("workspace not found"));
        }
        Ok(Response::new(dev_workspace()))
    }
    async fn list_workspaces(
        &self,
        _: Request<()>,
    ) -> Result<Response<ListWorkspacesResponse>, Status> {
        Ok(Response::new(ListWorkspacesResponse {
            workspaces: vec![dev_workspace()],
        }))
    }
}

#[tonic::async_trait]
impl OntologyService for Runtime {
    async fn create(
        &self,
        request: Request<CreateOntologyRequest>,
    ) -> Result<Response<Ontology>, Status> {
        let request = request.into_inner();
        if request.workspace_id != dev_workspace_id() {
            return Err(Status::permission_denied("workspace is not accessible"));
        }
        validate_api_name(&request.slug)?;
        if request.name.trim().is_empty() {
            return Err(Status::invalid_argument("ontology name is required"));
        }
        if self.ontologies.read().await.values().any(|ontology| {
            ontology.workspace_id == request.workspace_id && ontology.slug == request.slug
        }) {
            return Err(Status::already_exists("ontology slug already exists"));
        }
        let id = Uuid::new_v4().to_string();
        let ontology = Ontology {
            id: id.clone(),
            workspace_id: request.workspace_id.clone(),
            name: request.name.clone(),
            slug: request.slug.clone(),
            active_version_id: String::new(),
            revision: 0,
        };
        let definition = OntologyDefinition {
            api_name: request.slug,
            display_name: request.name.clone(),
            description: None,
            object_types: vec![],
            link_types: vec![],
            interfaces: vec![],
            value_types: vec![],
            struct_types: vec![],
            shared_properties: vec![],
            functions: vec![],
        };
        let draft = OntologyDraft {
            id: id.clone(),
            workspace_id: request.workspace_id,
            name: request.name,
            slug: definition.api_name.clone(),
            revision: 0,
            definition_json: serde_json::to_string_pretty(&definition)
                .map_err(|error| Status::internal(error.to_string()))?,
            layout_json: "{}".into(),
            updated_at: Some(timestamp(Utc::now())),
        };
        self.persist_ontology(&ontology).await?;
        self.persist_draft(&draft).await?;
        self.drafts.write().await.insert(id.clone(), draft);
        self.ontologies.write().await.insert(id, ontology.clone());
        Ok(Response::new(ontology))
    }

    async fn list(
        &self,
        request: Request<ListOntologiesRequest>,
    ) -> Result<Response<ListOntologiesResponse>, Status> {
        let workspace_id = request.into_inner().workspace_id;
        let mut ontologies = self
            .ontologies
            .read()
            .await
            .values()
            .filter(|ontology| ontology.workspace_id == workspace_id)
            .cloned()
            .collect::<Vec<_>>();
        ontologies.sort_by(|left, right| left.name.cmp(&right.name));
        Ok(Response::new(ListOntologiesResponse { ontologies }))
    }

    async fn get_draft(
        &self,
        request: Request<GetOntologyDraftRequest>,
    ) -> Result<Response<OntologyDraft>, Status> {
        self.drafts
            .read()
            .await
            .get(&request.into_inner().id)
            .cloned()
            .map(Response::new)
            .ok_or_else(|| Status::not_found("ontology draft not found"))
    }

    async fn save_draft(
        &self,
        request: Request<SaveOntologyDraftRequest>,
    ) -> Result<Response<OntologyDraft>, Status> {
        let request = request.into_inner();
        let mut incoming = request
            .draft
            .ok_or_else(|| Status::invalid_argument("draft is required"))?;
        let definition: OntologyDefinition = serde_json::from_str(&incoming.definition_json)
            .map_err(|error| Status::invalid_argument(format!("invalid definition: {error}")))?;
        let issues = definition.validate();
        if !issues.is_empty() {
            return Err(Status::invalid_argument(
                serde_json::to_string(&issues).unwrap_or_else(|_| "ontology is invalid".into()),
            ));
        }
        let mut drafts = self.drafts.write().await;
        if let Some(current) = drafts.get(&incoming.id) {
            if current.revision != request.expected_revision {
                return Err(Status::aborted(format!(
                    "revision conflict: current revision is {}",
                    current.revision
                )));
            }
            incoming.revision = current.revision + 1;
        } else if request.expected_revision != 0 {
            return Err(Status::aborted("revision conflict: draft does not exist"));
        }
        incoming.updated_at = Some(timestamp(Utc::now()));
        drafts.insert(incoming.id.clone(), incoming.clone());
        drop(drafts);
        self.persist_draft(&incoming).await?;
        Ok(Response::new(incoming))
    }

    async fn validate(
        &self,
        request: Request<ValidateOntologyRequest>,
    ) -> Result<Response<ValidateOntologyResponse>, Status> {
        let definition: OntologyDefinition =
            serde_json::from_str(&request.into_inner().definition_json).map_err(|error| {
                Status::invalid_argument(format!("invalid definition JSON: {error}"))
            })?;
        let issues = definition
            .validate()
            .into_iter()
            .map(|issue| ValidationIssue {
                path: issue.path,
                code: issue.code,
                message: issue.message,
            })
            .collect::<Vec<_>>();
        Ok(Response::new(ValidateOntologyResponse {
            valid: issues.is_empty(),
            issues,
        }))
    }

    async fn publish(
        &self,
        request: Request<PublishOntologyRequest>,
    ) -> Result<Response<OntologyVersion>, Status> {
        let request = request.into_inner();
        let draft = self
            .drafts
            .read()
            .await
            .get(&request.ontology_id)
            .cloned()
            .ok_or_else(|| Status::not_found("ontology draft not found"))?;
        if draft.revision != request.expected_revision {
            return Err(Status::aborted(format!(
                "revision conflict: current revision is {}",
                draft.revision
            )));
        }
        let definition: OntologyDefinition = serde_json::from_str(&draft.definition_json)
            .map_err(|error| Status::internal(error.to_string()))?;
        let issues = definition.validate();
        if !issues.is_empty() {
            return Err(Status::failed_precondition(
                serde_json::to_string(&issues).unwrap_or_default(),
            ));
        }
        let checksum = Sha256::digest(draft.definition_json.as_bytes())
            .iter()
            .fold(String::with_capacity(64), |mut output, byte| {
                write!(&mut output, "{byte:02x}").expect("writing to a String cannot fail");
                output
            });
        let mut versions = self.versions.write().await;
        let ontology_versions = versions.entry(request.ontology_id.clone()).or_default();
        for version in ontology_versions.iter_mut() {
            version.active = false;
        }
        let ontology_id = request.ontology_id;
        let version = OntologyVersion {
            id: Uuid::new_v4().to_string(),
            ontology_id: ontology_id.clone(),
            version: ontology_versions.len() as u64 + 1,
            definition_json: draft.definition_json,
            checksum,
            active: true,
            published_at: Some(timestamp(Utc::now())),
        };
        ontology_versions.push(version.clone());
        drop(versions);
        let ontology = if let Some(ontology) = self.ontologies.write().await.get_mut(&ontology_id) {
            ontology.active_version_id.clone_from(&version.id);
            ontology.revision += 1;
            Some(ontology.clone())
        } else {
            None
        };
        self.persist_version(&version, &draft.workspace_id).await?;
        if let Some(ontology) = ontology {
            self.persist_ontology(&ontology).await?;
        }
        Ok(Response::new(version))
    }

    async fn list_versions(
        &self,
        request: Request<ListOntologyVersionsRequest>,
    ) -> Result<Response<ListOntologyVersionsResponse>, Status> {
        let versions = self
            .versions
            .read()
            .await
            .get(&request.into_inner().ontology_id)
            .cloned()
            .unwrap_or_default();
        Ok(Response::new(ListOntologyVersionsResponse { versions }))
    }
}

#[tonic::async_trait]
impl DataSourceService for Runtime {
    async fn save(
        &self,
        request: Request<SaveDataSourceRequest>,
    ) -> Result<Response<DataSource>, Status> {
        let mut source = request
            .into_inner()
            .data_source
            .ok_or_else(|| Status::invalid_argument("data_source is required"))?;
        if source.id.is_empty() {
            source.id = Uuid::new_v4().to_string();
        }
        if source.workspace_id.is_empty() {
            source.workspace_id = dev_workspace_id();
        }
        self.persist_data_source(&source).await?;
        self.data_sources
            .write()
            .await
            .insert(source.id.clone(), source.clone());
        Ok(Response::new(source))
    }

    async fn upload(
        &self,
        request: Request<UploadDataSourceRequest>,
    ) -> Result<Response<UploadDataSourceResponse>, Status> {
        let request = request.into_inner();
        if request.workspace_id != dev_workspace_id() {
            return Err(Status::permission_denied("workspace is not accessible"));
        }
        if request.name.trim().is_empty() || request.file_name.trim().is_empty() {
            return Err(Status::invalid_argument(
                "source name and file name are required",
            ));
        }
        if request.content.is_empty() || request.content.len() > MAX_UPLOAD_BYTES {
            return Err(Status::resource_exhausted(
                "upload must contain between 1 byte and 32 MiB",
            ));
        }
        let format = source_format(request.format)?;
        let id = Uuid::new_v4().to_string();
        let object_key = format!(
            "workspaces/{}/sources/{}/{}",
            request.workspace_id,
            id,
            safe_file_name(&request.file_name)
        );
        let checksum = Sha256::digest(&request.content).iter().fold(
            String::with_capacity(64),
            |mut output, byte| {
                write!(&mut output, "{byte:02x}").expect("writing to a String cannot fail");
                output
            },
        );
        let size_bytes = u64::try_from(request.content.len())
            .expect("the upload limit is substantially smaller than u64::MAX");
        self.source_store
            .put(&object_key, request.content)
            .await
            .map_err(storage_status)?;
        let configuration = UploadSourceConfiguration {
            object_key: object_key.clone(),
            file_name: request.file_name,
            format,
            size_bytes,
            sha256: checksum.clone(),
        };
        let source = DataSource {
            id,
            workspace_id: request.workspace_id,
            name: request.name,
            kind: 1,
            configuration_json: serde_json::to_string(&configuration)
                .map_err(|error| Status::internal(error.to_string()))?,
        };
        self.persist_data_source(&source).await?;
        self.data_sources
            .write()
            .await
            .insert(source.id.clone(), source.clone());
        Ok(Response::new(UploadDataSourceResponse {
            data_source: Some(source),
            object_key,
            size_bytes,
            sha256: checksum,
        }))
    }

    async fn get_upload(
        &self,
        request: Request<GetUploadDataSourceRequest>,
    ) -> Result<Response<GetUploadDataSourceResponse>, Status> {
        let id = request.into_inner().id;
        let source = self
            .data_sources
            .read()
            .await
            .get(&id)
            .cloned()
            .ok_or_else(|| Status::not_found("data source not found"))?;
        if source.workspace_id != dev_workspace_id() {
            return Err(Status::permission_denied("workspace is not accessible"));
        }
        if source.kind != 1 {
            return Err(Status::failed_precondition(
                "data source is not an uploaded file",
            ));
        }
        let (configuration, content) = self
            .read_uploaded_source(&source)
            .await
            .map_err(Status::failed_precondition)?;
        Ok(Response::new(GetUploadDataSourceResponse {
            data_source: Some(source),
            file_name: configuration.file_name,
            format: proto_source_format(configuration.format),
            content,
            sha256: configuration.sha256,
        }))
    }

    async fn list(
        &self,
        request: Request<ListDataSourcesRequest>,
    ) -> Result<Response<ListDataSourcesResponse>, Status> {
        let workspace_id = request.into_inner().workspace_id;
        let data_sources = self
            .data_sources
            .read()
            .await
            .values()
            .filter(|source| source.workspace_id == workspace_id)
            .cloned()
            .collect();
        Ok(Response::new(ListDataSourcesResponse { data_sources }))
    }

    async fn save_mapping(
        &self,
        request: Request<SaveOntologyDataMappingRequest>,
    ) -> Result<Response<OntologyDataMapping>, Status> {
        let mut mapping = request
            .into_inner()
            .mapping
            .ok_or_else(|| Status::invalid_argument("mapping is required"))?;
        if mapping.workspace_id != dev_workspace_id() {
            return Err(Status::permission_denied("workspace is not accessible"));
        }
        if !self
            .ontologies
            .read()
            .await
            .contains_key(&mapping.ontology_id)
        {
            return Err(Status::not_found("ontology not found"));
        }
        if !self
            .data_sources
            .read()
            .await
            .contains_key(&mapping.data_source_id)
        {
            return Err(Status::not_found("data source not found"));
        }
        serde_json::from_str::<serde_json::Value>(&mapping.mapping_plan_json)
            .map_err(|error| Status::invalid_argument(format!("invalid mapping plan: {error}")))?;
        if mapping.id.is_empty() {
            mapping.id = Uuid::new_v4().to_string();
        }
        let mut mappings = self.mappings.write().await;
        mapping.revision = mappings
            .get(&mapping.id)
            .map_or(1, |current| current.revision + 1);
        mappings.insert(mapping.id.clone(), mapping.clone());
        drop(mappings);
        self.persist_mapping(&mapping).await?;
        Ok(Response::new(mapping))
    }

    async fn list_mappings(
        &self,
        request: Request<ListOntologyDataMappingsRequest>,
    ) -> Result<Response<ListOntologyDataMappingsResponse>, Status> {
        let request = request.into_inner();
        let mappings = self
            .mappings
            .read()
            .await
            .values()
            .filter(|mapping| {
                mapping.workspace_id == request.workspace_id
                    && mapping.ontology_id == request.ontology_id
            })
            .cloned()
            .collect();
        Ok(Response::new(ListOntologyDataMappingsResponse { mappings }))
    }
}

#[tonic::async_trait]
impl IngestionService for Runtime {
    async fn start(
        &self,
        request: Request<StartIngestionRequest>,
    ) -> Result<Response<IngestionJob>, Status> {
        let request = request.into_inner();
        let (source, mapping, version) = self
            .ingestion_scope(
                &request.data_source_id,
                &request.ontology_mapping_id,
                &request.ontology_version_id,
            )
            .await?;
        let job = IngestionJob {
            id: Uuid::new_v4().to_string(),
            data_source_id: request.data_source_id,
            state: IngestionState::Queued as i32,
            rows_read: 0,
            nodes_written: 0,
            edges_written: 0,
            rows_rejected: 0,
            error: String::new(),
            ontology_mapping_id: request.ontology_mapping_id,
            ontology_version_id: request.ontology_version_id,
            workspace_id: source.workspace_id.clone(),
        };
        self.persist_job(&job).await?;
        self.jobs.write().await.insert(job.id.clone(), job.clone());
        let runtime = self.clone();
        let job_id = job.id.clone();
        tokio::spawn(async move {
            runtime
                .run_ingestion_job(job_id, source, mapping, version)
                .await;
        });
        Ok(Response::new(job))
    }

    async fn import_graph(
        &self,
        request: Request<ImportGraphRequest>,
    ) -> Result<Response<IngestionJob>, Status> {
        let request = request.into_inner();
        if request.nodes.len() > 5_000 || request.edges.len() > 20_000 {
            return Err(Status::resource_exhausted(
                "one graph batch supports at most 5000 nodes and 20000 edges",
            ));
        }
        let (source, mapping, version) = self
            .ingestion_scope(
                &request.data_source_id,
                &request.ontology_mapping_id,
                &request.ontology_version_id,
            )
            .await?;
        let definition: OntologyDefinition = serde_json::from_str(&version.definition_json)
            .map_err(|error| Status::internal(format!("stored ontology is invalid: {error}")))?;
        let workspace_id = parse_uuid(&source.workspace_id, "workspace_id")?;
        let ontology_version_id = parse_uuid(&version.id, "ontology_version_id")?;
        let data_source_id = parse_uuid(&source.id, "data_source_id")?;
        let write_version = u64::try_from(Utc::now().timestamp_micros())
            .map_err(|_| Status::internal("system clock predates the Unix epoch"))?;

        let mut nodes = Vec::with_capacity(request.nodes.len());
        for node in &request.nodes {
            validate_graph_node(node, &definition)?;
            nodes.push(GraphNodeWrite {
                workspace_id,
                ontology_version_id,
                object_type: node.object_type.clone(),
                object_id: node.id.clone(),
                source_id: data_source_id,
                external_id: node.id.clone(),
                property_indexes: property_indexes(
                    &node.object_type,
                    &node.properties_json,
                    &definition,
                )
                .map_err(Status::invalid_argument)?,
                properties_json: node.properties_json.clone(),
                version: write_version,
            });
        }
        let mut edges = Vec::with_capacity(request.edges.len());
        for edge in &request.edges {
            let link = validate_graph_edge(edge, &definition)?;
            let edge_id = if edge.id.is_empty() {
                Uuid::new_v5(
                    &Uuid::NAMESPACE_URL,
                    format!(
                        "{}/{}/{}/{}/{}",
                        version.id, edge.link_type, edge.source_id, edge.target_id, source.id
                    )
                    .as_bytes(),
                )
                .to_string()
            } else {
                edge.id.clone()
            };
            edges.push(GraphEdgeWrite {
                workspace_id,
                ontology_version_id,
                link_type: edge.link_type.clone(),
                edge_id,
                source_type: link.source_type.clone(),
                source_id: edge.source_id.clone(),
                target_type: link.target_type.clone(),
                target_id: edge.target_id.clone(),
                data_source_id,
                properties_json: edge.properties_json.clone(),
                version: write_version,
            });
        }

        let mut job = IngestionJob {
            id: Uuid::new_v4().to_string(),
            data_source_id: source.id,
            state: IngestionState::Running as i32,
            rows_read: request.nodes.len() as u64,
            nodes_written: 0,
            edges_written: 0,
            rows_rejected: 0,
            error: String::new(),
            ontology_mapping_id: mapping.id,
            ontology_version_id: version.id,
            workspace_id: source.workspace_id,
        };
        self.persist_job(&job).await?;
        self.jobs.write().await.insert(job.id.clone(), job.clone());
        match self.graph.write_graph(&nodes, &edges).await {
            Ok(()) => {
                job.state = IngestionState::Succeeded as i32;
                job.nodes_written = nodes.len() as u64;
                job.edges_written = edges.len() as u64;
            }
            Err(error) => {
                job.state = IngestionState::Failed as i32;
                job.error = error.to_string();
            }
        }
        self.persist_job(&job).await?;
        self.jobs.write().await.insert(job.id.clone(), job.clone());
        Ok(Response::new(job))
    }

    async fn get_job(
        &self,
        request: Request<GetIngestionJobRequest>,
    ) -> Result<Response<IngestionJob>, Status> {
        self.jobs
            .read()
            .await
            .get(&request.into_inner().id)
            .cloned()
            .map(Response::new)
            .ok_or_else(|| Status::not_found("ingestion job not found"))
    }
}

#[tonic::async_trait]
impl GraphService for Runtime {
    async fn query(
        &self,
        request: Request<GraphQuery>,
    ) -> Result<Response<QueryGraphResponse>, Status> {
        let query = request.into_inner();
        let version = self
            .ontology_version(&query.workspace_id, &query.ontology_version_id)
            .await?;
        let definition: OntologyDefinition = serde_json::from_str(&version.definition_json)
            .map_err(|error| Status::internal(format!("stored ontology is invalid: {error}")))?;
        validate_graph_query(&query, &definition)?;
        let limit = if query.limit == 0 { 500 } else { query.limit };
        let storage_query = graph_query_to_storage(&query, limit)?;
        let mut nodes_by_key = HashMap::new();
        let depth_count = storage_query.traversal.len() + 1;
        let mut cursor = decode_graph_cursor(&query.cursor, depth_count)?;
        let per_depth_limit = (limit / u32::try_from(depth_count).unwrap_or(1)).max(1);
        for depth in 0..=storage_query.traversal.len() {
            if cursor.positions[depth].done {
                continue;
            }
            let remaining = limit as usize - nodes_by_key.len();
            if remaining == 0 {
                break;
            }
            let mut prefix = storage_query.clone();
            prefix.traversal.truncate(depth);
            prefix
                .after_object_id
                .clone_from(&cursor.positions[depth].after_object_id);
            prefix.limit = per_depth_limit.min(
                u32::try_from(remaining)
                    .expect("remaining node budget never exceeds the validated u32 query limit"),
            );
            let prefix_nodes = self
                .graph
                .query_nodes(&prefix)
                .await
                .map_err(storage_status)?;
            let has_more = prefix_nodes.len()
                == usize::try_from(prefix.limit)
                    .expect("validated graph query limits fit into usize");
            if let Some(last) = prefix_nodes.last() {
                cursor.positions[depth].after_object_id = Some(last.object_id.clone());
                cursor.positions[depth].done = !has_more;
            } else {
                cursor.positions[depth].done = true;
            }
            for node in prefix_nodes {
                nodes_by_key.insert((node.object_type.clone(), node.object_id.clone()), node);
            }
        }
        let mut nodes = nodes_by_key.into_values().collect::<Vec<_>>();
        nodes.sort_by(|left, right| {
            (&left.object_type, &left.object_id).cmp(&(&right.object_type, &right.object_id))
        });
        let object_ids = nodes
            .iter()
            .map(|node| node.object_id.clone())
            .collect::<Vec<_>>();
        let edges = self
            .graph
            .edges_between(
                storage_query.workspace_id,
                storage_query.ontology_version_id,
                &object_ids,
                20_000,
            )
            .await
            .map_err(storage_status)?;
        let next_cursor = encode_graph_cursor(&cursor)?;
        let truncated = !next_cursor.is_empty();
        Ok(Response::new(QueryGraphResponse {
            nodes: nodes
                .into_iter()
                .map(|node| GraphNode {
                    id: node.object_id,
                    object_type: node.object_type,
                    properties_json: node.properties_json,
                })
                .collect(),
            edges: edges
                .into_iter()
                .map(|edge| GraphEdge {
                    id: edge.edge_id,
                    link_type: edge.link_type,
                    source_id: edge.source_id,
                    target_id: edge.target_id,
                    properties_json: edge.properties_json,
                })
                .collect(),
            next_cursor,
            truncated,
        }))
    }
    async fn get_object(
        &self,
        request: Request<GetObjectRequest>,
    ) -> Result<Response<GraphNode>, Status> {
        let request = request.into_inner();
        let version = self
            .ontology_version(&request.workspace_id, &request.ontology_version_id)
            .await?;
        let definition: OntologyDefinition = serde_json::from_str(&version.definition_json)
            .map_err(|error| Status::internal(format!("stored ontology is invalid: {error}")))?;
        if !definition
            .object_types
            .iter()
            .any(|object_type| object_type.api_name == request.object_type)
        {
            return Err(Status::invalid_argument(
                "object type does not exist in the ontology version",
            ));
        }
        let node = self
            .graph
            .get_node(
                parse_uuid(&request.workspace_id, "workspace_id")?,
                parse_uuid(&request.ontology_version_id, "ontology_version_id")?,
                &request.object_type,
                &request.id,
            )
            .await
            .map_err(storage_status)?;
        Ok(Response::new(GraphNode {
            id: node.object_id,
            object_type: node.object_type,
            properties_json: node.properties_json,
        }))
    }
}

fn validate_graph_node(node: &GraphNode, definition: &OntologyDefinition) -> Result<(), Status> {
    if node.id.is_empty() || node.id.len() > 512 {
        return Err(Status::invalid_argument(
            "graph node id must contain between 1 and 512 characters",
        ));
    }
    validate_api_name(&node.object_type)?;
    if !definition
        .object_types
        .iter()
        .any(|object_type| object_type.api_name == node.object_type)
    {
        return Err(Status::invalid_argument(format!(
            "object type '{}' does not exist in the ontology version",
            node.object_type
        )));
    }
    validate_properties_json(&node.properties_json)
}

fn validate_worker_plan(plan: &MappingPlan, definition: &OntologyDefinition) -> Result<(), String> {
    let object_type = definition
        .object_types
        .iter()
        .find(|object_type| object_type.api_name == plan.object_type)
        .ok_or_else(|| {
            format!(
                "mapping object type '{}' does not exist in the ontology version",
                plan.object_type
            )
        })?;
    for field in &plan.fields {
        if !object_type
            .properties
            .iter()
            .any(|property| property.api_name == field.target)
        {
            return Err(format!(
                "mapping target '{}.{}' does not exist in the ontology version",
                plan.object_type, field.target
            ));
        }
    }
    for link in &plan.links {
        let definition = definition
            .link_types
            .iter()
            .find(|candidate| candidate.api_name == link.link_type)
            .ok_or_else(|| {
                format!(
                    "mapping link type '{}' does not exist in the ontology version",
                    link.link_type
                )
            })?;
        if definition.source_type != plan.object_type
            || definition.target_type != link.target_object_type
        {
            return Err(format!(
                "mapping link '{}' does not connect '{}' to '{}'",
                link.link_type, plan.object_type, link.target_object_type
            ));
        }
    }
    Ok(())
}

fn validate_graph_edge<'a>(
    edge: &GraphEdge,
    definition: &'a OntologyDefinition,
) -> Result<&'a context_hub_domain::LinkTypeDefinition, Status> {
    if edge.source_id.is_empty() || edge.target_id.is_empty() {
        return Err(Status::invalid_argument(
            "graph edge source_id and target_id are required",
        ));
    }
    let link = definition
        .link_types
        .iter()
        .find(|link| link.api_name == edge.link_type)
        .ok_or_else(|| {
            Status::invalid_argument(format!(
                "link type '{}' does not exist in the ontology version",
                edge.link_type
            ))
        })?;
    validate_properties_json(&edge.properties_json)?;
    Ok(link)
}

fn validate_properties_json(value: &str) -> Result<(), Status> {
    let value: serde_json::Value = serde_json::from_str(value)
        .map_err(|error| Status::invalid_argument(format!("invalid properties JSON: {error}")))?;
    if value.is_object() {
        Ok(())
    } else {
        Err(Status::invalid_argument(
            "graph properties must be a JSON object",
        ))
    }
}

fn property_indexes(
    object_type: &str,
    properties_json: &str,
    definition: &OntologyDefinition,
) -> Result<Vec<PropertyIndexWrite>, String> {
    let object_definition = definition
        .object_types
        .iter()
        .find(|candidate| candidate.api_name == object_type)
        .ok_or_else(|| format!("object type '{object_type}' does not exist in the ontology"))?;
    let properties: serde_json::Map<String, serde_json::Value> =
        serde_json::from_str(properties_json)
            .map_err(|error| format!("invalid properties JSON: {error}"))?;
    let mut indexes = Vec::new();
    for property in object_definition
        .properties
        .iter()
        .filter(|property| property.indexed)
    {
        let Some(value) = properties.get(&property.api_name) else {
            continue;
        };
        let values = if property.value_type.list {
            value.as_array().ok_or_else(|| {
                format!(
                    "indexed property '{}.{}' must be a list",
                    object_type, property.api_name
                )
            })?
        } else {
            std::slice::from_ref(value)
        };
        for value in values.iter().filter(|value| !value.is_null()) {
            let index_value = match &property.value_type.scalar {
                ScalarType::String | ScalarType::Uuid | ScalarType::Enum { .. } => {
                    PropertyIndexValue::String(
                        value
                            .as_str()
                            .ok_or_else(|| indexed_type_error(object_type, property))?
                            .to_owned(),
                    )
                }
                ScalarType::Int64 | ScalarType::Float64 | ScalarType::Decimal => {
                    let number = value.as_number().map(ToString::to_string).or_else(|| {
                        matches!(property.value_type.scalar, ScalarType::Decimal)
                            .then(|| value.as_str().map(str::to_owned))
                            .flatten()
                    });
                    PropertyIndexValue::Number(
                        number.ok_or_else(|| indexed_type_error(object_type, property))?,
                    )
                }
                ScalarType::Boolean => PropertyIndexValue::Boolean(
                    value
                        .as_bool()
                        .ok_or_else(|| indexed_type_error(object_type, property))?,
                ),
                ScalarType::Date => {
                    let date = NaiveDate::parse_from_str(
                        value
                            .as_str()
                            .ok_or_else(|| indexed_type_error(object_type, property))?,
                        "%Y-%m-%d",
                    )
                    .map_err(|_| indexed_type_error(object_type, property))?;
                    PropertyIndexValue::Timestamp(format!("{date} 00:00:00.000000"))
                }
                ScalarType::Timestamp => {
                    let timestamp = DateTime::parse_from_rfc3339(
                        value
                            .as_str()
                            .ok_or_else(|| indexed_type_error(object_type, property))?,
                    )
                    .map_err(|_| indexed_type_error(object_type, property))?;
                    PropertyIndexValue::Timestamp(
                        timestamp
                            .with_timezone(&Utc)
                            .format("%Y-%m-%d %H:%M:%S%.6f")
                            .to_string(),
                    )
                }
                ScalarType::Json => continue,
            };
            indexes.push(PropertyIndexWrite {
                property: property.api_name.clone(),
                value: index_value,
            });
        }
    }
    Ok(indexes)
}

fn indexed_type_error(object_type: &str, property: &PropertyDefinition) -> String {
    format!(
        "indexed property '{}.{}' does not match its ontology type",
        object_type, property.api_name
    )
}

fn validate_graph_query(query: &GraphQuery, definition: &OntologyDefinition) -> Result<(), Status> {
    if query.cursor.len() > 16_384 {
        return Err(Status::invalid_argument("graph query cursor is too large"));
    }
    if query.traversal.len() > 6 {
        return Err(Status::invalid_argument(
            "traversal depth exceeds the maximum of 6",
        ));
    }
    let root = definition
        .object_types
        .iter()
        .find(|object_type| object_type.api_name == query.root_type)
        .ok_or_else(|| Status::invalid_argument("root type does not exist in the ontology"))?;
    for filter in &query.filters {
        if !root
            .properties
            .iter()
            .any(|property| property.api_name == filter.property)
        {
            return Err(Status::invalid_argument(format!(
                "property '{}.{}' does not exist",
                root.api_name, filter.property
            )));
        }
    }
    let mut current_type = query.root_type.as_str();
    for step in &query.traversal {
        let link = definition
            .link_types
            .iter()
            .find(|link| link.api_name == step.link_type)
            .ok_or_else(|| Status::invalid_argument("traversal link type does not exist"))?;
        let (expected_source, expected_target) = if step.reverse {
            (&link.target_type, &link.source_type)
        } else {
            (&link.source_type, &link.target_type)
        };
        if expected_source != current_type || expected_target != &step.target_type {
            return Err(Status::invalid_argument(format!(
                "traversal '{}' does not connect '{}' to '{}'",
                step.link_type, current_type, step.target_type
            )));
        }
        current_type = &step.target_type;
    }
    let limit = if query.limit == 0 { 500 } else { query.limit };
    if limit > 5_000 {
        return Err(Status::invalid_argument(
            "node limit exceeds the maximum of 5000",
        ));
    }
    Ok(())
}

fn decode_graph_cursor(value: &str, depth_count: usize) -> Result<GraphCursor, Status> {
    if value.is_empty() {
        return Ok(GraphCursor {
            version: 1,
            positions: vec![GraphCursorPosition::default(); depth_count],
        });
    }
    let cursor: GraphCursor = serde_json::from_str(value)
        .map_err(|_| Status::invalid_argument("graph query cursor is invalid"))?;
    if cursor.version != 1 || cursor.positions.len() != depth_count {
        return Err(Status::invalid_argument(
            "graph query cursor does not match the traversal",
        ));
    }
    Ok(cursor)
}

fn encode_graph_cursor(cursor: &GraphCursor) -> Result<String, Status> {
    if cursor.positions.iter().all(|position| position.done) {
        return Ok(String::new());
    }
    serde_json::to_string(cursor)
        .map_err(|error| Status::internal(format!("graph cursor serialization failed: {error}")))
}

fn graph_query_to_storage(query: &GraphQuery, limit: u32) -> Result<StorageGraphQuery, Status> {
    let filters = query
        .filters
        .iter()
        .map(|filter| {
            if filter.values.len() != 1 {
                return Err(Status::invalid_argument(
                    "each graph filter currently requires exactly one value",
                ));
            }
            let operator = match filter.operator {
                1 => StorageFilterOperator::Equal,
                2 => StorageFilterOperator::NotEqual,
                3 => StorageFilterOperator::Contains,
                4 => StorageFilterOperator::GreaterThan,
                5 => StorageFilterOperator::LessThan,
                _ => {
                    return Err(Status::invalid_argument(
                        "unsupported or unspecified filter operator",
                    ));
                }
            };
            Ok(StorageGraphFilter {
                property: filter.property.clone(),
                operator,
                value: filter.values[0].clone(),
            })
        })
        .collect::<Result<Vec<_>, Status>>()?;
    Ok(StorageGraphQuery {
        workspace_id: parse_uuid(&query.workspace_id, "workspace_id")?,
        ontology_version_id: parse_uuid(&query.ontology_version_id, "ontology_version_id")?,
        root_type: query.root_type.clone(),
        filters,
        traversal: query
            .traversal
            .iter()
            .map(|step| StorageTraversalStep {
                link_type: step.link_type.clone(),
                target_type: step.target_type.clone(),
                reverse: step.reverse,
            })
            .collect(),
        after_object_id: None,
        limit,
    })
}

fn parse_uuid(value: &str, field: &str) -> Result<Uuid, Status> {
    Uuid::parse_str(value)
        .map_err(|error| Status::invalid_argument(format!("invalid {field}: {error}")))
}

fn source_format(value: i32) -> Result<SourceFormat, Status> {
    match value {
        1 => Ok(SourceFormat::Json),
        2 => Ok(SourceFormat::Ndjson),
        3 => Ok(SourceFormat::Csv),
        _ => Err(Status::invalid_argument(
            "source file format must be JSON, NDJSON, or CSV",
        )),
    }
}

const fn proto_source_format(value: SourceFormat) -> i32 {
    match value {
        SourceFormat::Json => 1,
        SourceFormat::Ndjson => 2,
        SourceFormat::Csv => 3,
    }
}

fn safe_file_name(value: &str) -> String {
    let value = value
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or("upload")
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '.' | '-' | '_') {
                character
            } else {
                '_'
            }
        })
        .take(128)
        .collect::<String>();
    if value.is_empty() {
        "upload".into()
    } else {
        value
    }
}

fn storage_status(error: StorageError) -> Status {
    match error {
        StorageError::NotFound => Status::not_found("graph object not found"),
        StorageError::InvalidRecord(message) => Status::invalid_argument(message),
        other => Status::internal(other.to_string()),
    }
}

fn dev_workspace_id() -> String {
    "00000000-0000-0000-0000-000000000001".into()
}
fn dev_workspace() -> Workspace {
    Workspace {
        id: dev_workspace_id(),
        name: "Development".into(),
        slug: "development".into(),
    }
}
fn validate_api_name(value: &str) -> Result<(), Status> {
    let valid = !value.is_empty()
        && value.len() <= 64
        && value
            .chars()
            .next()
            .is_some_and(|character| character.is_ascii_lowercase())
        && value.chars().all(|character| {
            character.is_ascii_lowercase() || character.is_ascii_digit() || character == '_'
        });
    if valid {
        Ok(())
    } else {
        Err(Status::invalid_argument(
            "slug must match [a-z][a-z0-9_]{0,63}",
        ))
    }
}
fn timestamp(value: chrono::DateTime<Utc>) -> Timestamp {
    Timestamp {
        seconds: value.timestamp(),
        nanos: i32::try_from(value.timestamp_subsec_nanos())
            .expect("nanoseconds within a timestamp are below one billion"),
    }
}

fn persistence_revision() -> u64 {
    u64::try_from(Utc::now().timestamp_micros()).unwrap_or_default()
}

#[derive(Debug, Serialize, Deserialize)]
struct PersistedJobStats {
    rows_read: u64,
    nodes_written: u64,
    edges_written: u64,
    rows_rejected: u64,
}

fn parse_uuid_storage(value: &str) -> Result<Uuid, StorageError> {
    Uuid::parse_str(value).map_err(|error| StorageError::InvalidRecord(error.to_string()))
}

fn timestamp_micros(value: Option<&Timestamp>) -> Result<i64, StorageError> {
    let value = value.ok_or_else(|| StorageError::InvalidRecord("timestamp is missing".into()))?;
    value
        .seconds
        .checked_mul(1_000_000)
        .and_then(|seconds| seconds.checked_add(i64::from(value.nanos) / 1_000))
        .ok_or_else(|| {
            StorageError::InvalidRecord("timestamp is outside the supported range".into())
        })
}

fn proto_timestamp(micros: i64) -> Result<Timestamp, StorageError> {
    let value = chrono::DateTime::from_timestamp_micros(micros).ok_or_else(|| {
        StorageError::InvalidRecord("timestamp is outside the supported range".into())
    })?;
    Ok(timestamp(value))
}

fn stored_ontology(value: &Ontology) -> Result<StoredOntology, StorageError> {
    Ok(StoredOntology {
        id: parse_uuid_storage(&value.id)?,
        workspace_id: parse_uuid_storage(&value.workspace_id)?,
        name: value.name.clone(),
        slug: value.slug.clone(),
        active_version_id: (!value.active_version_id.is_empty())
            .then(|| parse_uuid_storage(&value.active_version_id))
            .transpose()?,
        revision: value.revision,
    })
}

fn proto_ontology(value: StoredOntology) -> Ontology {
    Ontology {
        id: value.id.to_string(),
        workspace_id: value.workspace_id.to_string(),
        name: value.name,
        slug: value.slug,
        active_version_id: value
            .active_version_id
            .map_or_else(String::new, |id| id.to_string()),
        revision: value.revision,
    }
}

fn stored_draft(value: &OntologyDraft) -> Result<StoredOntologyDraft, StorageError> {
    Ok(StoredOntologyDraft {
        id: parse_uuid_storage(&value.id)?,
        workspace_id: parse_uuid_storage(&value.workspace_id)?,
        revision: value.revision,
        definition_json: value.definition_json.clone(),
        layout_json: value.layout_json.clone(),
        updated_at_micros: timestamp_micros(value.updated_at.as_ref())?,
    })
}

fn proto_draft(value: StoredOntologyDraft) -> Result<OntologyDraft, StorageError> {
    let definition: OntologyDefinition = serde_json::from_str(&value.definition_json)?;
    Ok(OntologyDraft {
        id: value.id.to_string(),
        workspace_id: value.workspace_id.to_string(),
        name: definition.display_name,
        slug: definition.api_name,
        revision: value.revision,
        definition_json: value.definition_json,
        layout_json: value.layout_json,
        updated_at: Some(proto_timestamp(value.updated_at_micros)?),
    })
}

fn stored_version(
    value: &OntologyVersion,
    workspace_id: &str,
) -> Result<StoredOntologyVersion, StorageError> {
    Ok(StoredOntologyVersion {
        id: parse_uuid_storage(&value.id)?,
        workspace_id: parse_uuid_storage(workspace_id)?,
        ontology_id: parse_uuid_storage(&value.ontology_id)?,
        version: value.version,
        definition_json: value.definition_json.clone(),
        checksum: value.checksum.clone(),
        published_at_micros: timestamp_micros(value.published_at.as_ref())?,
    })
}

fn proto_version(
    value: StoredOntologyVersion,
    active: bool,
) -> Result<OntologyVersion, StorageError> {
    Ok(OntologyVersion {
        id: value.id.to_string(),
        ontology_id: value.ontology_id.to_string(),
        version: value.version,
        definition_json: value.definition_json,
        checksum: value.checksum,
        active,
        published_at: Some(proto_timestamp(value.published_at_micros)?),
    })
}

fn stored_data_source(value: &DataSource, revision: u64) -> Result<StoredDataSource, StorageError> {
    Ok(StoredDataSource {
        id: parse_uuid_storage(&value.id)?,
        workspace_id: parse_uuid_storage(&value.workspace_id)?,
        name: value.name.clone(),
        kind: value.kind,
        configuration_json: value.configuration_json.clone(),
        revision,
    })
}

fn proto_data_source(value: StoredDataSource) -> DataSource {
    DataSource {
        id: value.id.to_string(),
        workspace_id: value.workspace_id.to_string(),
        name: value.name,
        kind: value.kind,
        configuration_json: value.configuration_json,
    }
}

fn stored_mapping(value: &OntologyDataMapping) -> Result<StoredOntologyMapping, StorageError> {
    Ok(StoredOntologyMapping {
        id: parse_uuid_storage(&value.id)?,
        workspace_id: parse_uuid_storage(&value.workspace_id)?,
        ontology_id: parse_uuid_storage(&value.ontology_id)?,
        data_source_id: parse_uuid_storage(&value.data_source_id)?,
        name: value.name.clone(),
        mapping_plan_json: value.mapping_plan_json.clone(),
        revision: value.revision,
    })
}

fn proto_mapping(value: StoredOntologyMapping) -> OntologyDataMapping {
    OntologyDataMapping {
        id: value.id.to_string(),
        workspace_id: value.workspace_id.to_string(),
        ontology_id: value.ontology_id.to_string(),
        data_source_id: value.data_source_id.to_string(),
        name: value.name,
        mapping_plan_json: value.mapping_plan_json,
        revision: value.revision,
    }
}

fn stored_job(value: &IngestionJob, revision: u64) -> Result<StoredIngestionJob, StorageError> {
    Ok(StoredIngestionJob {
        id: parse_uuid_storage(&value.id)?,
        workspace_id: parse_uuid_storage(&value.workspace_id)?,
        data_source_id: parse_uuid_storage(&value.data_source_id)?,
        ontology_mapping_id: parse_uuid_storage(&value.ontology_mapping_id)?,
        ontology_version_id: parse_uuid_storage(&value.ontology_version_id)?,
        state: value.state,
        stats_json: serde_json::to_string(&PersistedJobStats {
            rows_read: value.rows_read,
            nodes_written: value.nodes_written,
            edges_written: value.edges_written,
            rows_rejected: value.rows_rejected,
        })?,
        error: value.error.clone(),
        revision,
    })
}

fn proto_job(value: StoredIngestionJob) -> Result<IngestionJob, StorageError> {
    let stats: PersistedJobStats = serde_json::from_str(&value.stats_json)?;
    Ok(IngestionJob {
        id: value.id.to_string(),
        data_source_id: value.data_source_id.to_string(),
        state: value.state,
        rows_read: stats.rows_read,
        nodes_written: stats.nodes_written,
        edges_written: stats.edges_written,
        rows_rejected: stats.rows_rejected,
        error: value.error,
        ontology_mapping_id: value.ontology_mapping_id.to_string(),
        ontology_version_id: value.ontology_version_id.to_string(),
        workspace_id: value.workspace_id.to_string(),
    })
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("context_hub=debug")),
        )
        .init();
    let address: SocketAddr = std::env::var("GRPC_ADDR")
        .unwrap_or_else(|_| "0.0.0.0:50051".into())
        .parse()?;
    let clickhouse = clickhouse::Client::default()
        .with_url(
            std::env::var("CLICKHOUSE_URL").unwrap_or_else(|_| "http://localhost:8123".into()),
        )
        .with_database(
            std::env::var("CLICKHOUSE_DATABASE").unwrap_or_else(|_| "context_hub".into()),
        )
        .with_user(std::env::var("CLICKHOUSE_USER").unwrap_or_else(|_| "context_hub".into()))
        .with_password(
            std::env::var("CLICKHOUSE_PASSWORD").unwrap_or_else(|_| "context_hub".into()),
        )
        .with_setting("input_format_binary_read_json_as_string", "1")
        .with_setting("output_format_binary_write_json_as_string", "1");
    let source_store = ObjectStoreSourceRepository::s3(
        &std::env::var("S3_ENDPOINT").unwrap_or_else(|_| "http://localhost:9002".into()),
        &std::env::var("S3_BUCKET").unwrap_or_else(|_| "context-hub".into()),
        &std::env::var("S3_ACCESS_KEY").unwrap_or_else(|_| "context_hub".into()),
        &std::env::var("S3_SECRET_KEY").unwrap_or_else(|_| "context_hub_dev_secret".into()),
    )?;
    let runtime = Runtime::seeded_with_stores(
        Arc::new(ClickHouseGraphRepository::new(clickhouse.clone())),
        Arc::new(source_store),
        Some(Arc::new(ClickHouseControlPlaneRepository::new(clickhouse))),
    )
    .await?;
    tracing::info!(%address, "starting ContextHub gRPC and gRPC-Web server");
    Server::builder()
        .accept_http1(true)
        .layer(
            CorsLayer::new()
                .allow_origin(Any)
                .allow_methods(Any)
                .allow_headers(Any)
                .expose_headers(Any),
        )
        .layer(tonic_web::GrpcWebLayer::new())
        .add_service(WorkspaceServiceServer::new(runtime.clone()))
        .add_service(OntologyServiceServer::new(runtime.clone()))
        .add_service(
            DataSourceServiceServer::new(runtime.clone())
                .max_decoding_message_size(34 * 1024 * 1024)
                .max_encoding_message_size(34 * 1024 * 1024),
        )
        .add_service(
            IngestionServiceServer::new(runtime.clone())
                .max_decoding_message_size(64 * 1024 * 1024),
        )
        .add_service(GraphServiceServer::new(runtime))
        .serve(address)
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn derives_typed_property_indexes_from_the_ontology() {
        let runtime = Runtime::seeded().await;
        let draft = runtime
            .drafts
            .read()
            .await
            .values()
            .next()
            .cloned()
            .expect("seed draft exists");
        let mut definition: OntologyDefinition =
            serde_json::from_str(&draft.definition_json).expect("seed definition is valid");
        let properties = &mut definition.object_types[0].properties;
        for (api_name, scalar, list) in [
            ("score", ScalarType::Float64, false),
            ("enabled", ScalarType::Boolean, false),
            ("observed_at", ScalarType::Timestamp, false),
            ("tags", ScalarType::String, true),
        ] {
            properties.push(PropertyDefinition {
                api_name: api_name.into(),
                display_name: api_name.into(),
                value_type: ValueType { scalar, list },
                required: false,
                unique: false,
                identity: false,
                indexed: true,
                description: None,
            });
        }

        let indexes = property_indexes(
            "service",
            r#"{"id":"billing","score":4.5,"enabled":true,"observed_at":"2026-07-21T10:11:12Z","tags":["critical","public"]}"#,
            &definition,
        )
        .expect("typed indexes can be derived");

        assert_eq!(indexes.len(), 6);
        assert!(indexes.contains(&PropertyIndexWrite {
            property: "score".into(),
            value: PropertyIndexValue::Number("4.5".into()),
        }));
        assert!(indexes.contains(&PropertyIndexWrite {
            property: "enabled".into(),
            value: PropertyIndexValue::Boolean(true),
        }));
        assert!(indexes.contains(&PropertyIndexWrite {
            property: "tags".into(),
            value: PropertyIndexValue::String("critical".into()),
        }));
    }

    #[tokio::test]
    async fn shared_source_keeps_mappings_isolated_by_ontology() {
        let runtime = Runtime::seeded().await;
        let first = runtime
            .create(Request::new(CreateOntologyRequest {
                workspace_id: dev_workspace_id(),
                name: "Customer model".into(),
                slug: "customer_model".into(),
            }))
            .await
            .expect("first ontology can be created")
            .into_inner();
        let second = runtime
            .create(Request::new(CreateOntologyRequest {
                workspace_id: dev_workspace_id(),
                name: "Support model".into(),
                slug: "support_model".into(),
            }))
            .await
            .expect("second ontology can be created")
            .into_inner();
        let source = runtime
            .save(Request::new(SaveDataSourceRequest {
                data_source: Some(DataSource {
                    id: String::new(),
                    workspace_id: dev_workspace_id(),
                    name: "Shared CRM export".into(),
                    kind: 1,
                    configuration_json: "{}".into(),
                }),
            }))
            .await
            .expect("shared data source can be saved")
            .into_inner();

        for (ontology, object_type) in [(&first, "customer"), (&second, "ticket")] {
            runtime
                .save_mapping(Request::new(SaveOntologyDataMappingRequest {
                    mapping: Some(OntologyDataMapping {
                        id: String::new(),
                        workspace_id: dev_workspace_id(),
                        ontology_id: ontology.id.clone(),
                        data_source_id: source.id.clone(),
                        name: format!("{} mapping", ontology.name),
                        mapping_plan_json: format!(r#"{{"object_type":"{object_type}"}}"#),
                        revision: 0,
                    }),
                }))
                .await
                .expect("ontology-specific mapping can be saved");
        }

        let first_mappings = runtime
            .list_mappings(Request::new(ListOntologyDataMappingsRequest {
                workspace_id: dev_workspace_id(),
                ontology_id: first.id.clone(),
            }))
            .await
            .expect("first mappings can be listed")
            .into_inner()
            .mappings;
        let second_mappings = runtime
            .list_mappings(Request::new(ListOntologyDataMappingsRequest {
                workspace_id: dev_workspace_id(),
                ontology_id: second.id.clone(),
            }))
            .await
            .expect("second mappings can be listed")
            .into_inner()
            .mappings;

        assert_eq!(first_mappings.len(), 1);
        assert_eq!(second_mappings.len(), 1);
        assert_eq!(first_mappings[0].data_source_id, source.id);
        assert_eq!(second_mappings[0].data_source_id, source.id);
        assert_ne!(first_mappings[0].id, second_mappings[0].id);
        assert_eq!(first_mappings[0].ontology_id, first.id);
        assert_eq!(second_mappings[0].ontology_id, second.id);
    }

    #[tokio::test]
    async fn imported_graph_is_queryable_only_through_its_ontology_version() {
        let runtime = Runtime::seeded().await;
        let ontology_id =
            Uuid::new_v5(&Uuid::NAMESPACE_URL, b"context-hub/dev/service-map").to_string();
        let version = runtime
            .publish(Request::new(PublishOntologyRequest {
                ontology_id: ontology_id.clone(),
                expected_revision: 0,
            }))
            .await
            .expect("seed ontology can be published")
            .into_inner();
        let source = runtime
            .save(Request::new(SaveDataSourceRequest {
                data_source: Some(DataSource {
                    id: String::new(),
                    workspace_id: dev_workspace_id(),
                    name: "Services".into(),
                    kind: 1,
                    configuration_json: "{}".into(),
                }),
            }))
            .await
            .expect("source can be saved")
            .into_inner();
        let mapping = runtime
            .save_mapping(Request::new(SaveOntologyDataMappingRequest {
                mapping: Some(OntologyDataMapping {
                    id: String::new(),
                    workspace_id: dev_workspace_id(),
                    ontology_id,
                    data_source_id: source.id.clone(),
                    name: "Service mapping".into(),
                    mapping_plan_json: "{}".into(),
                    revision: 0,
                }),
            }))
            .await
            .expect("mapping can be saved")
            .into_inner();

        let job = runtime
            .import_graph(Request::new(ImportGraphRequest {
                data_source_id: source.id,
                ontology_mapping_id: mapping.id,
                ontology_version_id: version.id.clone(),
                nodes: [
                    ("billing", "Billing"),
                    ("search", "Search"),
                    ("users", "Users"),
                ]
                .into_iter()
                .map(|(id, name)| GraphNode {
                    id: format!("service:{id}"),
                    object_type: "service".into(),
                    properties_json: format!(r#"{{"id":"{id}","name":"{name}"}}"#),
                })
                .collect(),
                edges: vec![],
            }))
            .await
            .expect("mapped graph can be imported")
            .into_inner();
        assert_eq!(job.state, IngestionState::Succeeded as i32);
        assert_eq!(job.nodes_written, 3);

        let graph = query_service_page(&runtime, &version.id, 2, String::new()).await;
        assert_eq!(graph.nodes.len(), 2);
        assert_eq!(graph.nodes[0].id, "service:billing");
        assert!(!graph.next_cursor.is_empty());
        assert!(graph.truncated);

        let next_page = query_service_page(&runtime, &version.id, 2, graph.next_cursor).await;
        assert_eq!(next_page.nodes.len(), 1);
        assert_eq!(next_page.nodes[0].id, "service:users");
        assert!(next_page.next_cursor.is_empty());
        assert!(!next_page.truncated);

        let object = runtime
            .get_object(Request::new(GetObjectRequest {
                workspace_id: dev_workspace_id(),
                ontology_version_id: version.id,
                object_type: "service".into(),
                id: "service:billing".into(),
            }))
            .await
            .expect("imported object can be fetched")
            .into_inner();
        assert!(object.properties_json.contains("Billing"));
    }

    async fn query_service_page(
        runtime: &Runtime,
        ontology_version_id: &str,
        limit: u32,
        cursor: String,
    ) -> QueryGraphResponse {
        runtime
            .query(Request::new(GraphQuery {
                workspace_id: dev_workspace_id(),
                ontology_version_id: ontology_version_id.to_owned(),
                root_type: "service".into(),
                filters: vec![],
                traversal: vec![],
                projection: vec![],
                limit,
                cursor,
            }))
            .await
            .expect("graph page can be queried")
            .into_inner()
    }

    #[tokio::test]
    async fn uploaded_json_runs_through_datafusion_into_the_graph() {
        let runtime = Runtime::seeded().await;
        let _ = assert_uploaded_json_pipeline(&runtime).await;
    }

    #[tokio::test]
    async fn resumes_interrupted_ingestion_jobs() {
        let runtime = Runtime::seeded().await;
        let (source, mapping, version) = prepare_uploaded_json_scope(&runtime).await;
        let job = IngestionJob {
            id: Uuid::new_v4().to_string(),
            data_source_id: source.id,
            state: IngestionState::Running as i32,
            rows_read: 0,
            nodes_written: 0,
            edges_written: 0,
            rows_rejected: 0,
            error: String::new(),
            ontology_mapping_id: mapping.id,
            ontology_version_id: version.id,
            workspace_id: source.workspace_id,
        };
        runtime
            .jobs
            .write()
            .await
            .insert(job.id.clone(), job.clone());

        runtime.resume_ingestion_jobs().await;

        let completed = wait_for_job(&runtime, job).await;
        assert_eq!(completed.state, IngestionState::Succeeded as i32);
        assert_eq!(completed.nodes_written, 1);
    }

    #[tokio::test]
    async fn fails_jobs_that_cannot_be_resumed() {
        let runtime = Runtime::seeded().await;
        let job = IngestionJob {
            id: Uuid::new_v4().to_string(),
            data_source_id: Uuid::new_v4().to_string(),
            state: IngestionState::Queued as i32,
            rows_read: 0,
            nodes_written: 0,
            edges_written: 0,
            rows_rejected: 0,
            error: String::new(),
            ontology_mapping_id: Uuid::new_v4().to_string(),
            ontology_version_id: Uuid::new_v4().to_string(),
            workspace_id: dev_workspace_id(),
        };
        runtime
            .jobs
            .write()
            .await
            .insert(job.id.clone(), job.clone());

        runtime.resume_ingestion_jobs().await;

        let failed = runtime.jobs.read().await.get(&job.id).cloned().unwrap();
        assert_eq!(failed.state, IngestionState::Failed as i32);
        assert!(failed.error.contains("could not be resumed"));
    }

    #[tokio::test]
    async fn minio_datafusion_clickhouse_pipeline() {
        if std::env::var("CONTEXT_HUB_PIPELINE_TEST").as_deref() != Ok("1") {
            return;
        }
        let clickhouse = clickhouse::Client::default()
            .with_url(
                std::env::var("CLICKHOUSE_URL").unwrap_or_else(|_| "http://localhost:8123".into()),
            )
            .with_database("context_hub")
            .with_user("context_hub")
            .with_password("context_hub")
            .with_setting("input_format_binary_read_json_as_string", "1")
            .with_setting("output_format_binary_write_json_as_string", "1");
        let source_store: Arc<dyn SourceObjectStore> = Arc::new(
            ObjectStoreSourceRepository::s3(
                &std::env::var("S3_ENDPOINT").unwrap_or_else(|_| "http://localhost:9002".into()),
                &std::env::var("S3_BUCKET").unwrap_or_else(|_| "context-hub".into()),
                &std::env::var("S3_ACCESS_KEY").unwrap_or_else(|_| "context_hub".into()),
                &std::env::var("S3_SECRET_KEY").unwrap_or_else(|_| "context_hub_dev_secret".into()),
            )
            .expect("MinIO can be configured"),
        );
        let runtime = Runtime::seeded_with_stores(
            Arc::new(ClickHouseGraphRepository::new(clickhouse.clone())),
            source_store.clone(),
            Some(Arc::new(ClickHouseControlPlaneRepository::new(
                clickhouse.clone(),
            ))),
        )
        .await
        .expect("persistent runtime can be seeded");
        let (source_id, mapping_id, version_id, job_id) =
            assert_uploaded_json_pipeline(&runtime).await;
        let indexed = clickhouse
            .query(
                "SELECT count() FROM property_string_index FINAL WHERE workspace_id = ? AND ontology_version_id = ? AND object_type = 'service' AND property = 'id' AND value = 'billing' AND object_id = 'service:billing' AND deleted = false",
            )
            .bind(dev_workspace_id())
            .bind(version_id.clone())
            .fetch_one::<u64>()
            .await
            .expect("indexed property can be read from ClickHouse");
        assert_eq!(indexed, 1);
        let interrupted = IngestionJob {
            id: Uuid::new_v4().to_string(),
            data_source_id: source_id.clone(),
            state: IngestionState::Running as i32,
            rows_read: 0,
            nodes_written: 0,
            edges_written: 0,
            rows_rejected: 0,
            error: String::new(),
            ontology_mapping_id: mapping_id.clone(),
            ontology_version_id: version_id.clone(),
            workspace_id: dev_workspace_id(),
        };
        runtime
            .persist_job(&interrupted)
            .await
            .expect("interrupted job can be persisted");
        drop(runtime);

        let reloaded = Runtime::seeded_with_stores(
            Arc::new(ClickHouseGraphRepository::new(clickhouse.clone())),
            source_store,
            Some(Arc::new(ClickHouseControlPlaneRepository::new(clickhouse))),
        )
        .await
        .expect("persistent runtime can be reloaded");
        assert!(reloaded.data_sources.read().await.contains_key(&source_id));
        assert!(reloaded.mappings.read().await.contains_key(&mapping_id));
        assert!(reloaded.jobs.read().await.contains_key(&job_id));
        let recovered = wait_for_job(&reloaded, interrupted).await;
        assert_eq!(recovered.state, IngestionState::Succeeded as i32);
        assert_eq!(recovered.nodes_written, 1);
        assert!(
            reloaded
                .versions
                .read()
                .await
                .values()
                .flatten()
                .any(|version| version.id == version_id)
        );
    }

    async fn assert_uploaded_json_pipeline(runtime: &Runtime) -> (String, String, String, String) {
        let (source, mapping, version) = prepare_uploaded_json_scope(runtime).await;
        let queued = runtime
            .start(Request::new(StartIngestionRequest {
                data_source_id: source.id.clone(),
                ontology_mapping_id: mapping.id.clone(),
                ontology_version_id: version.id.clone(),
            }))
            .await
            .expect("ingestion can be queued")
            .into_inner();
        let completed = wait_for_job(runtime, queued).await;
        assert_eq!(completed.state, IngestionState::Succeeded as i32);
        assert_eq!(completed.nodes_written, 1);

        let graph = query_service_page(runtime, &version.id, 10, String::new()).await;
        assert_eq!(graph.nodes.len(), 1);
        assert_eq!(graph.nodes[0].id, "service:billing");
        (source.id, mapping.id, version.id, completed.id)
    }

    async fn prepare_uploaded_json_scope(
        runtime: &Runtime,
    ) -> (DataSource, OntologyDataMapping, OntologyVersion) {
        let ontology_id =
            Uuid::new_v5(&Uuid::NAMESPACE_URL, b"context-hub/dev/service-map").to_string();
        let expected_revision = current_draft_revision(runtime, &ontology_id).await;
        let version = runtime
            .publish(Request::new(PublishOntologyRequest {
                ontology_id: ontology_id.clone(),
                expected_revision,
            }))
            .await
            .expect("seed ontology can be published")
            .into_inner();
        let upload = runtime
            .upload(Request::new(UploadDataSourceRequest {
                workspace_id: dev_workspace_id(),
                name: "Service export".into(),
                file_name: "services.json".into(),
                format: 1,
                content: br#"[{"service_id":"billing"}]"#.to_vec(),
            }))
            .await
            .expect("JSON source can be uploaded")
            .into_inner();
        let source = upload.data_source.expect("upload returns a data source");
        let restored = runtime
            .get_upload(Request::new(GetUploadDataSourceRequest {
                id: source.id.clone(),
            }))
            .await
            .expect("uploaded source can be restored")
            .into_inner();
        assert_eq!(restored.file_name, "services.json");
        assert_eq!(restored.content, br#"[{"service_id":"billing"}]"#);
        assert_eq!(restored.sha256.len(), 64);
        let plan = MappingPlan {
            id: Uuid::new_v4(),
            object_type: "service".into(),
            identity_fields: vec!["service_id".into()],
            fields: vec![context_hub_mapping::FieldMapping {
                source: "service_id".into(),
                target: "id".into(),
                transforms: vec![],
                on_error: context_hub_mapping::ErrorStrategy::RejectRow,
            }],
            links: vec![],
            row_filter: None,
        };
        let mapping = runtime
            .save_mapping(Request::new(SaveOntologyDataMappingRequest {
                mapping: Some(OntologyDataMapping {
                    id: String::new(),
                    workspace_id: dev_workspace_id(),
                    ontology_id,
                    data_source_id: source.id.clone(),
                    name: "Service mapping".into(),
                    mapping_plan_json: serde_json::to_string(&plan).unwrap(),
                    revision: 0,
                }),
            }))
            .await
            .expect("mapping can be saved")
            .into_inner();
        (source, mapping, version)
    }

    async fn wait_for_job(runtime: &Runtime, mut job: IngestionJob) -> IngestionJob {
        for _ in 0..500 {
            job = runtime
                .get_job(Request::new(GetIngestionJobRequest { id: job.id.clone() }))
                .await
                .expect("job can be polled")
                .into_inner();
            if job.state == IngestionState::Succeeded as i32
                || job.state == IngestionState::Failed as i32
            {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        job
    }

    async fn current_draft_revision(runtime: &Runtime, ontology_id: &str) -> u64 {
        runtime
            .drafts
            .read()
            .await
            .get(ontology_id)
            .expect("seed ontology exists")
            .revision
    }
}
