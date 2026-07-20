import { createClient } from "@connectrpc/connect";
import { createGrpcWebTransport } from "@connectrpc/connect-web";
import {
  DataSourceService,
  GraphService,
  IngestionService,
  IngestionState,
  OntologyService,
  SourceFileFormat,
} from "@/gen/context_hub/v1/context_hub_pb";
import type { OntologyCatalog } from "@/lib/ontology-catalog";
import type { GraphValue, ImportedGraph } from "@/lib/graph-data";

export const DEV_WORKSPACE_ID = "00000000-0000-0000-0000-000000000001";

const transport = createGrpcWebTransport({
  baseUrl: process.env.NEXT_PUBLIC_CONTEXT_HUB_API_URL ?? "http://localhost:50051",
  useBinaryFormat: true,
});

const dataSources = createClient(DataSourceService, transport);
const ontologies = createClient(OntologyService, transport);
const ingestions = createClient(IngestionService, transport);
const graph = createClient(GraphService, transport);

export type BackendOntology = { id: string; name: string; slug: string; activeVersionId: string };

export type BackendFieldMapping = {
  sourceField: string;
  targetProperty: string;
  transform: "None" | "Trim" | "Lowercase" | "Uppercase";
};

export type BackendObjectMapping = {
  objectType: string;
  identityProperty: string;
  properties: BackendFieldMapping[];
};

export type BackendLinkMapping = {
  sourceObjectType: string;
  sourceField: string;
  linkType: string;
  targetObjectType: string;
  targetIdentityProperty: string;
  missingTarget: "create" | "skip" | "error";
};

function sourceFormat(fileName: string): SourceFileFormat {
  if (/\.(ndjson|jsonl)$/i.test(fileName)) return SourceFileFormat.NDJSON;
  if (/\.csv$/i.test(fileName)) return SourceFileFormat.CSV;
  return SourceFileFormat.JSON;
}

export async function uploadWorkspaceSource(file: File) {
  const response = await dataSources.upload({
    workspaceId: DEV_WORKSPACE_ID,
    name: file.name.replace(/\.[^.]+$/, "") || file.name,
    fileName: file.name,
    format: sourceFormat(file.name),
    content: new Uint8Array(await file.arrayBuffer()),
  });
  if (!response.dataSource) throw new Error("The backend did not return a data source.");
  return {
    id: response.dataSource.id,
    objectKey: response.objectKey,
    sizeBytes: response.sizeBytes,
    sha256: response.sha256,
  };
}

export async function listWorkspaceOntologies(): Promise<BackendOntology[]> {
  const response = await ontologies.list({ workspaceId: DEV_WORKSPACE_ID });
  return response.ontologies.map((ontology) => ({ id: ontology.id, name: ontology.name, slug: ontology.slug, activeVersionId: ontology.activeVersionId }));
}

export async function createWorkspaceOntology(name: string, slug: string): Promise<BackendOntology> {
  const ontology = await ontologies.create({ workspaceId: DEV_WORKSPACE_ID, name, slug });
  return { id: ontology.id, name: ontology.name, slug: ontology.slug, activeVersionId: ontology.activeVersionId };
}

function ontologyDefinition(name: string, slug: string, catalog: OntologyCatalog) {
  return {
    api_name: slug,
    display_name: name,
    description: null,
    object_types: catalog.objectTypes.map((objectType) => ({
      api_name: objectType.apiName,
      display_name: objectType.displayName,
      description: null,
      properties: objectType.properties.filter((property) => !property.derived).map((property) => ({
        api_name: property.apiName,
        display_name: property.displayName,
        value_type: { scalar: scalarType(property.type), list: property.type === "List" },
        required: !!property.identity,
        unique: !!property.identity,
        identity: !!property.identity,
        indexed: !!property.identity,
        description: null,
      })),
      shared_properties: [],
      derived_properties: [],
      implements: [],
    })),
    link_types: catalog.linkTypes.map((link) => ({
      api_name: link.apiName,
      display_name: link.displayName,
      source_type: link.sourceType,
      target_type: link.targetType,
      source_cardinality: "many",
      target_cardinality: "many",
      required: false,
      properties: [],
      description: null,
    })),
    interfaces: [], value_types: [], struct_types: [], shared_properties: [], functions: [],
  };
}

function scalarType(type: string) {
  const known: Record<string, string> = {
    String: "string", Boolean: "boolean", Int64: "int64", Float64: "float64",
    Decimal: "decimal", Date: "date", Timestamp: "timestamp", UUID: "uuid", JSON: "json",
  };
  return known[type] ?? "string";
}

export async function publishOntologyCatalog(ontology: Pick<BackendOntology, "id" | "name" | "slug">, catalog: OntologyCatalog) {
  const current = await ontologies.getDraft({ id: ontology.id });
  const definitionJson = JSON.stringify(ontologyDefinition(ontology.name, ontology.slug, catalog));
  const saved = await ontologies.saveDraft({
    draft: {
      id: current.id,
      workspaceId: current.workspaceId,
      name: ontology.name,
      slug: ontology.slug,
      revision: current.revision,
      definitionJson,
      layoutJson: current.layoutJson || "{}",
      updatedAt: current.updatedAt,
    },
    expectedRevision: current.revision,
  });
  return ontologies.publish({ ontologyId: ontology.id, expectedRevision: saved.revision });
}

function mappingPlan(objectMapping: BackendObjectMapping, links: BackendLinkMapping[]) {
  const identityFields = objectMapping.properties
    .filter((field) => field.targetProperty === objectMapping.identityProperty)
    .map((field) => field.sourceField);
  const transforms: Record<BackendFieldMapping["transform"], Array<{ kind: string }>> = {
    None: [], Trim: [{ kind: "trim" }], Lowercase: [{ kind: "lowercase" }], Uppercase: [{ kind: "uppercase" }],
  };
  return {
    id: crypto.randomUUID(),
    object_type: objectMapping.objectType,
    identity_fields: identityFields,
    fields: objectMapping.properties.map((field) => ({
      source: field.sourceField,
      target: field.targetProperty,
      transforms: transforms[field.transform],
      on_error: "reject_row",
    })),
    links: links.filter((link) => link.sourceObjectType === objectMapping.objectType).map((link) => ({
      link_type: link.linkType,
      target_object_type: link.targetObjectType,
      source_fields: [link.sourceField],
      target_identity_fields: [link.targetIdentityProperty],
      missing_target: link.missingTarget,
    })),
    row_filter: null,
  };
}

function mappingBundle(objectMappings: BackendObjectMapping[], links: BackendLinkMapping[]) {
  return {
    id: crypto.randomUUID(),
    plans: objectMappings.map((objectMapping) => mappingPlan(objectMapping, links)),
  };
}

export async function saveOntologyMapping(options: {
  id?: string;
  ontologyId: string;
  dataSourceId: string;
  name: string;
  objectMappings: BackendObjectMapping[];
  links: BackendLinkMapping[];
}) {
  return dataSources.saveMapping({ mapping: {
    id: options.id ?? "",
    workspaceId: DEV_WORKSPACE_ID,
    ontologyId: options.ontologyId,
    dataSourceId: options.dataSourceId,
    name: options.name,
    mappingPlanJson: JSON.stringify(mappingBundle(options.objectMappings, options.links)),
    revision: 0n,
  } });
}

export async function startIngestion(dataSourceId: string, mappingId: string, versionId: string) {
  let job = await ingestions.start({ dataSourceId, ontologyMappingId: mappingId, ontologyVersionId: versionId });
  for (let attempt = 0; attempt < 300 && (job.state === IngestionState.QUEUED || job.state === IngestionState.RUNNING); attempt += 1) {
    await new Promise((resolve) => window.setTimeout(resolve, 100));
    job = await ingestions.getJob({ id: job.id });
  }
  return job;
}

const graphColors = ["#7c9cff", "#5ed3b5", "#f7b267", "#c792ea", "#ff7d9d"];

export async function loadPersistedGraph(ontology: BackendOntology, catalog: OntologyCatalog): Promise<ImportedGraph> {
  if (!ontology.activeVersionId) throw new Error("This ontology has no published graph version yet.");
  const requests = [
    ...catalog.objectTypes.map((objectType) => graph.query({
      workspaceId: DEV_WORKSPACE_ID,
      ontologyVersionId: ontology.activeVersionId,
      rootType: objectType.apiName,
      filters: [], traversal: [], projection: [], limit: 5_000, cursor: "",
    })),
    ...catalog.linkTypes.map((link) => graph.query({
      workspaceId: DEV_WORKSPACE_ID,
      ontologyVersionId: ontology.activeVersionId,
      rootType: link.sourceType,
      filters: [],
      traversal: [{ linkType: link.apiName, targetType: link.targetType, reverse: false }],
      projection: [], limit: 5_000, cursor: "",
    })),
  ];
  const responses = await Promise.all(requests);
  const nodes = new Map<string, ImportedGraph["nodes"][number]>();
  const links = new Map<string, ImportedGraph["links"][number]>();
  for (const response of responses) {
    for (const node of response.nodes) {
      const objectType = catalog.objectTypes.find((type) => type.apiName === node.objectType);
      const properties = JSON.parse(node.propertiesJson || "{}") as Record<string, GraphValue>;
      const colorIndex = Math.max(0, catalog.objectTypes.findIndex((type) => type.apiName === node.objectType));
      nodes.set(node.id, {
        id: node.id,
        name: String(properties.name ?? properties.id ?? node.id),
        kind: objectType?.displayName ?? node.objectType,
        group: objectType?.displayName ?? node.objectType,
        color: graphColors[colorIndex % graphColors.length],
        properties,
      });
    }
    for (const edge of response.edges) {
      links.set(edge.id, {
        source: edge.sourceId,
        target: edge.targetId,
        label: edge.linkType,
        properties: JSON.parse(edge.propertiesJson || "{}") as Record<string, GraphValue>,
      });
    }
  }
  return {
    nodes: [...nodes.values()],
    links: [...links.values()],
    sourceName: `${ontology.name} · ClickHouse`,
    importedAt: new Date().toISOString(),
    recordCount: nodes.size,
    skippedCount: 0,
    linkErrorCount: 0,
    ontologyBindings: {
      objectTypes: catalog.objectTypes.map((type) => type.apiName),
      linkTypes: catalog.linkTypes.map((type) => type.apiName),
    },
  };
}
