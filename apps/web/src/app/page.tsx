"use client";

import { useState } from "react";
import { Boxes, DatabaseZap, Orbit, Workflow } from "lucide-react";
import { GraphExplorer } from "@/components/graph-explorer";
import { MappingPanel } from "@/components/mapping-panel";
import { OntologyEditor } from "@/components/ontology-editor";
import { emptyGraph, type ImportedGraph } from "@/lib/graph-data";
import { defaultOntologyCatalog, type OntologyCatalog } from "@/lib/ontology-catalog";

type Section = "ontology" | "mapping" | "graph";

const sections: Array<{ id: Section; label: string; icon: typeof Workflow }> = [
  { id: "ontology", label: "Ontology", icon: Workflow },
  { id: "mapping", label: "Data mapping", icon: DatabaseZap },
  { id: "graph", label: "Explore", icon: Orbit },
];

export default function Home() {
  const [section, setSection] = useState<Section>("ontology");
  const [graph, setGraph] = useState<ImportedGraph>(emptyGraph);
  const [ontology, setOntology] = useState<OntologyCatalog>(defaultOntologyCatalog);

  return (
    <main className="app-shell">
      <aside className="sidebar">
        <div className="brand"><span className="brand-mark"><Boxes size={19} /></span><span>ContextHub</span></div>
        <div className="workspace-card"><span className="eyebrow">Workspace</span><strong>Development</strong><span className="status"><i /> Connected</span></div>
        <nav aria-label="Main navigation">
          {sections.map(({ id, label, icon: Icon }) => (
            <button className={section === id ? "nav-item active" : "nav-item"} key={id} onClick={() => setSection(id)}>
              <Icon size={17} /><span>{label}</span>
            </button>
          ))}
        </nav>
        <div className="sidebar-footer"><span className="eyebrow">Ontology</span><strong>Service map</strong><span>Draft · revision 4</span></div>
      </aside>
      <section className="main-stage">
        <div className={section === "ontology" ? "section-pane active" : "section-pane"}><OntologyEditor onCatalogChange={setOntology} /></div>
        <div className={section === "mapping" ? "section-pane active" : "section-pane"}><MappingPanel ontology={ontology} onImport={(imported) => { setGraph(imported); setSection("graph"); }} /></div>
        {section === "graph" && <GraphExplorer graph={graph} onOpenMapping={() => setSection("mapping")} onOpenOntology={() => setSection("ontology")} />}
      </section>
    </main>
  );
}
