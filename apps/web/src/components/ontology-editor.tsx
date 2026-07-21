"use client";

import { useCallback, useEffect, useMemo, useRef, useState, useSyncExternalStore } from "react";
import {
  Background, BackgroundVariant, Controls, Handle, MiniMap, Position, ReactFlow, addEdge,
  useEdgesState, useNodesState, type Connection, type Edge, type Node, type NodeProps,
} from "@xyflow/react";
import { Braces, Check, CircleDot, Code2, Component, GitFork, Link2, Plus, Redo2, Save, Send, Share2, Trash2, Undo2, X } from "lucide-react";
import { executeOntologyFunction, loadOntologyDraft, publishSavedOntologyDraft, saveOntologyCatalogDraft } from "@/lib/context-hub-client";
import type { OntologyCatalog } from "@/lib/ontology-catalog";
import { ontologyDocumentFromBackend } from "@/lib/ontology-document";

type Property = { name: string; type: string; description?: string; identity?: boolean; shared?: boolean; derived?: boolean; expression?: string; required?: boolean; indexed?: boolean; unique?: boolean; reference?: string };
type NodeKind = "object" | "interface" | "value_type" | "struct" | "shared_property" | "function";
type OntologyNodeData = {
  [key: string]: unknown;
  kind: NodeKind;
  displayName: string;
  apiName: string;
  description: string;
  properties: Property[];
  functionOutput?: string;
  functionOutputReference?: string;
  implementation?: string;
  functionExpression?: string;
  endpoint?: string;
  method?: string;
  artifactUri?: string;
  entrypoint?: string;
};
type LinkData = {
  [key: string]: unknown;
  apiName: string;
  displayName: string;
  description: string;
  sourceCardinality: "one" | "many";
  targetCardinality: "one" | "many";
  required?: boolean;
  properties: Property[];
};
type OntologyNode = Node<OntologyNodeData, "ontology">;
type OntologyEdge = Edge<LinkData>;
type EditorSnapshot = { nodes: OntologyNode[]; edges: OntologyEdge[] };

function cloneSnapshot(snapshot: EditorSnapshot): EditorSnapshot {
  return JSON.parse(JSON.stringify(snapshot)) as EditorSnapshot;
}

const initialNodes: OntologyNode[] = [
  { id: "service", type: "ontology", position: { x: 80, y: 120 }, data: { kind: "object", displayName: "Service", apiName: "service", description: "A deployable software service", properties: [{ name: "id", type: "String", identity: true }, { name: "name", type: "String" }, { name: "display_label", type: "String", derived: true, expression: "concat(name, ' · ', id)" }] } },
  { id: "team", type: "ontology", position: { x: 490, y: 290 }, data: { kind: "object", displayName: "Team", apiName: "team", description: "A responsible team", properties: [{ name: "id", type: "String", identity: true }, { name: "name", type: "String" }] } },
  { id: "deployable", type: "ontology", position: { x: 500, y: 55 }, data: { kind: "interface", displayName: "Deployable", apiName: "deployable", description: "Common fields of deployable objects", properties: [{ name: "environment", type: "String", shared: true }] } },
];

const initialEdges: OntologyEdge[] = [
  { id: "owned-by", source: "service", target: "team", label: "owned_by", type: "smoothstep", animated: true, data: { apiName: "owned_by", displayName: "Owned by", description: "The team responsible for a service", sourceCardinality: "many", targetCardinality: "one", properties: [{ name: "since", type: "Date" }] } },
  { id: "depends-on", source: "service", target: "service", label: "depends_on", type: "smoothstep", data: { apiName: "depends_on", displayName: "Depends on", description: "A dependency between services", sourceCardinality: "many", targetCardinality: "many", properties: [] } },
  { id: "implements", source: "service", target: "deployable", label: "implements", type: "smoothstep", style: { strokeDasharray: "5 5" }, data: { apiName: "implements", displayName: "Implements", description: "Interface implementation", sourceCardinality: "many", targetCardinality: "many", properties: [] } },
];

function initialDocument(ontologyId: string, seedTemplate: boolean) {
  if (typeof window !== "undefined") {
    try {
      const saved = localStorage.getItem(`context-hub.ontology.${ontologyId}`);
      if (saved) {
        const parsed = JSON.parse(saved) as { nodes?: OntologyNode[]; edges?: OntologyEdge[] };
        if (Array.isArray(parsed.nodes) && Array.isArray(parsed.edges)) return { nodes: parsed.nodes, edges: parsed.edges };
      }
    } catch {
      // Start with a fresh document if this ontology's local draft cannot be read.
    }
  }
  return seedTemplate
    ? { nodes: JSON.parse(JSON.stringify(initialNodes)) as OntologyNode[], edges: JSON.parse(JSON.stringify(initialEdges)) as OntologyEdge[] }
    : { nodes: [] as OntologyNode[], edges: [] as OntologyEdge[] };
}

const subscribeToHydration = (onStoreChange: () => void) => {
  queueMicrotask(onStoreChange);
  return () => undefined;
};
const getClientHydrationSnapshot = () => true;
const getServerHydrationSnapshot = () => false;

const kindMeta: Record<NodeKind, { label: string; icon: typeof Braces }> = {
  object: { label: "Object type", icon: Braces }, interface: { label: "Interface", icon: GitFork },
  value_type: { label: "Value type", icon: CircleDot }, struct: { label: "Struct", icon: Component },
  shared_property: { label: "Shared property", icon: Share2 }, function: { label: "Function", icon: Code2 },
};

function OntologyCard({ data, selected }: NodeProps<OntologyNode>) {
  const MetaIcon = kindMeta[data.kind].icon;
  return <div className={selected ? `ontology-node ${data.kind} selected` : `ontology-node ${data.kind}`}>
    <Handle type="target" position={Position.Left}/><div className="node-heading"><span className="node-icon"><MetaIcon size={15}/></span><div><strong>{data.displayName}</strong><small>{data.apiName} · {kindMeta[data.kind].label}</small></div></div>
    <div className="property-list">{data.properties.map((property) => <div key={property.name}><span>{property.identity && <CircleDot size={10}/>} {property.derived && <Code2 size={10}/>} {property.shared && <Share2 size={10}/>} {property.name}</span><code>{property.type}</code></div>)}{data.kind === "function" && <div><span>returns</span><code>{data.functionOutput}</code></div>}</div><Handle type="source" position={Position.Right}/>
  </div>;
}

function PropertyFlags({ kind, property, onChange }: { kind: NodeKind | "link"; property: Property; onChange: (patch: Partial<Property>) => void }) {
  if (kind === "value_type") return null;
  const supportsSchemaFlags = kind === "object" || kind === "interface" || kind === "link";
  return <div className="property-flags">
    {kind === "object" && <label><input type="checkbox" checked={!!property.identity} onChange={(event) => onChange({ identity: event.target.checked })}/> Identity</label>}
    <label><input type="checkbox" checked={!!property.required} onChange={(event) => onChange({ required: event.target.checked })}/> Required</label>
    {supportsSchemaFlags && <label><input type="checkbox" checked={!!property.unique} onChange={(event) => onChange({ unique: event.target.checked })}/> Unique</label>}
    {(supportsSchemaFlags || kind === "shared_property") && <label><input type="checkbox" checked={!!property.indexed} onChange={(event) => onChange({ indexed: event.target.checked })}/> Indexed</label>}
    {(kind === "object" || kind === "interface") && <label><input type="checkbox" checked={!!property.shared} onChange={(event) => onChange({ shared: event.target.checked })}/> Shared</label>}
    {kind === "object" && <label><input type="checkbox" checked={!!property.derived} onChange={(event) => onChange({ derived: event.target.checked })}/> Derived</label>}
  </div>;
}

type OntologyEditorProps = { ontologyId: string; ontologyName: string; ontologySlug?: string; seedTemplate: boolean; onRename: (name: string) => void; onCatalogChange?: (catalog: OntologyCatalog) => void; onPublished?: (versionId: string) => void };

export function OntologyEditor(props: OntologyEditorProps) {
  const hydrated = useSyncExternalStore(subscribeToHydration, getClientHydrationSnapshot, getServerHydrationSnapshot);
  return hydrated ? <HydratedOntologyEditor {...props}/> : <div className="workspace-view" aria-label="Loading ontology editor"/>;
}

function HydratedOntologyEditor({ ontologyId, ontologyName, ontologySlug = ontologyId, seedTemplate, onRename, onCatalogChange, onPublished }: OntologyEditorProps) {
  const document = useMemo(() => initialDocument(ontologyId, seedTemplate), [ontologyId, seedTemplate]);
  const [nodes, setNodes, onNodesChange] = useNodesState<OntologyNode>(document.nodes);
  const [edges, setEdges, onEdgesChange] = useEdgesState<OntologyEdge>(document.edges);
  const [selectedId, setSelectedId] = useState<string>(document.nodes[0]?.id ?? "");
  const [selectedEdgeId, setSelectedEdgeId] = useState<string | null>(null);
  const [saveState, setSaveState] = useState("Autosave on");
  const [notice, setNotice] = useState<string | null>(null);
  const [publishedVersionId, setPublishedVersionId] = useState("");
  const [functionArguments, setFunctionArguments] = useState("{}");
  const [functionResult, setFunctionResult] = useState("");
  const [functionBusy, setFunctionBusy] = useState(false);
  const [draftReady, setDraftReady] = useState(false);
  const draftRevision = useRef(0);
  const saveChain = useRef<Promise<number>>(Promise.resolve(0));
  const lastSavedSnapshot = useRef("");
  const lastSavedDocument = useRef("");
  const [history, setHistory] = useState<EditorSnapshot[]>([cloneSnapshot(document)]);
  const [historyIndex, setHistoryIndex] = useState(0);
  const nodeTypes = useMemo(() => ({ ontology: OntologyCard }), []);
  const selected = nodes.find((node) => node.id === selectedId);
  const selectedEdge = edges.find((edge) => edge.id === selectedEdgeId);

  useEffect(() => {
    if (!draftReady) return;
    const timeout = window.setTimeout(() => {
      const current = JSON.stringify({ nodes, edges });
      if (JSON.stringify(history[historyIndex]) === current) return;
      setHistory((items) => [...items.slice(0, historyIndex + 1), cloneSnapshot({ nodes, edges })].slice(-50));
      setHistoryIndex((index) => Math.min(index + 1, 49));
    }, 250);
    return () => window.clearTimeout(timeout);
  }, [nodes, edges, draftReady, history, historyIndex]);

  function moveHistory(offset: -1 | 1) {
    const nextIndex = historyIndex + offset;
    const snapshot = history[nextIndex];
    if (!snapshot) return;
    const clone = cloneSnapshot(snapshot);
    setNodes(clone.nodes); setEdges(clone.edges); setHistoryIndex(nextIndex);
    setSelectedId(""); setSelectedEdgeId(null); setPublishedVersionId("");
  }

  const catalog = useMemo<OntologyCatalog>(() => {
    const objectNodes = nodes.filter((node) => node.data.kind === "object");
    const interfaceNodes = nodes.filter((node) => node.data.kind === "interface");
    const byId = new Map(nodes.map((node) => [node.id, node]));
    const propertyCatalog = (property: Property) => ({ apiName: property.name, displayName: property.name, type: property.type, description: property.description, identity: property.identity, shared: property.shared, derived: property.derived, expression: property.expression, required: property.required, indexed: property.indexed, unique: property.unique, reference: property.reference });
    const isImplements = (edge: OntologyEdge) => byId.get(edge.source)?.data.kind === "object" && byId.get(edge.target)?.data.kind === "interface" && edge.data?.apiName === "implements";
    const isExtends = (edge: OntologyEdge) => byId.get(edge.source)?.data.kind === "interface" && byId.get(edge.target)?.data.kind === "interface" && edge.data?.apiName === "extends";
    return {
      objectTypes: objectNodes.map((node) => ({
        apiName: node.data.apiName,
        displayName: node.data.displayName,
        description: node.data.description,
        properties: node.data.properties.map(propertyCatalog),
        implements: edges.filter((edge) => edge.source === node.id && isImplements(edge)).map((edge) => byId.get(edge.target)!.data.apiName),
      })),
      linkTypes: edges.filter((edge) => edge.data && byId.has(edge.source) && byId.has(edge.target) && !isImplements(edge) && !isExtends(edge) && ["object", "interface"].includes(byId.get(edge.source)!.data.kind) && ["object", "interface"].includes(byId.get(edge.target)!.data.kind)).map((edge) => ({
        apiName: edge.data!.apiName,
        displayName: edge.data!.displayName,
        sourceType: byId.get(edge.source)!.data.apiName,
        targetType: byId.get(edge.target)!.data.apiName,
        description: edge.data!.description,
        sourceCardinality: edge.data!.sourceCardinality,
        targetCardinality: edge.data!.targetCardinality,
        required: edge.data!.required,
        properties: edge.data!.properties.map(propertyCatalog),
      })),
      interfaces: interfaceNodes.map((node) => ({ apiName: node.data.apiName, displayName: node.data.displayName, description: node.data.description, properties: node.data.properties.map(propertyCatalog), sharedProperties: [], extends: edges.filter((edge) => edge.source === node.id && isExtends(edge)).map((edge) => byId.get(edge.target)!.data.apiName) })),
      valueTypes: nodes.filter((node) => node.data.kind === "value_type").map((node) => ({ apiName: node.data.apiName, displayName: node.data.displayName, description: node.data.description, baseType: node.data.properties[0]?.type ?? "String" })),
      structTypes: nodes.filter((node) => node.data.kind === "struct").map((node) => ({ apiName: node.data.apiName, displayName: node.data.displayName, description: node.data.description, fields: node.data.properties.map(propertyCatalog) })),
      sharedProperties: nodes.filter((node) => node.data.kind === "shared_property").map((node) => ({ apiName: node.data.apiName, displayName: node.data.displayName, description: node.data.description, type: node.data.properties[0]?.type ?? "String", required: node.data.properties[0]?.required, indexed: node.data.properties[0]?.indexed, reference: node.data.properties[0]?.reference })),
      functions: nodes.filter((node) => node.data.kind === "function").map((node) => ({
        apiName: node.data.apiName,
        displayName: node.data.displayName,
        description: node.data.description,
        inputs: node.data.properties.map(propertyCatalog),
        output: node.data.functionOutput ?? "String",
        outputReference: node.data.functionOutputReference,
        implementation: (node.data.implementation ?? "expression") as "expression" | "external_grpc" | "wasm",
        expression: node.data.functionExpression ?? "",
        endpoint: node.data.endpoint ?? "",
        method: node.data.method ?? "invoke",
        artifactUri: node.data.artifactUri ?? "",
        entrypoint: node.data.entrypoint ?? "run",
      })),
    };
  }, [edges, nodes]);

  const connect = useCallback((connection: Connection) => {
    const number = edges.length + 1;
    const data: LinkData = { apiName: `link_${number}`, displayName: `Link ${number}`, description: "", sourceCardinality: "many", targetCardinality: "many", properties: [] };
    const edge: OntologyEdge = { ...connection, id: crypto.randomUUID(), type: "smoothstep", label: data.apiName, data };
    setEdges((items) => addEdge(edge, items)); setSelectedId(""); setSelectedEdgeId(edge.id); setPublishedVersionId("");
  }, [edges.length, setEdges]);

  useEffect(() => {
    let cancelled = false;
    void loadOntologyDraft(ontologyId).then((draft) => {
      if (cancelled) return;
      draftRevision.current = draft.revision;
      const hasLocalDraft = localStorage.getItem(`context-hub.ontology.${ontologyId}`) !== null;
      let loadedDocument: EditorSnapshot = document;
      if (!hasLocalDraft) {
        const backendDocument = ontologyDocumentFromBackend(draft.layoutJson, draft.definitionJson);
        loadedDocument = { nodes: backendDocument.nodes as OntologyNode[], edges: backendDocument.edges as OntologyEdge[] };
        lastSavedDocument.current = JSON.stringify({ nodes: loadedDocument.nodes, edges: loadedDocument.edges, ontologyName: draft.name, ontologySlug: draft.slug });
        setNodes(loadedDocument.nodes);
        setEdges(loadedDocument.edges);
        setSelectedId(backendDocument.nodes[0]?.id ?? "");
      }
      setHistory([cloneSnapshot(loadedDocument)]);
      setHistoryIndex(0);
      setDraftReady(true);
      setSaveState(hasLocalDraft ? "Migrating local draft…" : `Backend draft r${draft.revision}`);
    }).catch((error) => {
      if (cancelled) return;
      setSaveState("Backend unavailable");
      setNotice(error instanceof Error ? error.message : "The ontology draft could not be loaded.");
    });
    return () => { cancelled = true; };
  }, [document, ontologyId, setEdges, setNodes]);

  const saveDraftSnapshot = useCallback((snapshotCatalog: OntologyCatalog, snapshotNodes: OntologyNode[], snapshotEdges: OntologyEdge[]) => {
    const layoutJson = JSON.stringify({ nodes: snapshotNodes, edges: snapshotEdges });
    const documentFingerprint = JSON.stringify({ nodes: snapshotNodes, edges: snapshotEdges, ontologyName, ontologySlug });
    const snapshot = JSON.stringify({ catalog: snapshotCatalog, layoutJson, ontologyName, ontologySlug });
    const operation = saveChain.current.catch(() => draftRevision.current).then(async () => {
      if (snapshot === lastSavedSnapshot.current) return draftRevision.current;
      setSaveState("Saving backend draft…");
      const revision = await saveOntologyCatalogDraft({ id: ontologyId, name: ontologyName, slug: ontologySlug }, snapshotCatalog, layoutJson, draftRevision.current);
      draftRevision.current = revision;
      lastSavedSnapshot.current = snapshot;
      lastSavedDocument.current = documentFingerprint;
      setSaveState(`Saved · backend r${revision}`);
      localStorage.removeItem(`context-hub.ontology.${ontologyId}`);
      return revision;
    });
    saveChain.current = operation;
    operation.catch((error) => {
      setSaveState("Autosave failed");
      setNotice(error instanceof Error ? error.message : "The backend draft could not be saved.");
    });
    return operation;
  }, [ontologyId, ontologyName, ontologySlug]);

  useEffect(() => {
    onCatalogChange?.(catalog);
    if (!draftReady) return;
    const documentFingerprint = JSON.stringify({ nodes, edges, ontologyName, ontologySlug });
    if (documentFingerprint === lastSavedDocument.current) return;
    localStorage.setItem(`context-hub.ontology.${ontologyId}`, JSON.stringify({ nodes, edges }));
    const timeout = window.setTimeout(() => { void saveDraftSnapshot(catalog, nodes, edges); }, 650);
    return () => window.clearTimeout(timeout);
  }, [nodes, edges, catalog, onCatalogChange, ontologyId, ontologyName, ontologySlug, draftReady, saveDraftSnapshot]);

  function addNode(kind: NodeKind) {
    const count = nodes.filter((node) => node.data.kind === kind).length + 1;
    const id = `${kind}-${crypto.randomUUID()}`;
    const label = kindMeta[kind].label;
    const properties: Property[] = kind === "object" ? [{ name: "id", type: "String", identity: true }] : kind === "value_type" ? [{ name: "base_type", type: "String" }] : kind === "shared_property" ? [{ name: "value", type: "String", shared: true }] : [];
    setNodes((items) => [...items, { id, type: "ontology", position: { x: 130 + count * 42, y: 100 + count * 48 }, data: { kind, displayName: `${label} ${count}`, apiName: `${kind}_${count}`, description: "", properties, ...(kind === "function" ? { functionOutput: "String", implementation: "expression", functionExpression: "concat('Hello ', name)", endpoint: "http://localhost:50061", method: "invoke", artifactUri: "object://functions/example.wasm", entrypoint: "run" } : {}) } }]);
    setSelectedEdgeId(null); setSelectedId(id);
    setPublishedVersionId("");
  }

  function updateSelected(field: "displayName" | "apiName" | "description" | "functionOutput" | "functionOutputReference" | "implementation" | "functionExpression" | "endpoint" | "method" | "artifactUri" | "entrypoint", value: string) {
    setNodes((items) => items.map((node) => node.id === selectedId ? { ...node, data: { ...node.data, [field]: value } } : node));
    setPublishedVersionId("");
  }

  function addProperty() {
    if (!selected) return;
    const number = selected.data.properties.length + 1;
    const property: Property = { name: `property_${number}`, type: "String", ...(selected.data.kind === "shared_property" ? { shared: true } : {}) };
    setNodes((items) => items.map((node) => node.id === selected.id ? { ...node, data: { ...node.data, properties: [...node.data.properties, property] } } : node));
    setPublishedVersionId("");
  }

  function updateProperty(index: number, patch: Partial<Property>) {
    setNodes((items) => items.map((node) => node.id === selectedId ? { ...node, data: { ...node.data, properties: node.data.properties.map((property, propertyIndex) => propertyIndex === index ? { ...property, ...patch } : property) } } : node));
    setPublishedVersionId("");
  }

  function deleteProperty(index: number) {
    setNodes((items) => items.map((node) => node.id === selectedId ? { ...node, data: { ...node.data, properties: node.data.properties.filter((_, propertyIndex) => propertyIndex !== index) } } : node));
    setPublishedVersionId("");
  }

  function deleteSelectedNode() {
    if (!selected) return;
    setNodes((items) => items.filter((node) => node.id !== selected.id));
    setEdges((items) => items.filter((edge) => edge.source !== selected.id && edge.target !== selected.id));
    setSelectedId(""); setPublishedVersionId("");
  }

  function updateLink(patch: Partial<LinkData>) {
    setEdges((items) => items.map((edge) => edge.id === selectedEdgeId ? { ...edge, label: patch.apiName ?? edge.data?.apiName, data: { ...(edge.data as LinkData), ...patch } } : edge));
    setPublishedVersionId("");
  }

  function addLinkProperty() {
    if (!selectedEdge?.data) return;
    updateLink({ properties: [...selectedEdge.data.properties, { name: `property_${selectedEdge.data.properties.length + 1}`, type: "String" }] });
  }

  function validationError() {
    const invalidNode = nodes.find((node) => !/^[a-z][a-z0-9_]{0,63}$/.test(node.data.apiName));
    const invalidLink = edges.find((edge) => !edge.data || !/^[a-z][a-z0-9_]{0,63}$/.test(edge.data.apiName));
    const missingIdentity = nodes.find((node) => node.data.kind === "object" && !node.data.properties.some((property) => property.identity));
    const missingReference = nodes.find((node) => node.data.properties.some((property) => (property.type === "ValueType" || property.type === "Struct") && !property.reference?.trim()));
    const incompleteFunction = nodes.find((node) => node.data.kind === "function" && (!node.data.functionOutput || (node.data.implementation === "expression" && !node.data.functionExpression?.trim()) || (node.data.implementation === "external_grpc" && (!node.data.endpoint?.trim() || !node.data.method?.trim())) || (node.data.implementation === "wasm" && (!node.data.artifactUri?.trim() || !node.data.entrypoint?.trim()))));
    const missingOutputReference = nodes.find((node) => node.data.kind === "function" && (node.data.functionOutput === "ValueType" || node.data.functionOutput === "Struct") && !node.data.functionOutputReference?.trim());
    return invalidNode ? `Invalid API name: ${invalidNode.data.apiName}` : invalidLink ? "A link needs a valid API name" : missingIdentity ? `${missingIdentity.data.displayName} needs an identity property` : missingReference ? `${missingReference.data.displayName} has a property without a reusable type API name` : missingOutputReference ? `${missingOutputReference.data.displayName} needs an output API name` : incompleteFunction ? `${incompleteFunction.data.displayName} has an incomplete implementation` : null;
  }

  function validateDraft() {
    setNotice(validationError() ?? "The ontology draft is valid. Functions are read-only; Actions are not part of this version.");
  }

  async function publishDraft() {
    const error = validationError();
    if (error) { setNotice(error); return; }
    setSaveState("Publishing…");
    try {
      const revision = await saveDraftSnapshot(catalog, nodes, edges);
      const version = await publishSavedOntologyDraft(ontologyId, revision);
      setPublishedVersionId(version.id);
      onPublished?.(version.id);
      setSaveState(`Published v${version.version}`);
      setNotice(`Published ontology version ${version.version}. Functions can now be executed.`);
    } catch (error) {
      setSaveState("Publish failed");
      setNotice(error instanceof Error ? error.message : "The ontology could not be published.");
    }
  }

  async function runFunction() {
    if (!selected || selected.data.kind !== "function" || !publishedVersionId) return;
    setFunctionBusy(true); setFunctionResult("");
    try {
      JSON.parse(functionArguments);
      const result = await executeOntologyFunction(publishedVersionId, selected.data.apiName, functionArguments);
      setFunctionResult(`${result.resultJson}\n\n${result.executor} · ${result.durationMillis} ms`);
    } catch (error) {
      setFunctionResult(error instanceof Error ? error.message : "Function execution failed.");
    } finally { setFunctionBusy(false); }
  }

  return <div className="workspace-view"><header className="stage-header"><div><span className="eyebrow">Ontology editor · isolated model</span><input className="ontology-name-input" aria-label="Ontology name" value={ontologyName} onChange={(event) => onRename(event.target.value)}/><p>Object types, links, interfaces, reusable types and read-only functions.</p></div><div className="header-actions"><span className="save-state"><Save size={13}/> {saveState}</span><button className="button secondary icon-button" aria-label="Undo ontology change" disabled={historyIndex === 0} onClick={() => moveHistory(-1)}><Undo2 size={15}/></button><button className="button secondary icon-button" aria-label="Redo ontology change" disabled={historyIndex >= history.length - 1} onClick={() => moveHistory(1)}><Redo2 size={15}/></button><button className="button secondary" onClick={validateDraft}><Check size={15}/> Validate</button><button className="button primary" onClick={() => void publishDraft()}><Send size={15}/> Publish</button></div></header>
    {notice && <div className="notice" role="status">{notice}<button onClick={() => setNotice(null)}>×</button></div>}
    <div className="editor-layout"><div className="canvas-panel"><div className="canvas-toolbar">{(["object", "interface", "value_type", "struct", "shared_property", "function"] as NodeKind[]).map((kind) => <button key={kind} onClick={() => addNode(kind)}><Plus size={14}/> {kindMeta[kind].label}</button>)}</div>
      <ReactFlow nodes={nodes} edges={edges} nodeTypes={nodeTypes} onNodesChange={onNodesChange} onEdgesChange={onEdgesChange} onConnect={connect} onNodeClick={(_, node) => { setSelectedId(node.id); setSelectedEdgeId(null); }} onEdgeClick={(_, edge) => { setSelectedId(""); setSelectedEdgeId(edge.id); }} fitView><Background variant={BackgroundVariant.Dots} gap={18} size={1}/><MiniMap pannable zoomable/><Controls/></ReactFlow></div>
      <aside className="inspector"><span className="eyebrow">Inspector</span>{selected ? <><h2>{kindMeta[selected.data.kind].label}</h2><label>Display name<input value={selected.data.displayName} onChange={(event) => updateSelected("displayName", event.target.value)}/></label><label>API name<input value={selected.data.apiName} onChange={(event) => updateSelected("apiName", event.target.value)}/><small>Stable after first publish</small></label><label>Description<textarea value={selected.data.description} onChange={(event) => updateSelected("description", event.target.value)}/></label>{selected.data.kind === "function" && <><label>Output type<select value={selected.data.functionOutput} onChange={(event) => updateSelected("functionOutput", event.target.value)}>{["String", "Boolean", "Int64", "Float64", "Decimal", "Date", "Timestamp", "UUID", "JSON", "List", "ValueType", "Struct"].map((type) => <option key={type}>{type}</option>)}</select></label>{(selected.data.functionOutput === "ValueType" || selected.data.functionOutput === "Struct") && <label>Output API name<input value={selected.data.functionOutputReference ?? ""} onChange={(event) => updateSelected("functionOutputReference", event.target.value)}/></label>}<label>Implementation<select value={selected.data.implementation} onChange={(event) => updateSelected("implementation", event.target.value)}><option value="expression">Expression</option><option value="external_grpc">External gRPC</option><option value="wasm">WASM</option></select><small>Functions are read-only in V1.</small></label>{selected.data.implementation === "expression" && <label>Controlled expression<textarea aria-label="Controlled expression" value={selected.data.functionExpression ?? ""} onChange={(event) => updateSelected("functionExpression", event.target.value)}/><small>Arguments, arithmetic, comparisons, concat, coalesce, lower, upper, trim and length.</small></label>}{selected.data.implementation === "external_grpc" && <><label>gRPC endpoint<input value={selected.data.endpoint ?? ""} onChange={(event) => updateSelected("endpoint", event.target.value)}/></label><label>Method<input value={selected.data.method ?? "invoke"} onChange={(event) => updateSelected("method", event.target.value)}/><small>Uses the typed ContextHub ExternalFunctionService contract.</small></label></>}{selected.data.implementation === "wasm" && <><label>Artifact URI<input value={selected.data.artifactUri ?? ""} onChange={(event) => updateSelected("artifactUri", event.target.value)}/><small>object:// or s3://context-hub/</small></label><label>Entrypoint<input value={selected.data.entrypoint ?? "run"} onChange={(event) => updateSelected("entrypoint", event.target.value)}/></label></>}</>}
        <div className="inspector-section"><div className="section-title"><span>{selected.data.kind === "function" ? "Inputs" : selected.data.kind === "struct" ? "Fields" : "Properties"}</span><button onClick={addProperty}><Plus size={13}/> Add</button></div>{selected.data.properties.map((property, index) => <div className="property-edit" key={`${property.name}-${index}`}><input value={property.name} onChange={(event) => updateProperty(index, { name: event.target.value })}/><select value={property.type} onChange={(event) => updateProperty(index, { type: event.target.value, reference: undefined })}>{["String", "Boolean", "Int64", "Float64", "Decimal", "Date", "Timestamp", "UUID", "Enum", "JSON", "List", "ValueType", "Struct"].map((type) => <option key={type}>{type}</option>)}</select><button className="property-delete" aria-label={`Delete ${property.name}`} onClick={() => deleteProperty(index)}><X size={12}/></button>{(property.type === "ValueType" || property.type === "Struct") && <input className="property-reference" placeholder={`${property.type} API name`} value={property.reference ?? ""} onChange={(event) => updateProperty(index, { reference: event.target.value })}/>}<input className="property-description" placeholder="Property description" value={property.description ?? ""} onChange={(event) => updateProperty(index, { description: event.target.value })}/><PropertyFlags kind={selected.data.kind} property={property} onChange={(propertyPatch) => updateProperty(index, propertyPatch)}/>{property.derived && <input placeholder="Controlled expression" value={property.expression ?? ""} onChange={(event) => updateProperty(index, { expression: event.target.value })}/>}</div>)}</div>
        {selected.data.kind === "function" && <div className="inspector-section function-runner"><div className="section-title"><span>Test function</span></div><label>Arguments JSON<textarea aria-label="Function arguments JSON" value={functionArguments} onChange={(event) => setFunctionArguments(event.target.value)}/></label><button className="button primary" disabled={!publishedVersionId || functionBusy} onClick={() => void runFunction()}>{functionBusy ? "Running…" : "Run published version"}</button>{!publishedVersionId && <small>Publish the current ontology before running this function.</small>}{functionResult && <pre aria-label="Function result">{functionResult}</pre>}</div>}
        <button className="button danger full" onClick={deleteSelectedNode}><Trash2 size={14}/> Delete {kindMeta[selected.data.kind].label.toLowerCase()}</button>
      </> : selectedEdge?.data ? <>
        <h2><Link2 size={15}/> Link type</h2>
        <label>Display name<input value={selectedEdge.data.displayName} onChange={(event) => updateLink({ displayName: event.target.value })}/></label>
        <label>API name<input value={selectedEdge.data.apiName} onChange={(event) => updateLink({ apiName: event.target.value })}/></label>
        <label>Description<textarea value={selectedEdge.data.description} onChange={(event) => updateLink({ description: event.target.value })}/></label>
        <div className="cardinality-grid"><label>Source cardinality<select value={selectedEdge.data.sourceCardinality} onChange={(event) => updateLink({ sourceCardinality: event.target.value as "one" | "many" })}><option value="one">One</option><option value="many">Many</option></select></label><label>Target cardinality<select value={selectedEdge.data.targetCardinality} onChange={(event) => updateLink({ targetCardinality: event.target.value as "one" | "many" })}><option value="one">One</option><option value="many">Many</option></select></label></div>
        <label className="inline-checkbox"><input type="checkbox" checked={!!selectedEdge.data.required} onChange={(event) => updateLink({ required: event.target.checked })}/> Required relationship</label>
        <div className="inspector-section"><div className="section-title"><span>Link properties</span><button onClick={addLinkProperty}><Plus size={13}/> Add</button></div>{selectedEdge.data.properties.map((property, index) => <div className="property-edit compact" key={index}><input value={property.name} onChange={(event) => updateLink({ properties: selectedEdge.data!.properties.map((item, itemIndex) => itemIndex === index ? { ...item, name: event.target.value } : item) })}/><select value={property.type} onChange={(event) => updateLink({ properties: selectedEdge.data!.properties.map((item, itemIndex) => itemIndex === index ? { ...item, type: event.target.value } : item) })}><option>String</option><option>Boolean</option><option>Int64</option><option>Date</option><option>Timestamp</option><option>JSON</option></select><button className="property-delete" aria-label={`Delete ${property.name}`} onClick={() => updateLink({ properties: selectedEdge.data!.properties.filter((_, itemIndex) => itemIndex !== index) })}><X size={12}/></button><input className="property-description" placeholder="Property description" value={property.description ?? ""} onChange={(event) => updateLink({ properties: selectedEdge.data!.properties.map((item, itemIndex) => itemIndex === index ? { ...item, description: event.target.value } : item) })}/><PropertyFlags kind="link" property={property} onChange={(propertyPatch) => updateLink({ properties: selectedEdge.data!.properties.map((item, itemIndex) => itemIndex === index ? { ...item, ...propertyPatch } : item) })}/></div>)}</div>
        <button className="button danger full" onClick={() => { setEdges((items) => items.filter((edge) => edge.id !== selectedEdge.id)); setSelectedEdgeId(null); }}><Trash2 size={14}/> Delete link</button>
      </> : <p>Select a node or a link to inspect it.</p>}</aside>
    </div>
  </div>;
}
