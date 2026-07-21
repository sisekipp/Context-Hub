"use client";

import { useCallback, useEffect, useMemo, useRef, useState, useSyncExternalStore } from "react";
import { Boxes, Database, DatabaseZap, History, Orbit, Plus, Workflow } from "lucide-react";
import { DataSourceManager } from "@/components/data-source-manager";
import { GraphExplorer } from "@/components/graph-explorer";
import { ImportHistory } from "@/components/import-history";
import { MappingPanel, parseSource, type BrowserDataSource } from "@/components/mapping-panel";
import { OntologyEditor } from "@/components/ontology-editor";
import { emptyGraph, type ImportedGraph } from "@/lib/graph-data";
import { defaultOntologyCatalog, type OntologyCatalog } from "@/lib/ontology-catalog";
import { createWorkspaceOntology, downloadWorkspaceSource, expandPersistedGraphNode, getObjectProvenance, listWorkspaceDataSources, listWorkspaceOntologies, loadPersistedGraph, previewWorkspaceSource, queryPersistedGraph, type BackendDataSource, type GraphQuerySpec } from "@/lib/context-hub-client";

type Section = "ontology" | "sources" | "mapping" | "imports" | "graph";
type OntologyWorkspace = { id: string; name: string; slug: string; activeVersionId: string };

const ontologyRegistryKey = "context-hub.ontology-registry";
const defaultOntology: OntologyWorkspace = { id: "service-map", name: "Service map", slug: "service_map", activeVersionId: "" };
const defaultRegistry = { ontologies: [defaultOntology], activeOntologyId: defaultOntology.id };
const defaultRegistrySnapshot = JSON.stringify(defaultRegistry);
const emptyOntologyCatalog: OntologyCatalog = { objectTypes: [], linkTypes: [], interfaces: [], valueTypes: [], structTypes: [], sharedProperties: [], functions: [] };
const registryListeners = new Set<() => void>();
let registryCache = defaultRegistrySnapshot;

function subscribeToRegistry(callback: () => void) {
  registryListeners.add(callback);
  const handleStorage = (event: StorageEvent) => {
    if (event.key === ontologyRegistryKey) {
      registryCache = event.newValue ?? defaultRegistrySnapshot;
      callback();
    }
  };
  window.addEventListener("storage", handleStorage);
  return () => {
    registryListeners.delete(callback);
    window.removeEventListener("storage", handleStorage);
  };
}

function registrySnapshot() {
  registryCache = localStorage.getItem(ontologyRegistryKey) ?? defaultRegistrySnapshot;
  return registryCache;
}

function parseRegistry(snapshot: string): typeof defaultRegistry {
  try {
    const parsed = JSON.parse(snapshot) as typeof defaultRegistry;
    return parsed.ontologies?.length ? parsed : defaultRegistry;
  } catch {
    return defaultRegistry;
  }
}

function saveRegistry(registry: typeof defaultRegistry) {
  registryCache = JSON.stringify(registry);
  localStorage.setItem(ontologyRegistryKey, registryCache);
  registryListeners.forEach((listener) => listener());
}

const sections: Array<{ id: Section; label: string; icon: typeof Workflow }> = [
  { id: "ontology", label: "Ontology", icon: Workflow },
  { id: "sources", label: "Data sources", icon: Database },
  { id: "mapping", label: "Data mapping", icon: DatabaseZap },
  { id: "imports", label: "Imports", icon: History },
  { id: "graph", label: "Explore", icon: Orbit },
];

export default function Home() {
  const [section, setSection] = useState<Section>("ontology");
  const [catalogs, setCatalogs] = useState<Record<string, OntologyCatalog>>({ [defaultOntology.id]: defaultOntologyCatalog });
  const [graphs, setGraphs] = useState<Record<string, ImportedGraph>>({});
  const [dataSources, setDataSources] = useState<BrowserDataSource[]>([]);
  const [backendDataSources, setBackendDataSources] = useState<BackendDataSource[]>([]);
  const [activeDataSourceId, setActiveDataSourceId] = useState("");
  const [loadingMoreGraph, setLoadingMoreGraph] = useState(false);
  const [queryingGraph, setQueryingGraph] = useState(false);
  const [expandingNodeId, setExpandingNodeId] = useState("");
  const graphRequests = useRef(new Set<string>());
  const savedRegistry = useSyncExternalStore(subscribeToRegistry, registrySnapshot, () => defaultRegistrySnapshot);
  const registry = useMemo(() => parseRegistry(savedRegistry), [savedRegistry]);
  const ontologies = registry.ontologies;
  const activeOntologyId = ontologies.some((ontology) => ontology.id === registry.activeOntologyId) ? registry.activeOntologyId : ontologies[0].id;
  const activeOntology = ontologies.find((ontology) => ontology.id === activeOntologyId) ?? ontologies[0] ?? defaultOntology;
  const activeCatalog = catalogs[activeOntology.id] ?? emptyOntologyCatalog;
  const activeGraph = graphs[activeOntology.id] ?? emptyGraph;
  const activeDataSource = dataSources.find((source) => source.id === activeDataSourceId) ?? null;
  const updateActiveCatalog = useCallback((catalog: OntologyCatalog) => {
    setCatalogs((items) => ({ ...items, [activeOntology.id]: catalog }));
  }, [activeOntology.id]);

  const refreshDataSources = useCallback(async () => {
    const sources = await listWorkspaceDataSources();
    setBackendDataSources(sources);
    setDataSources((current) => sources.map((source) => {
      const existing = current.find((item) => item.id === source.id);
      return { id: source.id, fileName: source.fileName, kind: source.kind, records: existing?.records ?? [] };
    }));
  }, []);

  useEffect(() => {
    let cancelled = false;
    void listWorkspaceOntologies().then((backendOntologies) => {
      if (cancelled || !backendOntologies.length) return;
      const current = parseRegistry(registrySnapshot());
      for (const ontology of backendOntologies) {
        if (ontology.slug === defaultOntology.slug) {
          const oldKey = `context-hub.ontology.${defaultOntology.id}`;
          const newKey = `context-hub.ontology.${ontology.id}`;
          const oldDraft = localStorage.getItem(oldKey);
          if (oldDraft && !localStorage.getItem(newKey)) localStorage.setItem(newKey, oldDraft);
        }
      }
      const activeSlug = current.ontologies.find((item) => item.id === current.activeOntologyId)?.slug;
      const active = backendOntologies.find((item) => item.slug === activeSlug) ?? backendOntologies[0];
      saveRegistry({ ontologies: backendOntologies, activeOntologyId: active.id });
    }).catch(() => undefined);
    return () => { cancelled = true; };
  }, []);

  useEffect(() => {
    let cancelled = false;
    void listWorkspaceDataSources()
      .then((sources) => {
        if (cancelled) return;
        setBackendDataSources(sources);
        setDataSources(sources.map((source) => ({ id: source.id, fileName: source.fileName, kind: source.kind, records: [] })));
      })
      .catch(() => undefined);
    return () => { cancelled = true; };
  }, [refreshDataSources]);

  useEffect(() => {
    if (section !== "graph" || graphs[activeOntology.id] || !activeOntology.activeVersionId || !activeCatalog.objectTypes.length || graphRequests.current.has(activeOntology.id)) return;
    graphRequests.current.add(activeOntology.id);
    void loadPersistedGraph(activeOntology, activeCatalog)
      .then((loaded) => setGraphs((items) => ({ ...items, [activeOntology.id]: loaded })))
      .catch(() => undefined)
      .finally(() => graphRequests.current.delete(activeOntology.id));
  }, [activeCatalog, activeOntology, graphs, section]);

  async function createOntology() {
    const number = ontologies.length + 1;
    try {
      const ontology = await createWorkspaceOntology(`Ontology ${number}`, `ontology_${crypto.randomUUID().replaceAll("-", "").slice(0, 8)}`);
      saveRegistry({ ontologies: [...ontologies, ontology], activeOntologyId: ontology.id });
      setSection("ontology");
    } catch {
      // Keep the current registry unchanged when the backend cannot create the ontology.
    }
  }

  function renameOntology(name: string) {
    saveRegistry({ ontologies: ontologies.map((ontology) => ontology.id === activeOntology.id ? { ...ontology, name } : ontology), activeOntologyId });
  }

  function recordPublishedVersion(versionId: string) {
    saveRegistry({ ontologies: ontologies.map((ontology) => ontology.id === activeOntology.id ? { ...ontology, activeVersionId: versionId } : ontology), activeOntologyId });
  }

  function registerDataSource(source: BrowserDataSource) {
    setDataSources((items) => [...items.filter((item) => item.id !== source.id), source]);
    setActiveDataSourceId(source.id);
    void refreshDataSources();
  }

  async function selectDataSource(id: string) {
    if (!id) {
      setActiveDataSourceId("");
      return;
    }
    const existing = dataSources.find((source) => source.id === id);
    if (existing?.records.length) {
      setActiveDataSourceId(id);
      return;
    }
    try {
      const content = existing?.kind === "upload" && !/\.parquet$/i.test(existing.fileName) ? await downloadWorkspaceSource(id) : await previewWorkspaceSource(id);
      const hydrated: BrowserDataSource = {
        id: content.id,
        fileName: content.fileName,
        kind: existing?.kind ?? "upload",
        records: parseSource(content.fileName, new TextDecoder().decode(content.content)),
      };
      setDataSources((items) => items.map((source) => source.id === id ? hydrated : source));
      setActiveDataSourceId(id);
    } catch {
      setActiveDataSourceId("");
    }
  }

  async function loadMoreGraph() {
    const pagination = activeGraph.pagination;
    if (!pagination?.hasMore || loadingMoreGraph) return;
    const ontologyId = activeOntology.id;
    setLoadingMoreGraph(true);
    try {
      const page = await loadPersistedGraph(activeOntology, activeCatalog, pagination.cursors);
      setGraphs((items) => ({ ...items, [ontologyId]: mergeImportedGraphs(items[ontologyId] ?? emptyGraph, page) }));
    } finally {
      setLoadingMoreGraph(false);
    }
  }

  async function runGraphQuery(spec: GraphQuerySpec) {
    setQueryingGraph(true);
    try {
      const result = await queryPersistedGraph(activeOntology, activeCatalog, spec);
      setGraphs((items) => ({ ...items, [activeOntology.id]: result }));
    } finally {
      setQueryingGraph(false);
    }
  }

  async function expandGraphNode(node: ImportedGraph["nodes"][number]) {
    const objectType = activeCatalog.objectTypes.find((type) => type.displayName === node.kind || type.apiName === node.kind);
    if (!objectType) throw new Error(`Object type '${node.kind}' does not exist in the active ontology.`);
    const linkTypes = activeCatalog.linkTypes.filter((link) => link.sourceType === objectType.apiName || link.targetType === objectType.apiName).map((link) => link.apiName);
    setExpandingNodeId(node.id);
    try {
      const expansion = await expandPersistedGraphNode(activeOntology, activeCatalog, objectType.apiName, node.id, linkTypes);
      setGraphs((items) => ({ ...items, [activeOntology.id]: mergeImportedGraphs(items[activeOntology.id] ?? emptyGraph, expansion) }));
    } finally {
      setExpandingNodeId("");
    }
  }

  const loadNodeProvenance = useCallback(async (node: ImportedGraph["nodes"][number]) => {
    const objectType = activeCatalog.objectTypes.find((type) => type.displayName === node.kind || type.apiName === node.kind);
    if (!objectType || !activeOntology.activeVersionId) return [];
    return getObjectProvenance(activeOntology.activeVersionId, objectType.apiName, node.id);
  }, [activeCatalog.objectTypes, activeOntology.activeVersionId]);

  return (
    <main className="app-shell">
      <aside className="sidebar">
        <div className="brand"><span className="brand-mark"><Boxes size={19} /></span><span>ContextHub</span></div>
        <div className="workspace-card"><span className="eyebrow">Workspace</span><strong>Development</strong><span className="status"><i /> Connected</span></div>
        <div className="ontology-switcher"><div><span className="eyebrow">Ontology</span><button onClick={createOntology} title="Create ontology" aria-label="Create ontology"><Plus size={13}/></button></div><select aria-label="Active ontology" value={activeOntology.id} onChange={(event) => { saveRegistry({ ontologies, activeOntologyId: event.target.value }); setSection("ontology"); }}>{ontologies.map((ontology) => <option value={ontology.id} key={ontology.id}>{ontology.name}</option>)}</select><small>{ontologies.length} {ontologies.length === 1 ? "ontology" : "ontologies"} · shared data sources</small></div>
        <div className="ontology-switcher source-switcher"><div><span className="eyebrow">Workspace source</span><DatabaseZap size={13}/></div><select aria-label="Active data source" value={activeDataSourceId} disabled={!dataSources.length} onChange={(event) => void selectDataSource(event.target.value)}><option value="">{dataSources.length ? "Select source" : "Add a file or REST source"}</option>{dataSources.map((source) => <option value={source.id} key={source.id}>{source.fileName}</option>)}</select><small>{dataSources.length ? "Shared sources · ontology-specific mappings" : "Sources are shared; mappings stay isolated"}</small></div>
        <nav aria-label="Main navigation">
          {sections.map(({ id, label, icon: Icon }) => (
            <button className={section === id ? "nav-item active" : "nav-item"} key={id} onClick={() => setSection(id)}>
              <Icon size={17} /><span>{label}</span>
            </button>
          ))}
        </nav>
        <div className="sidebar-footer"><span className="eyebrow">Active ontology</span><strong>{activeOntology.name}</strong><span>{activeOntology.slug} · isolated graph</span></div>
      </aside>
      <section className="main-stage">
        <div className={section === "ontology" ? "section-pane active" : "section-pane"}><OntologyEditor key={activeOntology.id} ontologyId={activeOntology.id} ontologyName={activeOntology.name} ontologySlug={activeOntology.slug} seedTemplate={activeOntology.slug === defaultOntology.slug} onRename={renameOntology} onCatalogChange={updateActiveCatalog} onPublished={recordPublishedVersion} /></div>
        {section === "sources" && <DataSourceManager
          sources={backendDataSources}
          onChanged={refreshDataSources}
          onUseForMapping={(id) => { void selectDataSource(id); setSection("mapping"); }}
        />}
        {section === "mapping" && <MappingPanel key={`${activeOntology.id}:${activeDataSource?.id ?? "new"}`} ontologyId={activeOntology.id} ontologyName={activeOntology.name} ontologySlug={activeOntology.slug} ontology={activeCatalog} dataSource={activeDataSource} onDataSourceLoaded={registerDataSource} onImport={(imported) => { setGraphs((items) => ({ ...items, [activeOntology.id]: imported })); setSection("graph"); }} />}
        {section === "imports" && <ImportHistory
          ontologyId={activeOntology.id}
          sources={backendDataSources}
          onUseSource={(id) => { void selectDataSource(id); setSection("mapping"); }}
        />}
        {section === "graph" && <GraphExplorer graph={activeGraph} ontology={activeCatalog} onOpenMapping={() => setSection("mapping")} onOpenOntology={() => setSection("ontology")} onLoadMore={loadMoreGraph} onRunQuery={runGraphQuery} onExpandNode={expandGraphNode} onLoadProvenance={loadNodeProvenance} canLoadMore={!!activeGraph.pagination?.hasMore} loadingMore={loadingMoreGraph} queryBusy={queryingGraph} expandingNodeId={expandingNodeId} />}
      </section>
    </main>
  );
}

function mergeImportedGraphs(current: ImportedGraph, page: ImportedGraph): ImportedGraph {
  const nodes = new Map(current.nodes.map((node) => [node.id, node]));
  for (const node of page.nodes) nodes.set(node.id, node);
  const linkKey = (link: ImportedGraph["links"][number]) => `${String(link.source)}\u0000${link.label}\u0000${String(link.target)}`;
  const links = new Map(current.links.map((link) => [linkKey(link), link]));
  for (const link of page.links) links.set(linkKey(link), link);
  return {
    ...current,
    nodes: [...nodes.values()],
    links: [...links.values()],
    recordCount: nodes.size,
    importedAt: page.importedAt,
    pagination: page.pagination,
    aggregations: page.aggregations?.length ? page.aggregations : current.aggregations,
  };
}
