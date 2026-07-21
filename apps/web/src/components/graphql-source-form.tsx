"use client";

import { useState, type FormEvent } from "react";
import { Braces, Plus, Trash2, X } from "lucide-react";
import type { GraphValue } from "@/lib/graph-data";
import { previewWorkspaceSource, saveWorkspaceGraphqlSource, type GraphqlSourceInput, type RestKeyValue } from "@/lib/context-hub-client";

export type GraphqlBrowserSource = {
  id: string;
  fileName: string;
  kind: "graphql";
  records: Array<Record<string, GraphValue>>;
};

const initialSource: GraphqlSourceInput = {
  name: "",
  url: "",
  query: "query Services {\n  services {\n    id\n    name\n  }\n}",
  variables: "{}",
  recordPath: "data.services",
  headers: [],
  cursorEnabled: false,
  cursorVariable: "after",
  nextCursorPath: "data.services.pageInfo.endCursor",
  maxPages: 100,
  maxBytes: 32 * 1024 * 1024,
  timeoutSeconds: 30,
  retryAttempts: 2,
};

export function validateGraphqlSourceInput(source: GraphqlSourceInput) {
  if (!source.name.trim()) return "Enter a source name.";
  try {
    const url = new URL(source.url);
    if (!["http:", "https:"].includes(url.protocol)) return "The GraphQL URL must use HTTP or HTTPS.";
  } catch {
    return "Enter a valid GraphQL URL.";
  }
  if (!source.query.trim()) return "Enter a GraphQL query.";
  if (!source.recordPath.trim()) return "Enter the response path containing the records.";
  try {
    const variables = JSON.parse(source.variables || "{}");
    if (!variables || Array.isArray(variables) || typeof variables !== "object") return "Variables must be a JSON object.";
  } catch {
    return "Variables must contain valid JSON.";
  }
  if (source.cursorEnabled && (!/^[_A-Za-z][_0-9A-Za-z]*$/.test(source.cursorVariable) || !source.nextCursorPath.trim())) {
    return "Cursor pagination needs a valid variable and next-cursor path.";
  }
  if (source.maxPages < 1 || source.maxPages > 1_000) return "Max pages must be between 1 and 1,000.";
  if (source.timeoutSeconds < 1 || source.timeoutSeconds > 60) return "Timeout must be between 1 and 60 seconds.";
  if (source.retryAttempts < 0 || source.retryAttempts > 5) return "Retries must be between 0 and 5.";
  return "";
}

export function GraphqlSourceForm({ onClose, onCreated, onSaved, initialValue }: { onClose: () => void; onCreated?: (source: GraphqlBrowserSource) => void; onSaved?: () => void; initialValue?: GraphqlSourceInput }) {
  const editing = !!initialValue?.id;
  const [source, setSource] = useState(() => initialValue ?? initialSource);
  const [busy, setBusy] = useState(false);
  const [message, setMessage] = useState("");
  const update = (patch: Partial<GraphqlSourceInput>) => setSource((current) => ({ ...current, ...patch }));

  async function submit(event: FormEvent) {
    event.preventDefault();
    const error = validateGraphqlSourceInput(source);
    if (error) { setMessage(error); return; }
    setBusy(true);
    setMessage("Saving source and executing a bounded GraphQL preview…");
    try {
      const saved = await saveWorkspaceGraphqlSource(source);
      if (!onCreated) {
        onSaved?.();
        onClose();
        return;
      }
      const preview = await previewWorkspaceSource(saved.id);
      const value = JSON.parse(new TextDecoder().decode(preview.content)) as unknown;
      if (!Array.isArray(value)) throw new Error("The GraphQL preview did not return a record array.");
      const records = value.filter((record): record is Record<string, GraphValue> => typeof record === "object" && record !== null && !Array.isArray(record));
      if (!records.length) throw new Error("The GraphQL query did not return object records.");
      onCreated({ id: saved.id, fileName: saved.fileName, kind: "graphql", records });
      onClose();
    } catch (error) {
      setMessage(error instanceof Error ? error.message : "The GraphQL source could not be created.");
    } finally {
      setBusy(false);
    }
  }

  function updateHeader(index: number, patch: Partial<RestKeyValue>) {
    update({ headers: source.headers.map((header, headerIndex) => headerIndex === index ? { ...header, ...patch } : header) });
  }

  return <div className="rest-source-overlay" role="presentation">
    <form className="rest-source-dialog graphql-source-dialog" onSubmit={submit} aria-label="GraphQL data source">
      <div className="rest-dialog-header"><div><span className="eyebrow">Workspace source</span><h2><Braces size={18}/> {editing ? "Edit GraphQL" : "Connect GraphQL"}</h2><p>Execute a controlled query through the secured backend connector.</p></div><button type="button" aria-label="Close GraphQL source" onClick={onClose}><X size={17}/></button></div>
      <div className="rest-form-grid">
        <label>Source name<input autoFocus value={source.name} onChange={(event) => update({ name: event.target.value })} placeholder="Service catalog GraphQL"/></label>
        <label className="wide">GraphQL URL<input value={source.url} onChange={(event) => update({ url: event.target.value })} placeholder="https://api.example.com/graphql"/></label>
        <label>Record path<input value={source.recordPath} onChange={(event) => update({ recordPath: event.target.value })} placeholder="data.services.nodes"/></label>
        <label className="graphql-query-field">Query<textarea value={source.query} onChange={(event) => update({ query: event.target.value })} spellCheck={false}/></label>
        <label className="graphql-variables-field">Variables JSON<textarea value={source.variables} onChange={(event) => update({ variables: event.target.value })} spellCheck={false}/></label>
        <label className="graphql-cursor-toggle"><input type="checkbox" checked={source.cursorEnabled} onChange={(event) => update({ cursorEnabled: event.target.checked })}/> Cursor pagination</label>
        {source.cursorEnabled && <><label>Cursor variable<input value={source.cursorVariable} onChange={(event) => update({ cursorVariable: event.target.value })}/></label><label className="wide">Next cursor path<input value={source.nextCursorPath} onChange={(event) => update({ nextCursorPath: event.target.value })}/></label></>}
        <label>Max pages<input type="number" min={1} max={1000} value={source.maxPages} onChange={(event) => update({ maxPages: Number(event.target.value) })}/></label>
        <label>Timeout seconds<input type="number" min={1} max={60} value={source.timeoutSeconds} onChange={(event) => update({ timeoutSeconds: Number(event.target.value) })}/></label>
        <label>Retries<input type="number" min={0} max={5} value={source.retryAttempts} onChange={(event) => update({ retryAttempts: Number(event.target.value) })}/></label>
        <label>Max response MiB<input type="number" min={1} max={64} value={Math.round(source.maxBytes / 1024 / 1024)} onChange={(event) => update({ maxBytes: Number(event.target.value) * 1024 * 1024 })}/></label>
      </div>
      <div className="rest-pairs graphql-headers"><div className="rest-pairs-title"><span>Headers</span><button type="button" onClick={() => update({ headers: [...source.headers, { key: "", value: "" }] })}><Plus size={12}/> Add</button></div>{source.headers.map((header, index) => <div className="rest-pair" key={`graphql-header-${index}`}><input aria-label={`GraphQL header key ${index + 1}`} placeholder="Key" value={header.key} onChange={(event) => updateHeader(index, { key: event.target.value })}/><input aria-label={`GraphQL header value ${index + 1}`} placeholder="Value" value={header.value} onChange={(event) => updateHeader(index, { value: event.target.value })}/><button type="button" aria-label={`Remove GraphQL header ${index + 1}`} onClick={() => update({ headers: source.headers.filter((_, headerIndex) => headerIndex !== index) })}><Trash2 size={12}/></button></div>)}{!source.headers.length && <small>No headers configured.</small>}</div>
      <div className="rest-security-note">Only the configured query and variables are sent. Private networks, unsafe redirects, oversized responses, and plaintext credentials are rejected.</div>
      {message && <div className="import-status" role="status">{message}</div>}
      <div className="rest-dialog-actions"><button type="button" className="button secondary" onClick={onClose}>Cancel</button><button type="submit" className="button primary" disabled={busy}>{busy ? "Saving…" : editing ? "Save changes" : "Save & preview"}</button></div>
    </form>
  </div>;
}
