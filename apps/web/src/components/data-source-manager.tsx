"use client";

import { useEffect, useState } from "react";
import { Braces, Database, FileJson, FlaskConical, Globe2, Pencil, Route, Save, Trash2, X } from "lucide-react";
import { GraphqlSourceForm } from "@/components/graphql-source-form";
import { RestSourceForm } from "@/components/rest-source-form";
import {
  deleteWorkspaceDataSource,
  getWorkspaceDataSourceUsage,
  graphqlSourceInput,
  previewWorkspaceSource,
  renameWorkspaceDataSource,
  restSourceInput,
  type BackendDataSource,
  type BackendDataSourceUsage,
} from "@/lib/context-hub-client";

type Props = {
  sources: BackendDataSource[];
  onChanged: () => Promise<void>;
  onUseForMapping: (id: string) => void;
};

export function DataSourceManager({ sources, onChanged, onUseForMapping }: Props) {
  const [usages, setUsages] = useState<Record<string, BackendDataSourceUsage[]>>({});
  const [editing, setEditing] = useState<BackendDataSource | null>(null);
  const [renaming, setRenaming] = useState<BackendDataSource | null>(null);
  const [rename, setRename] = useState("");
  const [confirmDelete, setConfirmDelete] = useState("");
  const [busy, setBusy] = useState("");
  const [messages, setMessages] = useState<Record<string, string>>({});

  useEffect(() => {
    let cancelled = false;
    void Promise.all(sources.map(async (source) => [source.id, await getWorkspaceDataSourceUsage(source.id)] as const))
      .then((entries) => { if (!cancelled) setUsages(Object.fromEntries(entries)); })
      .catch(() => undefined);
    return () => { cancelled = true; };
  }, [sources]);

  async function test(source: BackendDataSource) {
    setBusy(source.id);
    setMessages((current) => ({ ...current, [source.id]: "Testing bounded preview…" }));
    try {
      const preview = await previewWorkspaceSource(source.id);
      setMessages((current) => ({ ...current, [source.id]: `${preview.recordCount.toLocaleString()} records available in the preview.` }));
    } catch (error) {
      setMessages((current) => ({ ...current, [source.id]: error instanceof Error ? error.message : "The source test failed." }));
    } finally {
      setBusy("");
    }
  }

  async function saveRename(source: BackendDataSource) {
    if (!rename.trim()) return;
    setBusy(source.id);
    try {
      await renameWorkspaceDataSource(source, rename.trim());
      setRenaming(null);
      await onChanged();
    } catch (error) {
      setMessages((current) => ({ ...current, [source.id]: error instanceof Error ? error.message : "The source could not be renamed." }));
    } finally {
      setBusy("");
    }
  }

  async function remove(source: BackendDataSource) {
    setBusy(source.id);
    try {
      await deleteWorkspaceDataSource(source.id);
      setConfirmDelete("");
      await onChanged();
    } catch (error) {
      setMessages((current) => ({ ...current, [source.id]: error instanceof Error ? error.message : "The source could not be deleted." }));
    } finally {
      setBusy("");
    }
  }

  function edit(source: BackendDataSource) {
    try {
      setEditing(source);
    } catch (error) {
      setMessages((current) => ({ ...current, [source.id]: error instanceof Error ? error.message : "The source configuration is invalid." }));
    }
  }

  return <div className="source-manager">
    <header className="source-manager-header">
      <div><span className="eyebrow">Workspace catalog</span><h1><Database size={22}/> Data sources</h1><p>Sources are shared across the workspace. Every mapping and imported graph remains isolated inside its ontology.</p></div>
      <div className="source-summary"><strong>{sources.length}</strong><span>saved sources</span></div>
    </header>
    {!sources.length && <div className="source-empty"><Database size={28}/><h2>No data sources yet</h2><p>Create a file, REST, or GraphQL source in Data mapping. It will appear here for all ontologies.</p><button className="button primary" onClick={() => onUseForMapping("")}>Open data mapping</button></div>}
    <div className="source-grid">
      {sources.map((source) => {
        const sourceUsages = usages[source.id] ?? [];
        const Icon = source.kind === "rest" ? Globe2 : source.kind === "graphql" ? Braces : FileJson;
        const deleting = confirmDelete === source.id;
        return <article className="source-card" key={source.id}>
          <div className={`source-kind ${source.kind}`}><Icon size={18}/></div>
          <div className="source-card-body">
            <div className="source-card-title">
              {renaming?.id === source.id ? <div className="source-rename"><input autoFocus aria-label="New source name" value={rename} onChange={(event) => setRename(event.target.value)} onKeyDown={(event) => { if (event.key === "Enter") void saveRename(source); }}/><button aria-label="Save source name" onClick={() => void saveRename(source)}><Save size={14}/></button><button aria-label="Cancel rename" onClick={() => setRenaming(null)}><X size={14}/></button></div> : <><div><h2>{source.name}</h2><span>{source.fileName}</span></div><button aria-label={`Rename ${source.name}`} onClick={() => { setRenaming(source); setRename(source.name); }}><Pencil size={14}/></button></>}
            </div>
            <div className="source-usage">
              <div><Route size={14}/><strong>{sourceUsages.length ? `Used by ${sourceUsages.length} mapping${sourceUsages.length === 1 ? "" : "s"}` : "Not used by an ontology"}</strong></div>
              {sourceUsages.map((usage) => <span key={usage.mappingId}>{usage.ontologyName} · {usage.mappingName}</span>)}
            </div>
            {messages[source.id] && <div className="source-message" role="status">{messages[source.id]}</div>}
            {deleting && <div className="source-delete-confirm"><span>Delete this source permanently?</span><button className="button danger" disabled={busy === source.id} onClick={() => void remove(source)}>Delete</button><button className="button secondary" onClick={() => setConfirmDelete("")}>Cancel</button></div>}
            <div className="source-actions">
              <button onClick={() => void test(source)} disabled={busy === source.id}><FlaskConical size={14}/> Test</button>
              {source.kind !== "upload" && <button onClick={() => edit(source)}><Pencil size={14}/> Edit</button>}
              <button onClick={() => onUseForMapping(source.id)}><Route size={14}/> Map</button>
              <button className="delete" title={sourceUsages.length ? "Remove its ontology mappings before deleting this source" : "Delete source"} disabled={sourceUsages.length > 0} onClick={() => setConfirmDelete(source.id)}><Trash2 size={14}/> Delete</button>
            </div>
          </div>
        </article>;
      })}
    </div>
    {editing?.kind === "rest" && <RestSourceForm initialValue={restSourceInput(editing)} onClose={() => setEditing(null)} onSaved={() => void onChanged()}/>} 
    {editing?.kind === "graphql" && <GraphqlSourceForm initialValue={graphqlSourceInput(editing)} onClose={() => setEditing(null)} onSaved={() => void onChanged()}/>} 
  </div>;
}
