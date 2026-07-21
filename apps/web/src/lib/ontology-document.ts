export type StoredOntologyProperty = {
  name: string;
  type: string;
  identity?: boolean;
  shared?: boolean;
  derived?: boolean;
  expression?: string;
  required?: boolean;
  indexed?: boolean;
  unique?: boolean;
  reference?: string;
  description?: string;
};

export type StoredOntologyNodeData = {
  kind: "object" | "interface" | "value_type" | "struct" | "shared_property" | "function";
  displayName: string;
  apiName: string;
  description: string;
  properties: StoredOntologyProperty[];
  functionOutput?: string;
  functionOutputReference?: string;
  implementation?: string;
  functionExpression?: string;
  endpoint?: string;
  method?: string;
  artifactUri?: string;
  entrypoint?: string;
};

export type StoredOntologyNode = {
  id: string;
  type: "ontology";
  position: { x: number; y: number };
  data: StoredOntologyNodeData;
};

export type StoredOntologyEdge = {
  id: string;
  source: string;
  target: string;
  label: string;
  type: "smoothstep";
  animated?: boolean;
  style?: { strokeDasharray: string };
  data: {
    apiName: string;
    displayName: string;
    description: string;
    sourceCardinality: "one" | "many";
    targetCardinality: "one" | "many";
    required?: boolean;
    properties: StoredOntologyProperty[];
  };
};

type JsonObject = Record<string, unknown>;

const scalarLabels: Record<string, string> = {
  string: "String", boolean: "Boolean", int64: "Int64", float64: "Float64", decimal: "Decimal",
  date: "Date", timestamp: "Timestamp", uuid: "UUID", enum: "Enum", json: "JSON",
};

function object(value: unknown): JsonObject {
  return value && typeof value === "object" && !Array.isArray(value) ? value as JsonObject : {};
}

function array(value: unknown): JsonObject[] {
  return Array.isArray(value) ? value.map(object) : [];
}

function typeLabel(value: unknown): { type: string; reference?: string } {
  const reference = object(value);
  if (reference.kind === "value_type") return { type: "ValueType", reference: String(reference.api_name ?? "") };
  if (reference.kind === "struct") return { type: "Struct", reference: String(reference.api_name ?? "") };
  const valueType = reference.kind === "scalar" ? object(reference.value_type) : reference;
  const scalar = valueType.scalar;
  const scalarName = typeof scalar === "string" ? scalar : Object.keys(object(scalar))[0] ?? "string";
  return { type: valueType.list ? "List" : scalarLabels[scalarName] ?? "String" };
}

function property(value: JsonObject): StoredOntologyProperty {
  const valueType = typeLabel(value.value_type);
  return {
    name: String(value.api_name ?? "property"), type: valueType.type, reference: valueType.reference,
    identity: Boolean(value.identity), required: Boolean(value.required), indexed: Boolean(value.indexed),
    unique: Boolean(value.unique), description: String(value.description ?? ""),
  };
}

function position(index: number) {
  return { x: 70 + (index % 3) * 330, y: 70 + Math.floor(index / 3) * 230 };
}

export function ontologyDocumentFromBackend(layoutJson: string, definitionJson: string): { nodes: StoredOntologyNode[]; edges: StoredOntologyEdge[] } {
  try {
    const layout = JSON.parse(layoutJson) as { nodes?: StoredOntologyNode[]; edges?: StoredOntologyEdge[] };
    if (Array.isArray(layout.nodes) && Array.isArray(layout.edges)) return { nodes: layout.nodes, edges: layout.edges };
  } catch {
    // Reconstruct a canvas from the definition when no compatible layout has been saved yet.
  }

  const definition = object(JSON.parse(definitionJson));
  const nodes: StoredOntologyNode[] = [];
  const edges: StoredOntologyEdge[] = [];
  const ids = new Map<string, string>();
  const shared = new Map(array(definition.shared_properties).map((entry) => [String(entry.api_name), entry]));

  const addNode = (kind: StoredOntologyNodeData["kind"], entry: JsonObject, properties: StoredOntologyProperty[], extra: Partial<StoredOntologyNodeData> = {}) => {
    const apiName = String(entry.api_name ?? `${kind}_${nodes.length + 1}`);
    const id = `${kind}:${apiName}`;
    ids.set(apiName, id);
    nodes.push({ id, type: "ontology", position: position(nodes.length), data: {
      kind, apiName, displayName: String(entry.display_name ?? apiName), description: String(entry.description ?? ""), properties, ...extra,
    } });
  };

  for (const entry of array(definition.object_types)) {
    const properties = array(entry.properties).map(property);
    for (const derived of array(entry.derived_properties)) {
      const valueType = typeLabel(derived.value_type);
      properties.push({ name: String(derived.api_name), type: valueType.type, reference: valueType.reference, derived: true, expression: String(derived.expression ?? ""), description: String(derived.description ?? "") });
    }
    for (const sharedName of Array.isArray(entry.shared_properties) ? entry.shared_properties : []) {
      const sharedDefinition = shared.get(String(sharedName));
      const valueType = typeLabel(sharedDefinition?.value_type);
      properties.push({ name: String(sharedName), type: valueType.type, reference: valueType.reference, shared: true });
    }
    addNode("object", entry, properties);
  }
  for (const entry of array(definition.interfaces)) {
    const properties = array(entry.properties).map(property);
    for (const sharedName of Array.isArray(entry.shared_properties) ? entry.shared_properties : []) {
      const valueType = typeLabel(shared.get(String(sharedName))?.value_type);
      properties.push({ name: String(sharedName), type: valueType.type, reference: valueType.reference, shared: true });
    }
    addNode("interface", entry, properties);
  }
  for (const entry of array(definition.value_types)) addNode("value_type", entry, [{ name: "base_type", type: typeLabel({ scalar: entry.base_type, list: false }).type }]);
  for (const entry of array(definition.struct_types)) addNode("struct", entry, array(entry.fields).map(property));
  for (const entry of array(definition.shared_properties)) {
    const valueType = typeLabel(entry.value_type);
    addNode("shared_property", entry, [{ name: "value", type: valueType.type, reference: valueType.reference, shared: true, required: Boolean(entry.required), indexed: Boolean(entry.indexed) }]);
  }
  for (const entry of array(definition.functions)) {
    const output = typeLabel(entry.output);
    const implementation = object(entry.implementation);
    addNode("function", entry, array(entry.inputs).map(property), {
      functionOutput: output.type, functionOutputReference: output.reference,
      implementation: String(implementation.kind ?? "expression"),
      functionExpression: String(implementation.expression ?? ""), endpoint: String(implementation.endpoint ?? ""),
      method: String(implementation.method ?? "invoke"), artifactUri: String(implementation.artifact_uri ?? ""),
      entrypoint: String(implementation.entrypoint ?? "run"),
    });
  }

  const addSemanticEdge = (sourceName: unknown, targetName: unknown, apiName: "implements" | "extends") => {
    const source = ids.get(String(sourceName)); const target = ids.get(String(targetName));
    if (!source || !target) return;
    edges.push({ id: `${apiName}:${sourceName}:${targetName}`, source, target, label: apiName, type: "smoothstep", style: { strokeDasharray: "5 5" }, data: { apiName, displayName: apiName === "implements" ? "Implements" : "Extends", description: "", sourceCardinality: "many", targetCardinality: "many", properties: [] } });
  };
  for (const entry of array(definition.object_types)) for (const target of Array.isArray(entry.implements) ? entry.implements : []) addSemanticEdge(entry.api_name, target, "implements");
  for (const entry of array(definition.interfaces)) for (const target of Array.isArray(entry.extends) ? entry.extends : []) addSemanticEdge(entry.api_name, target, "extends");
  for (const entry of array(definition.link_types)) {
    const source = ids.get(String(entry.source_type)); const target = ids.get(String(entry.target_type));
    if (!source || !target) continue;
    const apiName = String(entry.api_name);
    edges.push({ id: `link:${apiName}`, source, target, label: apiName, type: "smoothstep", data: {
      apiName, displayName: String(entry.display_name ?? apiName), description: String(entry.description ?? ""),
      sourceCardinality: entry.source_cardinality === "one" ? "one" : "many",
      targetCardinality: entry.target_cardinality === "one" ? "one" : "many",
      required: Boolean(entry.required),
      properties: array(entry.properties).map(property),
    } });
  }
  return { nodes, edges };
}
