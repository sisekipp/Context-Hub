use std::{collections::HashMap, fmt::Write as _, net::SocketAddr, sync::Arc};

use chrono::Utc;
use context_hub_api::context_hub::v1::{
    CreateOntologyRequest, DataSource, GetIngestionJobRequest, GetObjectRequest,
    GetOntologyDraftRequest, GetWorkspaceRequest, GraphEdge, GraphNode, GraphQuery,
    ImportGraphRequest, IngestionJob, IngestionState, ListDataSourcesRequest,
    ListDataSourcesResponse, ListOntologiesRequest, ListOntologiesResponse,
    ListOntologyDataMappingsRequest, ListOntologyDataMappingsResponse, ListOntologyVersionsRequest,
    ListOntologyVersionsResponse, ListWorkspacesResponse, Ontology, OntologyDataMapping,
    OntologyDraft, OntologyVersion, PublishOntologyRequest, QueryGraphResponse,
    SaveDataSourceRequest, SaveOntologyDataMappingRequest, SaveOntologyDraftRequest,
    StartIngestionRequest, UploadDataSourceRequest, UploadDataSourceResponse,
    ValidateOntologyRequest, ValidateOntologyResponse, ValidationIssue, Workspace,
    data_source_service_server::{DataSourceService, DataSourceServiceServer},
    graph_service_server::{GraphService, GraphServiceServer},
    ingestion_service_server::{IngestionService, IngestionServiceServer},
    ontology_service_server::{OntologyService, OntologyServiceServer},
    workspace_service_server::{WorkspaceService, WorkspaceServiceServer},
};
use context_hub_domain::{
    ObjectTypeDefinition, OntologyDefinition, PropertyDefinition, ScalarType, ValueType,
};
use context_hub_mapping::{MappingPlan, SourceFormat, execute_source_mapping};
use context_hub_storage::{
    ClickHouseGraphRepository, FilterOperator as StorageFilterOperator, GraphEdgeWrite,
    GraphFilter as StorageGraphFilter, GraphNodeWrite, GraphQuery as StorageGraphQuery,
    GraphRepository, ObjectStoreSourceRepository, SourceObjectStore, StorageError,
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
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct UploadSourceConfiguration {
    object_key: String,
    file_name: String,
    format: SourceFormat,
    size_bytes: u64,
    sha256: String,
}

impl Runtime {
    #[cfg(test)]
    async fn seeded() -> Self {
        Self::seeded_with_stores(
            Arc::new(MemoryGraphRepository::default()),
            Arc::new(MemorySourceObjectStore::default()),
        )
        .await
    }

    async fn seeded_with_stores(
        graph: Arc<dyn GraphRepository>,
        source_store: Arc<dyn SourceObjectStore>,
    ) -> Self {
        let runtime = Self {
            ontologies: Arc::default(),
            drafts: Arc::default(),
            versions: Arc::default(),
            data_sources: Arc::default(),
            mappings: Arc::default(),
            jobs: Arc::default(),
            graph,
            source_store,
        };
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
        runtime
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
        if let Some(job) = self.jobs.write().await.get_mut(&job_id) {
            job.state = IngestionState::Running as i32;
        }
        let result = self
            .ingest_uploaded_source(&source, &mapping, &version)
            .await;
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
    }

    async fn ingest_uploaded_source(
        &self,
        source: &DataSource,
        mapping: &OntologyDataMapping,
        version: &OntologyVersion,
    ) -> Result<context_hub_mapping::MappedGraphBatch, String> {
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
        let plan: MappingPlan = serde_json::from_str(&mapping.mapping_plan_json)
            .map_err(|error| format!("mapping plan is invalid: {error}"))?;
        let definition: OntologyDefinition = serde_json::from_str(&version.definition_json)
            .map_err(|error| format!("stored ontology version is invalid: {error}"))?;
        validate_worker_plan(&plan, &definition)?;
        let mapped = execute_source_mapping(&plan, configuration.format, &content)
            .await
            .map_err(|error| error.to_string())?;
        let workspace_id = Uuid::parse_str(&source.workspace_id)
            .map_err(|error| format!("workspace id is invalid: {error}"))?;
        let ontology_version_id = Uuid::parse_str(&version.id)
            .map_err(|error| format!("ontology version id is invalid: {error}"))?;
        let data_source_id = Uuid::parse_str(&source.id)
            .map_err(|error| format!("data source id is invalid: {error}"))?;
        let write_version = u64::try_from(Utc::now().timestamp_micros())
            .map_err(|_| "system clock predates the Unix epoch".to_owned())?;
        let nodes = mapped
            .nodes
            .iter()
            .map(|node| GraphNodeWrite {
                workspace_id,
                ontology_version_id,
                object_type: node.object_type.clone(),
                object_id: node.object_id.clone(),
                source_id: data_source_id,
                external_id: node.object_id.clone(),
                properties_json: node.properties_json.clone(),
                version: write_version,
            })
            .collect::<Vec<_>>();
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
                source_type: plan.object_type.clone(),
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
        Ok(mapped)
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
        self.drafts.write().await.insert(
            id.clone(),
            OntologyDraft {
                id: id.clone(),
                workspace_id: request.workspace_id,
                name: request.name,
                slug: definition.api_name.clone(),
                revision: 0,
                definition_json: serde_json::to_string_pretty(&definition)
                    .map_err(|error| Status::internal(error.to_string()))?,
                layout_json: "{}".into(),
                updated_at: Some(timestamp(Utc::now())),
            },
        );
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
        if let Some(ontology) = self.ontologies.write().await.get_mut(&ontology_id) {
            ontology.active_version_id.clone_from(&version.id);
            ontology.revision += 1;
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
        const MAX_UPLOAD_BYTES: usize = 32 * 1024 * 1024;
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
        let mut truncated = false;
        for depth in 0..=storage_query.traversal.len() {
            let remaining = limit as usize - nodes_by_key.len();
            if remaining == 0 {
                truncated = true;
                break;
            }
            let mut prefix = storage_query.clone();
            prefix.traversal.truncate(depth);
            prefix.limit = u32::try_from(remaining)
                .expect("remaining node budget never exceeds the validated u32 query limit");
            let prefix_nodes = self
                .graph
                .query_nodes(&prefix)
                .await
                .map_err(storage_status)?;
            truncated |= prefix_nodes.len() == remaining;
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
            next_cursor: String::new(),
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

fn validate_graph_query(query: &GraphQuery, definition: &OntologyDefinition) -> Result<(), Status> {
    if !query.cursor.is_empty() {
        return Err(Status::unimplemented(
            "graph query cursors are not implemented yet",
        ));
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
        Arc::new(ClickHouseGraphRepository::new(clickhouse)),
        Arc::new(source_store),
    )
    .await;
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
                .max_decoding_message_size(34 * 1024 * 1024),
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
                nodes: vec![GraphNode {
                    id: "service:billing".into(),
                    object_type: "service".into(),
                    properties_json: r#"{"id":"billing","name":"Billing"}"#.into(),
                }],
                edges: vec![],
            }))
            .await
            .expect("mapped graph can be imported")
            .into_inner();
        assert_eq!(job.state, IngestionState::Succeeded as i32);
        assert_eq!(job.nodes_written, 1);

        let graph = runtime
            .query(Request::new(GraphQuery {
                workspace_id: dev_workspace_id(),
                ontology_version_id: version.id.clone(),
                root_type: "service".into(),
                filters: vec![],
                traversal: vec![],
                projection: vec![],
                limit: 50,
                cursor: String::new(),
            }))
            .await
            .expect("imported graph can be queried")
            .into_inner();
        assert_eq!(graph.nodes.len(), 1);
        assert_eq!(graph.nodes[0].id, "service:billing");

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

    #[tokio::test]
    async fn uploaded_json_runs_through_datafusion_into_the_graph() {
        assert_uploaded_json_pipeline(Runtime::seeded().await).await;
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
        let source_store = ObjectStoreSourceRepository::s3(
            &std::env::var("S3_ENDPOINT").unwrap_or_else(|_| "http://localhost:9002".into()),
            &std::env::var("S3_BUCKET").unwrap_or_else(|_| "context-hub".into()),
            &std::env::var("S3_ACCESS_KEY").unwrap_or_else(|_| "context_hub".into()),
            &std::env::var("S3_SECRET_KEY").unwrap_or_else(|_| "context_hub_dev_secret".into()),
        )
        .expect("MinIO can be configured");
        let runtime = Runtime::seeded_with_stores(
            Arc::new(ClickHouseGraphRepository::new(clickhouse)),
            Arc::new(source_store),
        )
        .await;
        assert_uploaded_json_pipeline(runtime).await;
    }

    async fn assert_uploaded_json_pipeline(runtime: Runtime) {
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
        let queued = runtime
            .start(Request::new(StartIngestionRequest {
                data_source_id: source.id,
                ontology_mapping_id: mapping.id,
                ontology_version_id: version.id.clone(),
            }))
            .await
            .expect("ingestion can be queued")
            .into_inner();

        let mut completed = queued;
        for _ in 0..500 {
            completed = runtime
                .get_job(Request::new(GetIngestionJobRequest {
                    id: completed.id.clone(),
                }))
                .await
                .expect("job can be polled")
                .into_inner();
            if completed.state == IngestionState::Succeeded as i32
                || completed.state == IngestionState::Failed as i32
            {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        assert_eq!(completed.state, IngestionState::Succeeded as i32);
        assert_eq!(completed.nodes_written, 1);

        let graph = runtime
            .query(Request::new(GraphQuery {
                workspace_id: dev_workspace_id(),
                ontology_version_id: version.id,
                root_type: "service".into(),
                filters: vec![],
                traversal: vec![],
                projection: vec![],
                limit: 10,
                cursor: String::new(),
            }))
            .await
            .expect("worker output can be queried")
            .into_inner();
        assert_eq!(graph.nodes.len(), 1);
        assert_eq!(graph.nodes[0].id, "service:billing");
    }
}
