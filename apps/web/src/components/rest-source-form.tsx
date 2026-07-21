"use client";

import { useState } from "react";
import { Globe2, Plus, Trash2, X } from "lucide-react";
import type { GraphValue } from "@/lib/graph-data";
import { previewWorkspaceSource, saveWorkspaceRestSource, type RestKeyValue, type RestSourceInput } from "@/lib/context-hub-client";

export type RestBrowserSource = {
  id: string;
  fileName: string;
  kind: "rest";
  records: Array<Record<string, GraphValue>>;
};

const emptyPair = (): RestKeyValue => ({ key: "", value: "" });
const initialSource: RestSourceInput = {
  name: "",
  url: "",
  recordPath: "",
  headers: [],
  query: [],
  pagination: "none",
  pageParameter: "page",
  pageStart: 1,
  pageSizeParameter: "limit",
  pageSize: 100,
  cursorParameter: "cursor",
  nextCursorPath: "meta.next_cursor",
  maxPages: 100,
  maxBytes: 32 * 1024 * 1024,
  timeoutSeconds: 30,
  retryAttempts: 2,
};

export function validateRestSourceInput(source: RestSourceInput) {
  if (!source.name.trim()) return "Enter a source name.";
  let url: URL;
  try {
    url = new URL(source.url);
  } catch {
    return "Enter a valid REST URL.";
  }
  if (!['http:', 'https:'].includes(url.protocol)) return "The REST URL must use HTTP or HTTPS.";
  if (source.pagination === "page" && (!source.pageParameter.trim() || source.pageStart < 0)) {
    return "Page pagination needs a parameter and a non-negative start page.";
  }
  if (source.pagination === "cursor" && (!source.cursorParameter.trim() || !source.nextCursorPath.trim())) {
    return "Cursor pagination needs a query parameter and next-cursor path.";
  }
  if (source.maxPages < 1 || source.maxPages > 1_000) return "Max pages must be between 1 and 1,000.";
  if (source.timeoutSeconds < 1 || source.timeoutSeconds > 60) return "Timeout must be between 1 and 60 seconds.";
  if (source.retryAttempts < 0 || source.retryAttempts > 5) return "Retries must be between 0 and 5.";
  return "";
}

function KeyValueEditor({ label, items, onChange }: { label: string; items: RestKeyValue[]; onChange: (items: RestKeyValue[]) => void }) {
  return <div className="rest-pairs">
    <div className="rest-pairs-title"><span>{label}</span><button type="button" onClick={() => onChange([...items, emptyPair()])}><Plus size={12}/> Add</button></div>
    {items.map((item, index) => <div className="rest-pair" key={`${label}-${index}`}>
      <input aria-label={`${label} key ${index + 1}`} placeholder="Key" value={item.key} onChange={(event) => onChange(items.map((entry, entryIndex) => entryIndex === index ? { ...entry, key: event.target.value } : entry))}/>
      <input aria-label={`${label} value ${index + 1}`} placeholder="Value" value={item.value} onChange={(event) => onChange(items.map((entry, entryIndex) => entryIndex === index ? { ...entry, value: event.target.value } : entry))}/>
      <button type="button" aria-label={`Remove ${label} ${index + 1}`} onClick={() => onChange(items.filter((_, entryIndex) => entryIndex !== index))}><Trash2 size={12}/></button>
    </div>)}
    {!items.length && <small>No {label.toLowerCase()} configured.</small>}
  </div>;
}

export function RestSourceForm({ onClose, onCreated, onSaved, initialValue }: { onClose: () => void; onCreated?: (source: RestBrowserSource) => void; onSaved?: () => void; initialValue?: RestSourceInput }) {
  const editing = !!initialValue?.id;
  const [source, setSource] = useState(() => initialValue ?? initialSource);
  const [busy, setBusy] = useState(false);
  const [message, setMessage] = useState("");
  const update = (patch: Partial<RestSourceInput>) => setSource((current) => ({ ...current, ...patch }));

  async function submit(event: React.FormEvent) {
    event.preventDefault();
    const error = validateRestSourceInput(source);
    if (error) { setMessage(error); return; }
    setBusy(true);
    setMessage("Saving source and loading a bounded preview…");
    try {
      const saved = await saveWorkspaceRestSource(source);
      if (!onCreated) {
        onSaved?.();
        onClose();
        return;
      }
      const preview = await previewWorkspaceSource(saved.id);
      const value = JSON.parse(new TextDecoder().decode(preview.content)) as unknown;
      if (!Array.isArray(value)) throw new Error("The REST preview did not return a record array.");
      const records = value.filter((record): record is Record<string, GraphValue> => typeof record === "object" && record !== null && !Array.isArray(record));
      if (!records.length) throw new Error("The REST source did not return object records.");
      onCreated({ id: saved.id, fileName: saved.fileName, kind: "rest", records });
      onClose();
    } catch (error) {
      setMessage(error instanceof Error ? error.message : "The REST source could not be created.");
    } finally {
      setBusy(false);
    }
  }

  return <div className="rest-source-overlay" role="presentation">
    <form className="rest-source-dialog" onSubmit={submit} aria-label="REST data source">
      <div className="rest-dialog-header"><div><span className="eyebrow">Workspace source</span><h2><Globe2 size={18}/> {editing ? "Edit REST API" : "Connect REST API"}</h2><p>Fetch bounded JSON through the secured backend connector.</p></div><button type="button" aria-label="Close REST source" onClick={onClose}><X size={17}/></button></div>
      <div className="rest-form-grid">
        <label>Source name<input autoFocus value={source.name} onChange={(event) => update({ name: event.target.value })} placeholder="Service catalog API"/></label>
        <label className="wide">GET URL<input value={source.url} onChange={(event) => update({ url: event.target.value })} placeholder="https://api.example.com/services"/></label>
        <label>Record path<input value={source.recordPath} onChange={(event) => update({ recordPath: event.target.value })} placeholder="data.items or /data/items"/></label>
        <label>Pagination<select value={source.pagination} onChange={(event) => update({ pagination: event.target.value as RestSourceInput["pagination"] })}><option value="none">None</option><option value="page">Page number</option><option value="cursor">Cursor</option></select></label>
        {source.pagination === "page" && <>
          <label>Page parameter<input value={source.pageParameter} onChange={(event) => update({ pageParameter: event.target.value })}/></label>
          <label>Start page<input type="number" min={0} value={source.pageStart} onChange={(event) => update({ pageStart: Number(event.target.value) })}/></label>
          <label>Page-size parameter<input value={source.pageSizeParameter} onChange={(event) => update({ pageSizeParameter: event.target.value })}/></label>
          <label>Page size<input type="number" min={1} value={source.pageSize} onChange={(event) => update({ pageSize: Number(event.target.value) })}/></label>
        </>}
        {source.pagination === "cursor" && <>
          <label>Cursor parameter<input value={source.cursorParameter} onChange={(event) => update({ cursorParameter: event.target.value })}/></label>
          <label>Next cursor path<input value={source.nextCursorPath} onChange={(event) => update({ nextCursorPath: event.target.value })}/></label>
        </>}
        <label>Max pages<input type="number" min={1} max={1000} value={source.maxPages} onChange={(event) => update({ maxPages: Number(event.target.value) })}/></label>
        <label>Timeout seconds<input type="number" min={1} max={60} value={source.timeoutSeconds} onChange={(event) => update({ timeoutSeconds: Number(event.target.value) })}/></label>
        <label>Retries<input type="number" min={0} max={5} value={source.retryAttempts} onChange={(event) => update({ retryAttempts: Number(event.target.value) })}/></label>
        <label>Max response MiB<input type="number" min={1} max={64} value={Math.round(source.maxBytes / 1024 / 1024)} onChange={(event) => update({ maxBytes: Number(event.target.value) * 1024 * 1024 })}/></label>
      </div>
      <div className="rest-collections"><KeyValueEditor label="Query parameters" items={source.query} onChange={(query) => update({ query })}/><KeyValueEditor label="Headers" items={source.headers} onChange={(headers) => update({ headers })}/></div>
      <div className="rest-security-note">Private networks, unsafe redirects, oversized responses, and plaintext credential headers are rejected by the backend.</div>
      {message && <div className="import-status" role="status">{message}</div>}
      <div className="rest-dialog-actions"><button type="button" className="button secondary" onClick={onClose}>Cancel</button><button type="submit" className="button primary" disabled={busy}>{busy ? "Saving…" : editing ? "Save changes" : "Save & preview"}</button></div>
    </form>
  </div>;
}
