import { createClient } from "@connectrpc/connect";
import { createGrpcWebTransport } from "@connectrpc/connect-web";
import {
  DataSourceService,
  DataSourceKind,
  GraphService,
  IngestionService,
  IngestionState,
  OntologyService,
  SourceFileFormat,
} from "@/gen/context_hub/v1/context_hub_pb";
import type { OntologyCatalog } from "@/lib/ontology-catalog";
import type { GraphValue, ImportedGraph } from "@/lib/graph-data";
import { serializeTransform, type MappingTransform } from "@/lib/mapping-transforms";

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
export type BackendDataSource = { id: string; name: string; fileName: string; kind: "upload" | "rest" | "graphql" };
export type RestKeyValue = { key: string; value: string };
export type RestSourceInput = {
  name: string;
  url: string;
  recordPath: string;
  headers: RestKeyValue[];
  query: RestKeyValue[];
  pagination: "none" | "page" | "cursor";
  pageParameter: string;
  pageStart: number;
  pageSizeParameter: string;
  pageSize: number;
  cursorParameter: string;
  nextCursorPath: string;
  maxPages: number;
  maxBytes: number;
  timeoutSeconds: number;
  retryAttempts: number;
};
export type GraphqlSourceInput = {
  name: string;
  url: string;
  query: string;
  variables: string;
  recordPath: string;
  headers: RestKeyValue[];
  cursorEnabled: boolean;
  cursorVariable: string;
  nextCursorPath: string;
  maxPages: number;
  maxBytes: number;
  timeoutSeconds: number;
  retryAttempts: number;
};

export type BackendFieldMapping = {
  sourceField: string;
  targetProperty: string;
  transforms: MappingTransform[];
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

export async function listWorkspaceDataSources(): Promise<BackendDataSource[]> {
  const response = await dataSources.list({ workspaceId: DEV_WORKSPACE_ID });
  return response.dataSources.flatMap<BackendDataSource>((source) => {
    if (source.kind === DataSourceKind.REST) {
      return [{ id: source.id, name: source.name, fileName: `REST · ${source.name}.json`, kind: "rest" as const }];
    }
    if (source.kind === DataSourceKind.GRAPHQL) {
      return [{ id: source.id, name: source.name, fileName: `GraphQL · ${source.name}.json`, kind: "graphql" as const }];
    }
    if (source.kind !== DataSourceKind.UPLOAD) return [];
    try {
      const configuration = JSON.parse(source.configurationJson) as { file_name?: string };
      return [{ id: source.id, name: source.name, fileName: configuration.file_name || source.name, kind: "upload" as const }];
    } catch {
      return [{ id: source.id, name: source.name, fileName: source.name, kind: "upload" as const }];
    }
  });
}

function keyValues(items: RestKeyValue[]) {
  return Object.fromEntries(items.filter((item) => item.key.trim()).map((item) => [item.key.trim(), item.value]));
}

export async function saveWorkspaceRestSource(input: RestSourceInput) {
  const pagination = input.pagination === "page" ? {
    mode: "page",
    parameter: input.pageParameter,
    start: input.pageStart,
    page_size_parameter: input.pageSizeParameter || null,
    page_size: input.pageSizeParameter ? input.pageSize : null,
    stop_on_short_page: true,
  } : input.pagination === "cursor" ? {
    mode: "cursor",
    query_parameter: input.cursorParameter,
    next_cursor_path: input.nextCursorPath,
    initial_cursor: null,
  } : { mode: "none" };
  const source = await dataSources.save({ dataSource: {
    id: "",
    workspaceId: DEV_WORKSPACE_ID,
    name: input.name,
    kind: DataSourceKind.REST,
    configurationJson: JSON.stringify({
      url: input.url,
      headers: keyValues(input.headers),
      query: keyValues(input.query),
      record_path: input.recordPath || null,
      pagination,
      max_pages: input.maxPages,
      max_bytes: input.maxBytes,
      timeout_seconds: input.timeoutSeconds,
      retry_attempts: input.retryAttempts,
    }),
  } });
  return { id: source.id, name: source.name, fileName: `REST · ${source.name}.json`, kind: "rest" as const };
}

export async function saveWorkspaceGraphqlSource(input: GraphqlSourceInput) {
  const variables = JSON.parse(input.variables || "{}") as unknown;
  const pagination = input.cursorEnabled ? {
    mode: "cursor",
    variable: input.cursorVariable,
    next_cursor_path: input.nextCursorPath,
    initial_cursor: null,
  } : { mode: "none" };
  const source = await dataSources.save({ dataSource: {
    id: "",
    workspaceId: DEV_WORKSPACE_ID,
    name: input.name,
    kind: DataSourceKind.GRAPHQL,
    configurationJson: JSON.stringify({
      url: input.url,
      query: input.query,
      variables,
      headers: keyValues(input.headers),
      record_path: input.recordPath,
      pagination,
      max_pages: input.maxPages,
      max_bytes: input.maxBytes,
      timeout_seconds: input.timeoutSeconds,
      retry_attempts: input.retryAttempts,
    }),
  } });
  return { id: source.id, name: source.name, fileName: `GraphQL · ${source.name}.json`, kind: "graphql" as const };
}

export async function previewWorkspaceSource(id: string) {
  const response = await dataSources.preview({ id });
  if (!response.dataSource) throw new Error("The backend did not return the requested data source.");
  return {
    id: response.dataSource.id,
    fileName: `${response.dataSource.kind === DataSourceKind.GRAPHQL ? "GraphQL" : "REST"} · ${response.dataSource.name}.json`,
    content: response.content,
    recordCount: Number(response.recordCount),
  };
}

export async function downloadWorkspaceSource(id: string) {
  const response = await dataSources.getUpload({ id });
  if (!response.dataSource) throw new Error("The backend did not return the requested data source.");
  return {
    id: response.dataSource.id,
    fileName: response.fileName,
    content: response.content,
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
  return {
    id: crypto.randomUUID(),
    object_type: objectMapping.objectType,
    identity_fields: identityFields,
    fields: objectMapping.properties.map((field) => ({
      source: field.sourceField,
      target: field.targetProperty,
      transforms: field.transforms.map(serializeTransform),
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

export async function loadPersistedGraph(
  ontology: BackendOntology,
  catalog: OntologyCatalog,
  cursors?: Array<string | null>,
): Promise<ImportedGraph> {
  if (!ontology.activeVersionId) throw new Error("This ontology has no published graph version yet.");
  const queries = [
    ...catalog.objectTypes.map((objectType) => ({
      workspaceId: DEV_WORKSPACE_ID,
      ontologyVersionId: ontology.activeVersionId,
      rootType: objectType.apiName,
      filters: [], traversal: [], projection: [], limit: 2_000,
    })),
    ...catalog.linkTypes.map((link) => ({
      workspaceId: DEV_WORKSPACE_ID,
      ontologyVersionId: ontology.activeVersionId,
      rootType: link.sourceType,
      filters: [],
      traversal: [{ linkType: link.apiName, targetType: link.targetType, reverse: false }],
      projection: [], limit: 2_000,
    })),
  ];
  const responses = await Promise.all(queries.map((query, index) => {
    const cursor = cursors?.[index];
    return cursor === null ? null : graph.query({ ...query, cursor: cursor ?? "" });
  }));
  const nextCursors = responses.map((response) => response?.nextCursor || null);
  const nodes = new Map<string, ImportedGraph["nodes"][number]>();
  const links = new Map<string, ImportedGraph["links"][number]>();
  for (const response of responses) {
    if (!response) continue;
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
    pagination: { cursors: nextCursors, hasMore: nextCursors.some((cursor) => cursor !== null) },
  };
}
