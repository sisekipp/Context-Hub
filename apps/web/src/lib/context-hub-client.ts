import { createClient } from "@connectrpc/connect";
import { createGrpcWebTransport } from "@connectrpc/connect-web";
import {
  DataSourceService,
  DataSourceKind,
  AggregationFunction,
  FilterOperator,
  FunctionExecutionState,
  FunctionService,
  GraphService,
  IngestionService,
  IngestionState,
  OntologyService,
  SourceFileFormat,
  SortDirection,
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
const functions = createClient(FunctionService, transport);

export type BackendOntology = { id: string; name: string; slug: string; activeVersionId: string };
export type BackendOntologyDraft = { id: string; workspaceId: string; name: string; slug: string; revision: number; definitionJson: string; layoutJson: string };
export type BackendDataSource = { id: string; name: string; fileName: string; kind: "upload" | "rest" | "graphql"; configurationJson: string };
export type BackendDataSourceUsage = { ontologyId: string; ontologyName: string; mappingId: string; mappingName: string };
export type BackendIngestionJob = {
  id: string; dataSourceId: string; state: IngestionState; rowsRead: number; nodesWritten: number;
  edgesWritten: number; rowsRejected: number; error: string; ontologyMappingId: string;
  ontologyVersionId: string; createdAt: Date | null; startedAt: Date | null; completedAt: Date | null;
};
export type BackendIngestionEvent = { eventType: string; rowNumber: number; objectType: string; externalId: string; message: string; detailsJson: string; occurredAt: Date | null };
export type BackendPropertyProvenance = { property: string; dataSourceId: string; dataSourceName: string; sourceField: string; ontologyMappingId: string; ontologyMappingName: string; ingestionJobId: string; importedAt: Date | null };
export type BackendFunctionArtifact = { id: string; name: string; fileName: string; artifactUri: string; sizeBytes: number; sha256: string; createdAt: Date | null };
export type BackendFunctionExecution = { id: string; ontologyVersionId: string; functionApiName: string; executor: string; state: FunctionExecutionState; argumentsJson: string; resultJson: string; error: string; durationMillis: number; executedAt: Date | null };
export type RestKeyValue = { key: string; value: string };
export type RestSourceInput = {
  id?: string;
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
  id?: string;
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
  onError: "reject_row" | "use_null" | "abort_job";
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

export type GraphQuerySpec = {
  rootType: string;
  filters: Array<{ property: string; operator: "equal" | "not_equal" | "contains" | "greater_than" | "less_than"; value: string }>;
  traversal: Array<{ linkType: string; targetType: string; reverse: boolean }>;
  projection: string[];
  sort?: { property: string; direction: "ascending" | "descending" };
  aggregations: Array<{ property: string; function: "count" | "distinct_count" | "sum" | "average" | "minimum" | "maximum"; alias: string }>;
  limit: number;
};

function sourceFormat(fileName: string): SourceFileFormat {
  if (/\.(ndjson|jsonl)$/i.test(fileName)) return SourceFileFormat.NDJSON;
  if (/\.csv$/i.test(fileName)) return SourceFileFormat.CSV;
  if (/\.parquet$/i.test(fileName)) return SourceFileFormat.PARQUET;
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
      return [{ id: source.id, name: source.name, fileName: `REST · ${source.name}.json`, kind: "rest" as const, configurationJson: source.configurationJson }];
    }
    if (source.kind === DataSourceKind.GRAPHQL) {
      return [{ id: source.id, name: source.name, fileName: `GraphQL · ${source.name}.json`, kind: "graphql" as const, configurationJson: source.configurationJson }];
    }
    if (source.kind !== DataSourceKind.UPLOAD) return [];
    try {
      const configuration = JSON.parse(source.configurationJson) as { file_name?: string };
      return [{ id: source.id, name: source.name, fileName: configuration.file_name || source.name, kind: "upload" as const, configurationJson: source.configurationJson }];
    } catch {
      return [{ id: source.id, name: source.name, fileName: source.name, kind: "upload" as const, configurationJson: source.configurationJson }];
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
    id: input.id ?? "",
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
  return { id: source.id, name: source.name, fileName: `REST · ${source.name}.json`, kind: "rest" as const, configurationJson: source.configurationJson };
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
    id: input.id ?? "",
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
  return { id: source.id, name: source.name, fileName: `GraphQL · ${source.name}.json`, kind: "graphql" as const, configurationJson: source.configurationJson };
}

function pairs(value: unknown): RestKeyValue[] {
  if (!value || typeof value !== "object" || Array.isArray(value)) return [];
  return Object.entries(value).map(([key, entry]) => ({ key, value: String(entry ?? "") }));
}

export function restSourceInput(source: BackendDataSource): RestSourceInput {
  const configuration = JSON.parse(source.configurationJson) as Record<string, unknown>;
  const pagination = (configuration.pagination ?? {}) as Record<string, unknown>;
  const mode = pagination.mode === "page" || pagination.mode === "cursor" ? pagination.mode : "none";
  return {
    id: source.id, name: source.name, url: String(configuration.url ?? ""), recordPath: String(configuration.record_path ?? ""),
    headers: pairs(configuration.headers), query: pairs(configuration.query), pagination: mode,
    pageParameter: String(pagination.parameter ?? "page"), pageStart: Number(pagination.start ?? 1),
    pageSizeParameter: String(pagination.page_size_parameter ?? "limit"), pageSize: Number(pagination.page_size ?? 100),
    cursorParameter: String(pagination.query_parameter ?? "cursor"), nextCursorPath: String(pagination.next_cursor_path ?? "meta.next_cursor"),
    maxPages: Number(configuration.max_pages ?? 100), maxBytes: Number(configuration.max_bytes ?? 32 * 1024 * 1024),
    timeoutSeconds: Number(configuration.timeout_seconds ?? 30), retryAttempts: Number(configuration.retry_attempts ?? 2),
  };
}

export function graphqlSourceInput(source: BackendDataSource): GraphqlSourceInput {
  const configuration = JSON.parse(source.configurationJson) as Record<string, unknown>;
  const pagination = (configuration.pagination ?? {}) as Record<string, unknown>;
  return {
    id: source.id, name: source.name, url: String(configuration.url ?? ""), query: String(configuration.query ?? ""),
    variables: JSON.stringify(configuration.variables ?? {}, null, 2), recordPath: String(configuration.record_path ?? ""),
    headers: pairs(configuration.headers), cursorEnabled: pagination.mode === "cursor",
    cursorVariable: String(pagination.variable ?? "after"), nextCursorPath: String(pagination.next_cursor_path ?? "data.pageInfo.endCursor"),
    maxPages: Number(configuration.max_pages ?? 100), maxBytes: Number(configuration.max_bytes ?? 32 * 1024 * 1024),
    timeoutSeconds: Number(configuration.timeout_seconds ?? 30), retryAttempts: Number(configuration.retry_attempts ?? 2),
  };
}

function protoKind(kind: BackendDataSource["kind"]) {
  return kind === "upload" ? DataSourceKind.UPLOAD : kind === "rest" ? DataSourceKind.REST : DataSourceKind.GRAPHQL;
}

export async function renameWorkspaceDataSource(source: BackendDataSource, name: string) {
  await dataSources.save({ dataSource: { id: source.id, workspaceId: DEV_WORKSPACE_ID, name, kind: protoKind(source.kind), configurationJson: source.configurationJson } });
}

export async function getWorkspaceDataSourceUsage(id: string): Promise<BackendDataSourceUsage[]> {
  const response = await dataSources.getUsage({ id });
  return response.usages.map((usage) => ({ ontologyId: usage.ontologyId, ontologyName: usage.ontologyName, mappingId: usage.mappingId, mappingName: usage.mappingName }));
}

export async function deleteWorkspaceDataSource(id: string) {
  await dataSources.delete({ id });
}

export async function previewWorkspaceSource(id: string) {
  const response = await dataSources.preview({ id });
  if (!response.dataSource) throw new Error("The backend did not return the requested data source.");
  let fileName = response.dataSource.name;
  if (response.dataSource.kind === DataSourceKind.UPLOAD) {
    try {
      const configuration = JSON.parse(response.dataSource.configurationJson) as { file_name?: string };
      fileName = configuration.file_name || fileName;
    } catch {
      // Use the source name when an old upload configuration has no file name.
    }
  } else {
    fileName = `${response.dataSource.kind === DataSourceKind.GRAPHQL ? "GraphQL" : "REST"} · ${response.dataSource.name}.json`;
  }
  return {
    id: response.dataSource.id,
    fileName,
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

export function ontologyDefinition(name: string, slug: string, catalog: OntologyCatalog) {
  const sharedNames = new Set(catalog.sharedProperties.map((property) => property.apiName));
  const propertyDefinition = (property: OntologyCatalog["objectTypes"][number]["properties"][number]) => ({
    api_name: property.apiName,
    display_name: property.displayName,
    value_type: { scalar: scalarType(property.type), list: property.type === "List" },
    required: !!property.required || !!property.identity,
    unique: !!property.unique || !!property.identity,
    identity: !!property.identity,
    indexed: !!property.indexed || !!property.identity,
    description: property.description || null,
  });
  const typeReference = (type: string, reference?: string) => type === "ValueType" && reference
    ? { kind: "value_type", api_name: reference, list: false }
    : type === "Struct" && reference
      ? { kind: "struct", api_name: reference, list: false }
      : { kind: "scalar", value_type: { scalar: scalarType(type), list: type === "List" } };
  return {
    api_name: slug,
    display_name: name,
    description: null,
    object_types: catalog.objectTypes.map((objectType) => ({
      api_name: objectType.apiName,
      display_name: objectType.displayName,
      description: objectType.description || null,
      properties: objectType.properties.filter((property) => !property.derived && !(property.shared && sharedNames.has(property.apiName))).map(propertyDefinition),
      shared_properties: [...(objectType.sharedProperties ?? []), ...objectType.properties.filter((property) => property.shared && sharedNames.has(property.apiName)).map((property) => property.apiName)],
      derived_properties: objectType.properties.filter((property) => property.derived).map((property) => ({
        api_name: property.apiName, display_name: property.displayName,
        value_type: typeReference(property.type, property.reference), expression: property.expression ?? "",
        description: property.description || null,
      })),
      implements: objectType.implements ?? [],
    })),
    link_types: catalog.linkTypes.map((link) => ({
      api_name: link.apiName,
      display_name: link.displayName,
      source_type: link.sourceType,
      target_type: link.targetType,
      source_cardinality: link.sourceCardinality ?? "many",
      target_cardinality: link.targetCardinality ?? "many",
      required: !!link.required,
      properties: link.properties.map(propertyDefinition),
      description: link.description || null,
    })),
    interfaces: catalog.interfaces.map((ontologyInterface) => ({
      api_name: ontologyInterface.apiName, display_name: ontologyInterface.displayName,
      description: ontologyInterface.description || null,
      properties: ontologyInterface.properties.filter((property) => !(property.shared && sharedNames.has(property.apiName))).map(propertyDefinition),
      shared_properties: [...ontologyInterface.sharedProperties, ...ontologyInterface.properties.filter((property) => property.shared && sharedNames.has(property.apiName)).map((property) => property.apiName)],
      extends: ontologyInterface.extends,
    })),
    value_types: catalog.valueTypes.map((valueType) => ({
      api_name: valueType.apiName, display_name: valueType.displayName,
      description: valueType.description || null, base_type: scalarType(valueType.baseType),
      unit: null, minimum: null, maximum: null, pattern: null,
    })),
    struct_types: catalog.structTypes.map((structType) => ({
      api_name: structType.apiName, display_name: structType.displayName,
      description: structType.description || null,
      fields: structType.fields.map((field) => ({
        api_name: field.apiName, display_name: field.displayName,
        value_type: typeReference(field.type, field.reference), required: !!field.required,
        description: field.description || null,
      })),
    })),
    shared_properties: catalog.sharedProperties.map((property) => ({
      api_name: property.apiName, display_name: property.displayName,
      value_type: typeReference(property.type, property.reference), description: property.description || null,
      required: !!property.required, indexed: !!property.indexed,
    })),
    functions: catalog.functions.map((ontologyFunction) => ({
      api_name: ontologyFunction.apiName,
      display_name: ontologyFunction.displayName,
      description: ontologyFunction.description || null,
      inputs: ontologyFunction.inputs.map((input) => ({
        api_name: input.apiName,
        display_name: input.displayName,
        value_type: typeReference(input.type, input.reference),
        required: !!input.required,
      })),
      output: typeReference(ontologyFunction.output, ontologyFunction.outputReference),
      implementation: ontologyFunction.implementation === "external_grpc"
        ? { kind: "external_grpc", endpoint: ontologyFunction.endpoint, method: ontologyFunction.method }
        : ontologyFunction.implementation === "wasm"
          ? { kind: "wasm", artifact_uri: ontologyFunction.artifactUri, entrypoint: ontologyFunction.entrypoint }
          : { kind: "expression", expression: ontologyFunction.expression },
      read_only: true,
    })),
  };
}

export async function loadOntologyDraft(id: string): Promise<BackendOntologyDraft> {
  const draft = await ontologies.getDraft({ id });
  return { id: draft.id, workspaceId: draft.workspaceId, name: draft.name, slug: draft.slug, revision: Number(draft.revision), definitionJson: draft.definitionJson, layoutJson: draft.layoutJson };
}

export async function saveOntologyCatalogDraft(ontology: Pick<BackendOntology, "id" | "name" | "slug">, catalog: OntologyCatalog, layoutJson: string, expectedRevision: number) {
  const saved = await ontologies.saveDraft({
    draft: {
      id: ontology.id, workspaceId: DEV_WORKSPACE_ID, name: ontology.name, slug: ontology.slug,
      revision: BigInt(expectedRevision), definitionJson: JSON.stringify(ontologyDefinition(ontology.name, ontology.slug, catalog)),
      layoutJson, updatedAt: undefined,
    },
    expectedRevision: BigInt(expectedRevision),
  });
  return Number(saved.revision);
}

export async function publishSavedOntologyDraft(ontologyId: string, expectedRevision: number) {
  return ontologies.publish({ ontologyId, expectedRevision: BigInt(expectedRevision) });
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

export async function executeOntologyFunction(ontologyVersionId: string, functionApiName: string, argumentsJson: string) {
  const response = await functions.execute({
    workspaceId: DEV_WORKSPACE_ID,
    ontologyVersionId,
    functionApiName,
    argumentsJson,
  });
  return { resultJson: response.resultJson, executor: response.executor, durationMillis: Number(response.durationMillis), executionId: response.executionId };
}

export async function uploadFunctionArtifact(file: File): Promise<BackendFunctionArtifact> {
  const artifact = await functions.uploadArtifact({
    workspaceId: DEV_WORKSPACE_ID,
    name: file.name.replace(/\.wasm$/i, "") || file.name,
    fileName: file.name,
    content: new Uint8Array(await file.arrayBuffer()),
  });
  return backendFunctionArtifact(artifact);
}

function backendFunctionArtifact(artifact: Awaited<ReturnType<typeof functions.uploadArtifact>>): BackendFunctionArtifact {
  return {
    id: artifact.id, name: artifact.name, fileName: artifact.fileName, artifactUri: artifact.artifactUri,
    sizeBytes: Number(artifact.sizeBytes), sha256: artifact.sha256, createdAt: protoDate(artifact.createdAt),
  };
}

export async function listFunctionArtifacts(): Promise<BackendFunctionArtifact[]> {
  const response = await functions.listArtifacts({ workspaceId: DEV_WORKSPACE_ID });
  return response.artifacts.map(backendFunctionArtifact);
}

export async function deleteFunctionArtifact(id: string) {
  await functions.deleteArtifact({ workspaceId: DEV_WORKSPACE_ID, id });
}

export async function testExternalFunctionProvider(endpoint: string, method: string, functionApiName: string, argumentsJson: string) {
  const response = await functions.testExternalProvider({ workspaceId: DEV_WORKSPACE_ID, endpoint, method, functionApiName, argumentsJson });
  return { resultJson: response.resultJson, durationMillis: Number(response.durationMillis) };
}

export async function listFunctionExecutions(ontologyVersionId: string, functionApiName: string): Promise<BackendFunctionExecution[]> {
  const response = await functions.listExecutions({ workspaceId: DEV_WORKSPACE_ID, ontologyVersionId, functionApiName, limit: 50 });
  return response.executions.map((execution) => ({
    id: execution.id, ontologyVersionId: execution.ontologyVersionId, functionApiName: execution.functionApiName,
    executor: execution.executor, state: execution.state, argumentsJson: execution.argumentsJson,
    resultJson: execution.resultJson, error: execution.error, durationMillis: Number(execution.durationMillis),
    executedAt: protoDate(execution.executedAt),
  }));
}

export function mappingPlan(objectMapping: BackendObjectMapping, links: BackendLinkMapping[]) {
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
      on_error: field.onError,
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
  const job = await ingestions.start({ dataSourceId, ontologyMappingId: mappingId, ontologyVersionId: versionId });
  return pollIngestionJob(job);
}

async function pollIngestionJob(initial: Awaited<ReturnType<typeof ingestions.start>>) {
  let job = initial;
  for (let attempt = 0; attempt < 300 && (job.state === IngestionState.QUEUED || job.state === IngestionState.RUNNING); attempt += 1) {
    await new Promise((resolve) => window.setTimeout(resolve, 100));
    job = await ingestions.getJob({ id: job.id });
  }
  return job;
}

function protoDate(value: { seconds: bigint; nanos: number } | undefined): Date | null {
  return value ? new Date(Number(value.seconds) * 1_000 + value.nanos / 1_000_000) : null;
}

function backendJob(job: Awaited<ReturnType<typeof ingestions.getJob>>): BackendIngestionJob {
  return {
    id: job.id, dataSourceId: job.dataSourceId, state: job.state, rowsRead: Number(job.rowsRead),
    nodesWritten: Number(job.nodesWritten), edgesWritten: Number(job.edgesWritten), rowsRejected: Number(job.rowsRejected),
    error: job.error, ontologyMappingId: job.ontologyMappingId, ontologyVersionId: job.ontologyVersionId,
    createdAt: protoDate(job.createdAt), startedAt: protoDate(job.startedAt), completedAt: protoDate(job.completedAt),
  };
}

export async function listOntologyIngestionJobs(ontologyId: string): Promise<BackendIngestionJob[]> {
  const response = await ingestions.listJobs({ workspaceId: DEV_WORKSPACE_ID, ontologyId, limit: 100 });
  return response.jobs.map(backendJob);
}

export async function retryIngestionJob(id: string): Promise<BackendIngestionJob> {
  return backendJob(await pollIngestionJob(await ingestions.retry({ id })));
}

export async function listIngestionEvents(jobId: string): Promise<BackendIngestionEvent[]> {
  const response = await ingestions.listEvents({ jobId, limit: 500 });
  return response.events.map((event) => ({
    eventType: event.eventType, rowNumber: Number(event.rowNumber), objectType: event.objectType,
    externalId: event.externalId, message: event.message, detailsJson: event.detailsJson,
    occurredAt: protoDate(event.occurredAt),
  }));
}

export async function getObjectProvenance(ontologyVersionId: string, objectType: string, id: string): Promise<BackendPropertyProvenance[]> {
  const response = await graph.getObjectProvenance({ workspaceId: DEV_WORKSPACE_ID, ontologyVersionId, objectType, id });
  return response.properties.map((property) => ({
    property: property.property, dataSourceId: property.dataSourceId, dataSourceName: property.dataSourceName,
    sourceField: property.sourceField, ontologyMappingId: property.ontologyMappingId,
    ontologyMappingName: property.ontologyMappingName, ingestionJobId: property.ingestionJobId,
    importedAt: protoDate(property.importedAt),
  }));
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

const filterOperators: Record<GraphQuerySpec["filters"][number]["operator"], FilterOperator> = {
  equal: FilterOperator.EQUAL,
  not_equal: FilterOperator.NOT_EQUAL,
  contains: FilterOperator.CONTAINS,
  greater_than: FilterOperator.GREATER_THAN,
  less_than: FilterOperator.LESS_THAN,
};

const aggregationFunctions: Record<GraphQuerySpec["aggregations"][number]["function"], AggregationFunction> = {
  count: AggregationFunction.COUNT,
  distinct_count: AggregationFunction.DISTINCT_COUNT,
  sum: AggregationFunction.SUM,
  average: AggregationFunction.AVERAGE,
  minimum: AggregationFunction.MINIMUM,
  maximum: AggregationFunction.MAXIMUM,
};

function importedGraphFromResponse(
  response: Awaited<ReturnType<typeof graph.query>>,
  ontology: BackendOntology,
  catalog: OntologyCatalog,
): ImportedGraph {
  const nodes = response.nodes.map((node) => {
    const objectType = catalog.objectTypes.find((type) => type.apiName === node.objectType);
    const properties = JSON.parse(node.propertiesJson || "{}") as Record<string, GraphValue>;
    const colorIndex = Math.max(0, catalog.objectTypes.findIndex((type) => type.apiName === node.objectType));
    return {
      id: node.id,
      name: String(properties.name ?? properties.id ?? node.id),
      kind: objectType?.displayName ?? node.objectType,
      group: objectType?.displayName ?? node.objectType,
      color: graphColors[colorIndex % graphColors.length],
      properties,
    };
  });
  return {
    nodes,
    links: response.edges.map((edge) => ({
      source: edge.sourceId,
      target: edge.targetId,
      label: edge.linkType,
      properties: JSON.parse(edge.propertiesJson || "{}") as Record<string, GraphValue>,
    })),
    sourceName: `${ontology.name} · Query`,
    importedAt: new Date().toISOString(),
    recordCount: nodes.length,
    skippedCount: 0,
    linkErrorCount: 0,
    ontologyBindings: {
      objectTypes: catalog.objectTypes.map((type) => type.apiName),
      linkTypes: catalog.linkTypes.map((type) => type.apiName),
    },
    aggregations: response.aggregationResults.map((result) => ({ alias: result.alias, value: JSON.parse(result.valueJson) as GraphValue })),
  };
}

export async function queryPersistedGraph(
  ontology: BackendOntology,
  catalog: OntologyCatalog,
  spec: GraphQuerySpec,
): Promise<ImportedGraph> {
  if (!ontology.activeVersionId) throw new Error("Publish the ontology before querying its graph.");
  const response = await graph.query({
    workspaceId: DEV_WORKSPACE_ID,
    ontologyVersionId: ontology.activeVersionId,
    rootType: spec.rootType,
    filters: spec.filters.map((filter) => ({ property: filter.property, operator: filterOperators[filter.operator], values: [filter.value] })),
    traversal: spec.traversal,
    projection: spec.projection,
    limit: spec.limit,
    cursor: "",
    sort: spec.sort ? { property: spec.sort.property, direction: spec.sort.direction === "descending" ? SortDirection.DESCENDING : SortDirection.ASCENDING } : undefined,
    aggregations: spec.aggregations.map((aggregation) => ({ property: aggregation.property, function: aggregationFunctions[aggregation.function], alias: aggregation.alias })),
  });
  return importedGraphFromResponse(response, ontology, catalog);
}

export async function expandPersistedGraphNode(
  ontology: BackendOntology,
  catalog: OntologyCatalog,
  objectType: string,
  objectId: string,
  linkTypes: string[],
): Promise<ImportedGraph> {
  if (!ontology.activeVersionId) throw new Error("Publish the ontology before expanding graph objects.");
  const response = await graph.expand({
    workspaceId: DEV_WORKSPACE_ID,
    ontologyVersionId: ontology.activeVersionId,
    objectType,
    objectId,
    linkTypes,
    limit: 250,
  });
  return importedGraphFromResponse(response, ontology, catalog);
}
