"use client";

import dynamic from "next/dynamic";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type { ForceGraphMethods as ForceGraph2DMethods, NodeObject as NodeObject2D } from "react-force-graph-2d";
import type { ForceGraphMethods as ForceGraph3DMethods, NodeObject as NodeObject3D } from "react-force-graph-3d";
import { CanvasTexture, LinearFilter, Sprite, SpriteMaterial, SRGBColorSpace } from "three";
import {
  ArrowLeft, ArrowRight, Box, DatabaseZap, Download, Filter, Focus, GitBranch, Layers3,
  Maximize2, Minus, Network, Plus, RotateCcw, Search, Tags, X,
} from "lucide-react";
import { GraphQueryBuilder } from "@/components/graph-query-builder";
import type { BackendPropertyProvenance, GraphQuerySpec } from "@/lib/context-hub-client";
import type { ImportedGraph, ImportedGraphNode } from "@/lib/graph-data";
import type { OntologyCatalog } from "@/lib/ontology-catalog";

const ForceGraph2D = dynamic(() => import("react-force-graph-2d"), { ssr: false });
const ForceGraph3D = dynamic(() => import("react-force-graph-3d"), { ssr: false });

type VisualNode = ImportedGraphNode & { x?: number; y?: number; z?: number };

const linkColors: Record<string, string> = {
  owned_by: "#f59e0b",
  depends_on: "#788cff",
};

function endpointId(value: unknown) {
  return typeof value === "object" && value !== null && "id" in value ? String((value as { id: unknown }).id) : String(value);
}

function create3dLabel(node: VisualNode) {
  const label = node.name.length > 28 ? `${node.name.slice(0, 27)}…` : node.name;
  const canvas = document.createElement("canvas");
  const context = canvas.getContext("2d");
  if (!context) return new Sprite();

  context.font = "600 32px Inter, sans-serif";
  canvas.width = Math.ceil(context.measureText(label).width) + 28;
  canvas.height = 52;

  const drawingContext = canvas.getContext("2d");
  if (!drawingContext) return new Sprite();
  drawingContext.fillStyle = "rgba(7, 11, 18, .86)";
  drawingContext.fillRect(0, 0, canvas.width, canvas.height);
  drawingContext.fillStyle = node.color;
  drawingContext.fillRect(0, 0, 5, canvas.height);
  drawingContext.font = "600 32px Inter, sans-serif";
  drawingContext.textAlign = "center";
  drawingContext.textBaseline = "middle";
  drawingContext.fillStyle = "rgba(235, 240, 250, .96)";
  drawingContext.fillText(label, canvas.width / 2 + 2, canvas.height / 2 + 1);

  const texture = new CanvasTexture(canvas);
  texture.colorSpace = SRGBColorSpace;
  texture.minFilter = LinearFilter;
  const sprite = new Sprite(new SpriteMaterial({ map: texture, transparent: true, depthTest: false, depthWrite: false }));
  const height = 5.5;
  sprite.scale.set(height * canvas.width / canvas.height, height, 1);
  sprite.position.set(0, 11, 0);
  sprite.renderOrder = 10;
  return sprite;
}

export function GraphExplorer({
  graph, ontology, onOpenMapping, onOpenOntology, onLoadMore, onRunQuery, onExpandNode,
  onLoadProvenance,
  canLoadMore = false, loadingMore = false, queryBusy = false, expandingNodeId = "",
}: {
  graph: ImportedGraph;
  ontology: OntologyCatalog;
  onOpenMapping: () => void;
  onOpenOntology: () => void;
  onLoadMore?: () => void;
  canLoadMore?: boolean;
  loadingMore?: boolean;
  onRunQuery: (spec: GraphQuerySpec) => Promise<void>;
  onExpandNode: (node: ImportedGraphNode) => Promise<void>;
  onLoadProvenance: (node: ImportedGraphNode) => Promise<BackendPropertyProvenance[]>;
  queryBusy?: boolean;
  expandingNodeId?: string;
}) {
  const graph2dRef = useRef<ForceGraph2DMethods | undefined>(undefined);
  const graph3dRef = useRef<ForceGraph3DMethods | undefined>(undefined);
  const [mode, setMode] = useState<"2d" | "3d">("2d");
  const [selected, setSelected] = useState<VisualNode | null>(null);
  const [query, setQuery] = useState("");
  const [focusId, setFocusId] = useState<string | null>(null);
  const [history, setHistory] = useState<Array<string | null>>([null]);
  const [historyIndex, setHistoryIndex] = useState(0);
  const [hiddenKinds, setHiddenKinds] = useState<string[]>([]);
  const [showLabels, setShowLabels] = useState(true);
  const [filterOpen, setFilterOpen] = useState(false);
  const [queryOpen, setQueryOpen] = useState(false);
  const [nodeLimit, setNodeLimit] = useState(750);
  const [provenanceResult, setProvenanceResult] = useState<{ nodeId: string; items: BackendPropertyProvenance[]; error: string }>({ nodeId: "", items: [], error: "" });
  const provenance = selected?.id === provenanceResult.nodeId ? provenanceResult.items : [];
  const provenanceError = selected?.id === provenanceResult.nodeId ? provenanceResult.error : "";
  const provenanceLoading = !!selected && selected.id !== provenanceResult.nodeId;

  useEffect(() => {
    if (!selected) return;
    let cancelled = false;
    void onLoadProvenance(selected)
      .then((loaded) => { if (!cancelled) setProvenanceResult({ nodeId: selected.id, items: loaded, error: "" }); })
      .catch((cause) => { if (!cancelled) setProvenanceResult({ nodeId: selected.id, items: [], error: cause instanceof Error ? cause.message : "Provenance is unavailable." }); });
    return () => { cancelled = true; };
  }, [onLoadProvenance, selected]);

  const kindSummary = useMemo(() => {
    const summary = new Map<string, { count: number; color: string }>();
    for (const node of graph.nodes) {
      const current = summary.get(node.kind);
      summary.set(node.kind, { count: (current?.count ?? 0) + 1, color: current?.color ?? node.color });
    }
    return [...summary.entries()].sort((left, right) => right[1].count - left[1].count);
  }, [graph.nodes]);

  const data = useMemo(() => {
    const visibleByType = graph.nodes.filter((node) => !hiddenKinds.includes(node.kind));
    let candidates = visibleByType;
    if (focusId) {
      const neighborhood = new Set([focusId]);
      for (const link of graph.links) {
        const source = endpointId(link.source);
        const target = endpointId(link.target);
        if (source === focusId) neighborhood.add(target);
        if (target === focusId) neighborhood.add(source);
      }
      candidates = visibleByType.filter((node) => neighborhood.has(node.id));
    }
    let visibleNodes = candidates.slice(0, nodeLimit);
    if (!focusId && candidates.length > nodeLimit) {
      const reservedPerKind = Math.max(1, Math.floor(nodeLimit * .08));
      const reserved = kindSummary.flatMap(([kind]) => candidates.filter((node) => node.kind === kind).slice(0, reservedPerKind));
      const reservedIds = new Set(reserved.map((node) => node.id));
      visibleNodes = [...reserved, ...candidates.filter((node) => !reservedIds.has(node.id)).slice(0, Math.max(0, nodeLimit - reserved.length))];
    }
    const ids = new Set(visibleNodes.map((node) => node.id));
    const visibleLinks = graph.links.filter((link) => ids.has(endpointId(link.source)) && ids.has(endpointId(link.target)));
    const degrees = new Map<string, number>();
    for (const link of visibleLinks) {
      const source = endpointId(link.source), target = endpointId(link.target);
      degrees.set(source, (degrees.get(source) ?? 0) + 1);
      degrees.set(target, (degrees.get(target) ?? 0) + 1);
    }
    return {
      nodes: visibleNodes.map((node) => ({ ...node, degree: degrees.get(node.id) ?? 0 })),
      links: visibleLinks.map((link) => ({ ...link })),
    };
  }, [focusId, graph.links, graph.nodes, hiddenKinds, kindSummary, nodeLimit]);

  const searchResults = useMemo(() => {
    const normalized = query.trim().toLowerCase();
    if (!normalized) return [];
    return graph.nodes.filter((node) => node.name.toLowerCase().includes(normalized) || node.id.toLowerCase().includes(normalized)).slice(0, 8);
  }, [graph.nodes, query]);

  const selectedRelations = useMemo(() => {
    if (!selected) return [];
    const names = new Map(graph.nodes.map((node) => [node.id, node.name]));
    return graph.links.flatMap((link) => {
      const source = endpointId(link.source), target = endpointId(link.target);
      if (source === selected.id) return [{ direction: "outgoing", label: link.label, object: names.get(target) ?? target, objectId: target }];
      if (target === selected.id) return [{ direction: "incoming", label: link.label, object: names.get(source) ?? source, objectId: source }];
      return [];
    });
  }, [graph, selected]);

  function fitGraph() {
    if (mode === "2d") graph2dRef.current?.zoomToFit(650, 70);
    else graph3dRef.current?.zoomToFit(750, 80);
  }

  function zoomBy(factor: number) {
    if (mode === "2d") {
      const current = graph2dRef.current?.zoom() ?? 1;
      graph2dRef.current?.zoom(Math.max(0.15, Math.min(20, current * factor)), 250);
      return;
    }
    const camera = graph3dRef.current?.camera();
    if (!camera) return;
    graph3dRef.current?.cameraPosition({ x: camera.position.x / factor, y: camera.position.y / factor, z: camera.position.z / factor }, undefined, 300);
  }

  function centerNode(node: VisualNode) {
    setSelected(node);
    if (mode === "2d" && node.x !== undefined && node.y !== undefined) {
      graph2dRef.current?.centerAt(node.x, node.y, 500);
      graph2dRef.current?.zoom(Math.max(2.5, graph2dRef.current.zoom()), 500);
    }
    if (mode === "3d" && node.x !== undefined && node.y !== undefined && node.z !== undefined) {
      const distance = 90;
      const length = Math.hypot(node.x, node.y, node.z) || 1;
      const ratio = 1 + distance / length;
      graph3dRef.current?.cameraPosition({ x: node.x * ratio, y: node.y * ratio, z: node.z * ratio }, { x: node.x, y: node.y, z: node.z }, 650);
    }
  }

  function navigateTo(id: string | null) {
    const nextHistory = [...history.slice(0, historyIndex + 1), id];
    setHistory(nextHistory); setHistoryIndex(nextHistory.length - 1); setFocusId(id);
    if (id) {
      const node = graph.nodes.find((item) => item.id === id);
      if (node) setSelected(node);
    } else setSelected(null);
  }

  function moveHistory(direction: -1 | 1) {
    const nextIndex = historyIndex + direction;
    if (nextIndex < 0 || nextIndex >= history.length) return;
    const id = history[nextIndex];
    setHistoryIndex(nextIndex); setFocusId(id);
    setSelected(id ? graph.nodes.find((node) => node.id === id) ?? null : null);
  }

  function switchMode(nextMode: "2d" | "3d") {
    setMode(nextMode);
    if (nextMode === "3d" && nodeLimit > 500) setNodeLimit(500);
    window.setTimeout(() => {
      if (nextMode === "2d") graph2dRef.current?.zoomToFit(650, 70);
      else graph3dRef.current?.zoomToFit(750, 80);
    }, 120);
  }

  function toggleKind(kind: string) {
    setHiddenKinds((items) => items.includes(kind) ? items.filter((item) => item !== kind) : [...items, kind]);
  }

  async function expandNode(node: VisualNode) {
    await onExpandNode(node);
    navigateTo(node.id);
  }

  const renderLabel = (rawNode: NodeObject2D, context: CanvasRenderingContext2D, scale: number) => {
    const node = rawNode as NodeObject2D<VisualNode>;
    if (!showLabels || node.x === undefined || node.y === undefined || (!focusId && data.nodes.length > 900 && scale < 1.6)) return;
    const fontSize = Math.max(3.6, 11 / scale);
    const label = String(node.name).length > 28 ? `${String(node.name).slice(0, 27)}…` : String(node.name);
    const labelY = node.y + 7 / scale;
    const paddingX = 3 / scale;
    const paddingY = 2 / scale;
    context.font = `600 ${fontSize}px Inter, sans-serif`;
    context.textAlign = "center";
    context.textBaseline = "top";
    const textWidth = context.measureText(label).width;
    context.fillStyle = "rgba(7, 11, 18, .82)";
    context.fillRect(node.x - textWidth / 2 - paddingX, labelY - paddingY, textWidth + paddingX * 2, fontSize + paddingY * 2);
    context.fillStyle = selected?.id === node.id ? "#ffffff" : "rgba(225, 232, 247, .92)";
    context.fillText(label, node.x, labelY);
  };

  const paintNodePointerArea = (rawNode: NodeObject2D, color: string, context: CanvasRenderingContext2D, scale: number) => {
    const node = rawNode as NodeObject2D<VisualNode>;
    if (node.x === undefined || node.y === undefined) return;
    const hitRadius = Math.max(8 / scale, 4.5);
    context.beginPath();
    context.arc(node.x, node.y, hitRadius, 0, 2 * Math.PI);
    context.fillStyle = color;
    context.fill();
  };

  const render3dLabel = useCallback((rawNode: NodeObject3D) => {
    const label = create3dLabel(rawNode as VisualNode);
    label.visible = showLabels;
    return label;
  }, [showLabels]);

  if (graph.nodes.length === 0) return <div className="workspace-view graph-view"><header className="stage-header"><div><span className="eyebrow">Graph explorer</span><h1>Explore</h1><p>Navigate ontology instances and their relationships.</p></div></header><div className="empty-graph"><DatabaseZap size={34}/><h2>No imported data yet</h2><p>Map a source file to ontology object and link types, then import it.</p><button className="button primary" onClick={onOpenMapping}>Open data mapping</button></div></div>;

  return <div className="workspace-view graph-view orbit-explorer">
    <header className="explorer-header"><div><span className="eyebrow">Knowledge graph</span><h1>{graph.sourceName || "Explore"}</h1><p>{graph.nodes.length.toLocaleString("de-DE")} loaded objects · {graph.links.length.toLocaleString("de-DE")} relationships</p></div><div className="binding-pills">{graph.aggregations?.map((aggregation) => <span className="aggregation-pill" key={aggregation.alias}>{aggregation.alias} · {String(aggregation.value)}</span>)}{graph.ontologyBindings.objectTypes.map((type) => <span key={type}>Object · {type}</span>)}</div></header>
    <div className="explorer-commandbar"><div className="explorer-tabs"><button className="active">Explore</button><button onClick={onOpenOntology}>Schema</button></div><div className="explorer-actions"><div className="explorer-search"><Search size={14}/><input value={query} onChange={(event) => setQuery(event.target.value)} placeholder="Search objects…"/>{query && <button onClick={() => setQuery("")} aria-label="Clear search"><X size={13}/></button>}{searchResults.length > 0 && <div className="search-results">{searchResults.map((node) => <button key={node.id} onClick={() => { navigateTo(node.id); setQuery(""); }}><i style={{ background: node.color }}/><span><strong>{node.name}</strong><small>{node.kind} · {node.id}</small></span></button>)}</div>}</div><button className={queryOpen ? "active" : ""} onClick={() => setQueryOpen(true)}><Filter size={14}/> Query</button><button className={filterOpen ? "active" : ""} onClick={() => setFilterOpen((open) => !open)}><Layers3 size={14}/> Types</button>{canLoadMore && <button onClick={onLoadMore} disabled={loadingMore}><Download size={14}/> {loadingMore ? "Loading…" : "Load more"}</button>}<label className="limit-select">Show<select value={nodeLimit} onChange={(event) => setNodeLimit(Number(event.target.value))}><option value={250}>250</option><option value={500}>500</option><option value={750}>750</option><option value={1000}>1.000</option><option value={2000}>2.000</option><option value={5000}>5.000</option></select></label><div className="view-switch"><button className={mode === "2d" ? "active" : ""} onClick={() => switchMode("2d")}><Network size={14}/> 2D</button><button className={mode === "3d" ? "active" : ""} onClick={() => switchMode("3d")}><Box size={14}/> 3D</button></div></div></div>
    {queryOpen && <GraphQueryBuilder
      catalog={ontology}
      busy={queryBusy}
      onClose={() => setQueryOpen(false)}
      onRun={onRunQuery}
    />}
    <div className="explorer-canvas-shell">
      <div className="graph-canvas orbit-canvas">{mode === "2d" ? <ForceGraph2D ref={graph2dRef} graphData={data} nodeLabel={(node) => `${node.name} · ${node.kind}`} nodeColor="color" nodeVal={(node) => 1.5 + Math.sqrt(Number(node.degree ?? 0) + 1)} nodeRelSize={4} nodeCanvasObjectMode={() => "after"} nodeCanvasObject={renderLabel} nodePointerAreaPaint={paintNodePointerArea} linkLabel="label" linkWidth={(link) => focusId && (endpointId(link.source) === focusId || endpointId(link.target) === focusId) ? 2.2 : .75} linkColor={(link) => linkColors[String(link.label)] ?? "#566889"} linkDirectionalArrowLength={3.5} linkDirectionalArrowRelPos={1} linkCurvature={.08} cooldownTicks={120} onEngineStop={fitGraph} onNodeClick={(node, event) => { const visualNode = node as VisualNode; centerNode(visualNode); if (event.detail >= 2) void expandNode(visualNode); }} onBackgroundClick={() => setSelected(null)} enableNodeDrag enablePanInteraction enableZoomInteraction backgroundColor="#090c13"/> : <ForceGraph3D ref={graph3dRef} graphData={data} nodeLabel={(node) => `${node.name} · ${node.kind}`} nodeColor="color" nodeVal={(node) => 1.5 + Math.sqrt(Number(node.degree ?? 0) + 1)} nodeRelSize={6} nodeThreeObject={render3dLabel} nodeThreeObjectExtend linkLabel="label" linkWidth={(link) => focusId && (endpointId(link.source) === focusId || endpointId(link.target) === focusId) ? 2.2 : 1} linkColor={(link) => linkColors[String(link.label)] ?? "#60769c"} linkOpacity={.68} linkDirectionalArrowLength={4} linkDirectionalParticles={focusId ? 1 : 0} linkDirectionalParticleWidth={1.4} cooldownTicks={120} onEngineStop={fitGraph} onNodeClick={(node, event) => { const visualNode = node as VisualNode; centerNode(visualNode); if (event.detail >= 2) void expandNode(visualNode); }} onBackgroundClick={() => setSelected(null)} enableNodeDrag enableNavigationControls backgroundColor="#090c13"/>}</div>

      <aside className="entity-legend"><div className="overlay-heading"><Layers3 size={14}/><strong>Entities</strong></div>{kindSummary.map(([kind, summary]) => <button className={hiddenKinds.includes(kind) ? "disabled" : ""} key={kind} onClick={() => toggleKind(kind)}><i style={{ background: summary.color }}/><span>{kind}</span><strong>{summary.count.toLocaleString("de-DE")}</strong></button>)}<div className="legend-divider"/><label><span><Tags size={13}/> Show labels</span><input type="checkbox" checked={showLabels} onChange={(event) => setShowLabels(event.target.checked)}/></label></aside>

      <div className="navigation-cluster"><button onClick={() => moveHistory(-1)} disabled={historyIndex === 0} title="Back"><ArrowLeft size={15}/></button><button onClick={() => moveHistory(1)} disabled={historyIndex >= history.length - 1} title="Forward"><ArrowRight size={15}/></button><span/><button onClick={() => zoomBy(1.35)} title="Zoom in"><Plus size={15}/></button><button onClick={() => zoomBy(.74)} title="Zoom out"><Minus size={15}/></button><button onClick={fitGraph} title="Fit graph"><Maximize2 size={14}/></button><button onClick={() => navigateTo(null)} title="Reset exploration"><RotateCcw size={14}/></button></div>

      {filterOpen && <div className="filter-popover"><div className="overlay-heading"><Filter size={14}/><strong>Visible object types</strong><button onClick={() => setFilterOpen(false)}><X size={13}/></button></div>{kindSummary.map(([kind, summary]) => <label key={kind}><input type="checkbox" checked={!hiddenKinds.includes(kind)} onChange={() => toggleKind(kind)}/><i style={{ background: summary.color }}/><span>{kind}</span></label>)}</div>}

      {focusId && <div className="focus-breadcrumb"><Focus size={13}/><span>Neighborhood</span><strong>{graph.nodes.find((node) => node.id === focusId)?.name}</strong><button onClick={() => navigateTo(null)}><X size={13}/></button></div>}

      {selected && <aside className="floating-inspector"><button className="inspector-close" onClick={() => setSelected(null)} aria-label="Close inspector"><X size={14}/></button><span className="type-pill" style={{ color: selected.color }}>{selected.kind}</span><h2>{selected.name}</h2><span className="mono-id">{selected.id}</span><div className="inspector-section"><span className="eyebrow">Properties</span>{Object.entries(selected.properties).slice(0, 8).map(([key, value]) => { const origin = provenance.find((item) => item.property === key); return <div className="property-provenance" key={key}><div className="detail-row"><span>{key}</span><strong title={typeof value === "object" ? JSON.stringify(value) : String(value ?? "")}>{typeof value === "object" ? JSON.stringify(value) : String(value ?? "")}</strong></div>{origin && <small title={`Mapping ${origin.ontologyMappingName} · Job ${origin.ingestionJobId}`}>{origin.dataSourceName} · {origin.sourceField}{origin.importedAt ? ` · ${origin.importedAt.toLocaleString("de-DE")}` : ""}</small>}</div>; })}{provenanceLoading && <p className="muted-copy">Loading property origins…</p>}{provenanceError && <p className="provenance-error">{provenanceError}</p>}{!provenanceLoading && !provenanceError && !provenance.length && <p className="muted-copy">No property origins were recorded.</p>}</div><div className="inspector-section"><span className="eyebrow">Relationships · {selectedRelations.length}</span>{selectedRelations.slice(0, 8).map((relation, index) => <button className="relation-row" key={`${relation.direction}-${relation.objectId}-${index}`} onClick={() => navigateTo(relation.objectId)}><span>{relation.direction === "outgoing" ? "→" : "←"} {relation.label}</span><strong>{relation.object}</strong></button>)}{!selectedRelations.length && <p className="muted-copy">No loaded relationships.</p>}</div><button className="button secondary full" disabled={expandingNodeId === selected.id} onClick={() => void expandNode(selected)}><GitBranch size={14}/> {expandingNodeId === selected.id ? "Loading connections…" : "Load connected objects"}</button></aside>}

      <div className="explorer-status"><span>Showing {data.nodes.length.toLocaleString("de-DE")} of {graph.nodes.length.toLocaleString("de-DE")} loaded objects · {data.links.length.toLocaleString("de-DE")} visible links{canLoadMore ? " · more available" : ""}</span><span>{mode === "2d" ? "Drag to move · Wheel to zoom · Double-click a node to expand" : "Drag to rotate · Right-drag to pan · Wheel to zoom · Double-click to expand"}</span></div>
    </div>
  </div>;
}
