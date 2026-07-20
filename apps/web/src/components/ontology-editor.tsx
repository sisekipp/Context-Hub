"use client";

import { useCallback, useEffect, useMemo, useState, useSyncExternalStore } from "react";
import {
  Background, BackgroundVariant, Controls, Handle, MiniMap, Position, ReactFlow, addEdge,
  useEdgesState, useNodesState, type Connection, type Edge, type Node, type NodeProps,
} from "@xyflow/react";
import { Braces, Check, CircleDot, Code2, Component, GitFork, Link2, Plus, Save, Send, Share2, Trash2 } from "lucide-react";
import type { OntologyCatalog } from "@/lib/ontology-catalog";

type Property = { name: string; type: string; identity?: boolean; shared?: boolean; derived?: boolean; expression?: string };
type NodeKind = "object" | "interface" | "value_type" | "struct" | "shared_property" | "function";
type OntologyNodeData = {
  [key: string]: unknown;
  kind: NodeKind;
  displayName: string;
  apiName: string;
  description: string;
  properties: Property[];
  functionOutput?: string;
  implementation?: string;
};
type LinkData = {
  [key: string]: unknown;
  apiName: string;
  displayName: string;
  description: string;
  sourceCardinality: "one" | "many";
  targetCardinality: "one" | "many";
  properties: Property[];
};
type OntologyNode = Node<OntologyNodeData, "ontology">;
type OntologyEdge = Edge<LinkData>;

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

const subscribeToHydration = () => () => undefined;
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

type OntologyEditorProps = { ontologyId: string; ontologyName: string; seedTemplate: boolean; onRename: (name: string) => void; onCatalogChange?: (catalog: OntologyCatalog) => void };

export function OntologyEditor(props: OntologyEditorProps) {
  const hydrated = useSyncExternalStore(subscribeToHydration, getClientHydrationSnapshot, getServerHydrationSnapshot);
  return hydrated ? <HydratedOntologyEditor {...props}/> : <div className="workspace-view" aria-label="Loading ontology editor"/>;
}

function HydratedOntologyEditor({ ontologyId, ontologyName, seedTemplate, onRename, onCatalogChange }: OntologyEditorProps) {
  const document = useMemo(() => initialDocument(ontologyId, seedTemplate), [ontologyId, seedTemplate]);
  const [nodes, setNodes, onNodesChange] = useNodesState<OntologyNode>(document.nodes);
  const [edges, setEdges, onEdgesChange] = useEdgesState<OntologyEdge>(document.edges);
  const [selectedId, setSelectedId] = useState<string>(document.nodes[0]?.id ?? "");
  const [selectedEdgeId, setSelectedEdgeId] = useState<string | null>(null);
  const [saveState, setSaveState] = useState("Autosave on");
  const [notice, setNotice] = useState<string | null>(null);
  const nodeTypes = useMemo(() => ({ ontology: OntologyCard }), []);
  const selected = nodes.find((node) => node.id === selectedId);
  const selectedEdge = edges.find((edge) => edge.id === selectedEdgeId);

  const connect = useCallback((connection: Connection) => {
    const number = edges.length + 1;
    const data: LinkData = { apiName: `link_${number}`, displayName: `Link ${number}`, description: "", sourceCardinality: "many", targetCardinality: "many", properties: [] };
    const edge: OntologyEdge = { ...connection, id: crypto.randomUUID(), type: "smoothstep", label: data.apiName, data };
    setEdges((items) => addEdge(edge, items)); setSelectedId(""); setSelectedEdgeId(edge.id);
  }, [edges.length, setEdges]);

  useEffect(() => {
    const objectNodes = nodes.filter((node) => node.data.kind === "object");
    const objectNodeIds = new Set(objectNodes.map((node) => node.id));
    onCatalogChange?.({
      objectTypes: objectNodes.map((node) => ({
        apiName: node.data.apiName,
        displayName: node.data.displayName,
        properties: node.data.properties.map((property) => ({ apiName: property.name, displayName: property.name, type: property.type, identity: property.identity, derived: property.derived })),
      })),
      linkTypes: edges.filter((edge) => objectNodeIds.has(edge.source) && objectNodeIds.has(edge.target) && edge.data).map((edge) => ({
        apiName: edge.data!.apiName,
        displayName: edge.data!.displayName,
        sourceType: nodes.find((node) => node.id === edge.source)!.data.apiName,
        targetType: nodes.find((node) => node.id === edge.target)!.data.apiName,
        properties: edge.data!.properties.map((property) => ({ apiName: property.name, displayName: property.name, type: property.type })),
      })),
    });
    const timeout = window.setTimeout(() => { localStorage.setItem(`context-hub.ontology.${ontologyId}`, JSON.stringify({ nodes, edges })); setSaveState("Saved locally"); }, 500);
    return () => window.clearTimeout(timeout);
  }, [nodes, edges, onCatalogChange, ontologyId]);

  function addNode(kind: NodeKind) {
    const count = nodes.filter((node) => node.data.kind === kind).length + 1;
    const id = `${kind}-${crypto.randomUUID()}`;
    const label = kindMeta[kind].label;
    const properties: Property[] = kind === "object" ? [{ name: "id", type: "String", identity: true }] : kind === "value_type" ? [{ name: "base_type", type: "String" }] : kind === "shared_property" ? [{ name: "value", type: "String", shared: true }] : [];
    setNodes((items) => [...items, { id, type: "ontology", position: { x: 130 + count * 42, y: 100 + count * 48 }, data: { kind, displayName: `${label} ${count}`, apiName: `${kind}_${count}`, description: "", properties, ...(kind === "function" ? { functionOutput: "String", implementation: "expression" } : {}) } }]);
    setSelectedEdgeId(null); setSelectedId(id);
  }

  function updateSelected(field: "displayName" | "apiName" | "description" | "functionOutput" | "implementation", value: string) {
    setNodes((items) => items.map((node) => node.id === selectedId ? { ...node, data: { ...node.data, [field]: value } } : node));
  }

  function addProperty() {
    if (!selected) return;
    const number = selected.data.properties.length + 1;
    const property: Property = { name: `property_${number}`, type: "String", ...(selected.data.kind === "shared_property" ? { shared: true } : {}) };
    setNodes((items) => items.map((node) => node.id === selected.id ? { ...node, data: { ...node.data, properties: [...node.data.properties, property] } } : node));
  }

  function updateProperty(index: number, patch: Partial<Property>) {
    setNodes((items) => items.map((node) => node.id === selectedId ? { ...node, data: { ...node.data, properties: node.data.properties.map((property, propertyIndex) => propertyIndex === index ? { ...property, ...patch } : property) } } : node));
  }

  function updateLink(patch: Partial<LinkData>) {
    setEdges((items) => items.map((edge) => edge.id === selectedEdgeId ? { ...edge, label: patch.apiName ?? edge.data?.apiName, data: { ...(edge.data as LinkData), ...patch } } : edge));
  }

  function addLinkProperty() {
    if (!selectedEdge?.data) return;
    updateLink({ properties: [...selectedEdge.data.properties, { name: `property_${selectedEdge.data.properties.length + 1}`, type: "String" }] });
  }

  function validateAndPublish() {
    const invalidNode = nodes.find((node) => !/^[a-z][a-z0-9_]{0,63}$/.test(node.data.apiName));
    const invalidLink = edges.find((edge) => !edge.data || !/^[a-z][a-z0-9_]{0,63}$/.test(edge.data.apiName));
    const missingIdentity = nodes.find((node) => node.data.kind === "object" && !node.data.properties.some((property) => property.identity));
    setNotice(invalidNode ? `Invalid API name: ${invalidNode.data.apiName}` : invalidLink ? "A link needs a valid API name" : missingIdentity ? `${missingIdentity.data.displayName} needs an identity property` : "The ontology draft is valid. Functions are read-only; Actions are not part of this version.");
  }

  return <div className="workspace-view"><header className="stage-header"><div><span className="eyebrow">Ontology editor · isolated model</span><input className="ontology-name-input" aria-label="Ontology name" value={ontologyName} onChange={(event) => onRename(event.target.value)}/><p>Object types, links, interfaces, reusable types and read-only functions.</p></div><div className="header-actions"><span className="save-state"><Save size={13}/> {saveState}</span><button className="button secondary" onClick={validateAndPublish}><Check size={15}/> Validate</button><button className="button primary" onClick={validateAndPublish}><Send size={15}/> Publish</button></div></header>
    {notice && <div className="notice" role="status">{notice}<button onClick={() => setNotice(null)}>×</button></div>}
    <div className="editor-layout"><div className="canvas-panel"><div className="canvas-toolbar">{(["object", "interface", "value_type", "struct", "shared_property", "function"] as NodeKind[]).map((kind) => <button key={kind} onClick={() => addNode(kind)}><Plus size={14}/> {kindMeta[kind].label}</button>)}</div>
      <ReactFlow nodes={nodes} edges={edges} nodeTypes={nodeTypes} onNodesChange={onNodesChange} onEdgesChange={onEdgesChange} onConnect={connect} onNodeClick={(_, node) => { setSelectedId(node.id); setSelectedEdgeId(null); }} onEdgeClick={(_, edge) => { setSelectedId(""); setSelectedEdgeId(edge.id); }} fitView><Background variant={BackgroundVariant.Dots} gap={18} size={1}/><MiniMap pannable zoomable/><Controls/></ReactFlow></div>
      <aside className="inspector"><span className="eyebrow">Inspector</span>{selected ? <><h2>{kindMeta[selected.data.kind].label}</h2><label>Display name<input value={selected.data.displayName} onChange={(event) => updateSelected("displayName", event.target.value)}/></label><label>API name<input value={selected.data.apiName} onChange={(event) => updateSelected("apiName", event.target.value)}/><small>Stable after first publish</small></label><label>Description<textarea value={selected.data.description} onChange={(event) => updateSelected("description", event.target.value)}/></label>{selected.data.kind === "function" && <><label>Output type<input value={selected.data.functionOutput} onChange={(event) => updateSelected("functionOutput", event.target.value)}/></label><label>Implementation<select value={selected.data.implementation} onChange={(event) => updateSelected("implementation", event.target.value)}><option value="expression">Expression</option><option value="external_grpc">External gRPC</option><option value="wasm">WASM</option></select><small>Functions are read-only in V1.</small></label></>}
        <div className="inspector-section"><div className="section-title"><span>{selected.data.kind === "function" ? "Inputs" : selected.data.kind === "struct" ? "Fields" : "Properties"}</span><button onClick={addProperty}><Plus size={13}/> Add</button></div>{selected.data.properties.map((property, index) => <div className="property-edit" key={`${property.name}-${index}`}><input value={property.name} onChange={(event) => updateProperty(index, { name: event.target.value })}/><select value={property.type} onChange={(event) => updateProperty(index, { type: event.target.value })}>{["String", "Boolean", "Int64", "Float64", "Decimal", "Date", "Timestamp", "UUID", "Enum", "JSON", "List", "ValueType", "Struct"].map((type) => <option key={type}>{type}</option>)}</select>{selected.data.kind === "object" && <div className="property-flags"><label><input type="checkbox" checked={!!property.identity} onChange={(event) => updateProperty(index, { identity: event.target.checked })}/> Identity</label><label><input type="checkbox" checked={!!property.shared} onChange={(event) => updateProperty(index, { shared: event.target.checked })}/> Shared</label><label><input type="checkbox" checked={!!property.derived} onChange={(event) => updateProperty(index, { derived: event.target.checked })}/> Derived</label></div>}{property.derived && <input placeholder="Controlled expression" value={property.expression ?? ""} onChange={(event) => updateProperty(index, { expression: event.target.value })}/>}</div>)}</div>
      </> : selectedEdge?.data ? <><h2><Link2 size={15}/> Link type</h2><label>Display name<input value={selectedEdge.data.displayName} onChange={(event) => updateLink({ displayName: event.target.value })}/></label><label>API name<input value={selectedEdge.data.apiName} onChange={(event) => updateLink({ apiName: event.target.value })}/></label><label>Description<textarea value={selectedEdge.data.description} onChange={(event) => updateLink({ description: event.target.value })}/></label><div className="cardinality-grid"><label>Source cardinality<select value={selectedEdge.data.sourceCardinality} onChange={(event) => updateLink({ sourceCardinality: event.target.value as "one" | "many" })}><option value="one">One</option><option value="many">Many</option></select></label><label>Target cardinality<select value={selectedEdge.data.targetCardinality} onChange={(event) => updateLink({ targetCardinality: event.target.value as "one" | "many" })}><option value="one">One</option><option value="many">Many</option></select></label></div><div className="inspector-section"><div className="section-title"><span>Link properties</span><button onClick={addLinkProperty}><Plus size={13}/> Add</button></div>{selectedEdge.data.properties.map((property, index) => <div className="property-edit compact" key={index}><input value={property.name} onChange={(event) => updateLink({ properties: selectedEdge.data!.properties.map((item, itemIndex) => itemIndex === index ? { ...item, name: event.target.value } : item) })}/><select value={property.type} onChange={(event) => updateLink({ properties: selectedEdge.data!.properties.map((item, itemIndex) => itemIndex === index ? { ...item, type: event.target.value } : item) })}><option>String</option><option>Boolean</option><option>Int64</option><option>Date</option><option>Timestamp</option><option>JSON</option></select></div>)}</div><button className="button danger full" onClick={() => { setEdges((items) => items.filter((edge) => edge.id !== selectedEdge.id)); setSelectedEdgeId(null); }}><Trash2 size={14}/> Delete link</button></> : <p>Select a node or a link to inspect it.</p>}</aside>
    </div>
  </div>;
}
