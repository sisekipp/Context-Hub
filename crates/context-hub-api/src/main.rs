use std::{collections::HashMap, fmt::Write as _, net::SocketAddr, sync::Arc};

use chrono::Utc;
use context_hub_api::context_hub::v1::{
    CreateOntologyRequest, DataSource, GetIngestionJobRequest, GetObjectRequest,
    GetOntologyDraftRequest, GetWorkspaceRequest, GraphNode, GraphQuery, IngestionJob,
    IngestionState, ListDataSourcesRequest, ListDataSourcesResponse, ListOntologiesRequest,
    ListOntologiesResponse, ListOntologyDataMappingsRequest, ListOntologyDataMappingsResponse,
    ListOntologyVersionsRequest, ListOntologyVersionsResponse, ListWorkspacesResponse, Ontology,
    OntologyDataMapping, OntologyDraft, OntologyVersion, PublishOntologyRequest,
    QueryGraphResponse, SaveDataSourceRequest, SaveOntologyDataMappingRequest,
    SaveOntologyDraftRequest, StartIngestionRequest, ValidateOntologyRequest,
    ValidateOntologyResponse, ValidationIssue, Workspace,
    data_source_service_server::{DataSourceService, DataSourceServiceServer},
    graph_service_server::{GraphService, GraphServiceServer},
    ingestion_service_server::{IngestionService, IngestionServiceServer},
    ontology_service_server::{OntologyService, OntologyServiceServer},
    workspace_service_server::{WorkspaceService, WorkspaceServiceServer},
};
use context_hub_domain::{
    ObjectTypeDefinition, OntologyDefinition, PropertyDefinition, ScalarType, ValueType,
};
use prost_types::Timestamp;
use sha2::{Digest, Sha256};
use tokio::sync::RwLock;
use tonic::{Request, Response, Status, transport::Server};
use tracing_subscriber::EnvFilter;
use uuid::Uuid;

#[derive(Clone, Default)]
struct Runtime {
    ontologies: Arc<RwLock<HashMap<String, Ontology>>>,
    drafts: Arc<RwLock<HashMap<String, OntologyDraft>>>,
    versions: Arc<RwLock<HashMap<String, Vec<OntologyVersion>>>>,
    data_sources: Arc<RwLock<HashMap<String, DataSource>>>,
    mappings: Arc<RwLock<HashMap<String, OntologyDataMapping>>>,
    jobs: Arc<RwLock<HashMap<String, IngestionJob>>>,
}

impl Runtime {
    async fn seeded() -> Self {
        let runtime = Self::default();
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
        if !self
            .data_sources
            .read()
            .await
            .contains_key(&request.data_source_id)
        {
            return Err(Status::not_found("data source not found"));
        }
        let mapping = self
            .mappings
            .read()
            .await
            .get(&request.ontology_mapping_id)
            .cloned()
            .ok_or_else(|| Status::not_found("ontology mapping not found"))?;
        if mapping.data_source_id != request.data_source_id {
            return Err(Status::invalid_argument(
                "mapping does not belong to the selected data source",
            ));
        }
        if request.ontology_version_id.is_empty() {
            return Err(Status::invalid_argument("ontology_version_id is required"));
        }
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
        };
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
        if query.workspace_id != dev_workspace_id() {
            return Err(Status::permission_denied("workspace is not accessible"));
        }
        if query.traversal.len() > 6 {
            return Err(Status::invalid_argument(
                "traversal depth exceeds the maximum of 6",
            ));
        }
        let limit = if query.limit == 0 { 500 } else { query.limit };
        if limit > 5_000 {
            return Err(Status::invalid_argument(
                "node limit exceeds the maximum of 5000",
            ));
        }
        Ok(Response::new(QueryGraphResponse {
            nodes: vec![],
            edges: vec![],
            next_cursor: String::new(),
            truncated: false,
        }))
    }
    async fn get_object(
        &self,
        _: Request<GetObjectRequest>,
    ) -> Result<Response<GraphNode>, Status> {
        Err(Status::not_found("object not found"))
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
    let runtime = Runtime::seeded().await;
    tracing::info!(%address, "starting ContextHub gRPC and gRPC-Web server");
    Server::builder()
        .accept_http1(true)
        .layer(tonic_web::GrpcWebLayer::new())
        .add_service(WorkspaceServiceServer::new(runtime.clone()))
        .add_service(OntologyServiceServer::new(runtime.clone()))
        .add_service(DataSourceServiceServer::new(runtime.clone()))
        .add_service(IngestionServiceServer::new(runtime.clone()))
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
}
