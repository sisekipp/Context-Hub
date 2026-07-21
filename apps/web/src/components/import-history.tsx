"use client";

import { useCallback, useEffect, useMemo, useState } from "react";
import { AlertTriangle, CheckCircle2, Clock3, DatabaseZap, FileWarning, RefreshCw, RotateCcw } from "lucide-react";
import { IngestionState } from "@/gen/context_hub/v1/context_hub_pb";
import {
  listIngestionEvents,
  listOntologyIngestionJobs,
  retryIngestionJob,
  type BackendDataSource,
  type BackendIngestionEvent,
  type BackendIngestionJob,
} from "@/lib/context-hub-client";

type Props = {
  ontologyId: string;
  sources: BackendDataSource[];
  onUseSource: (id: string) => void;
};

function stateLabel(state: IngestionState) {
  if (state === IngestionState.SUCCEEDED) return "Succeeded";
  if (state === IngestionState.FAILED) return "Failed";
  if (state === IngestionState.RUNNING) return "Running";
  if (state === IngestionState.QUEUED) return "Queued";
  return "Unknown";
}

function dateLabel(value: Date | null) {
  return value ? new Intl.DateTimeFormat("de-DE", { dateStyle: "medium", timeStyle: "medium" }).format(value) : "—";
}

function durationLabel(job: BackendIngestionJob) {
  if (!job.startedAt) return "—";
  const end = job.completedAt ?? new Date();
  const milliseconds = Math.max(0, end.getTime() - job.startedAt.getTime());
  return milliseconds < 1_000 ? `${milliseconds} ms` : `${(milliseconds / 1_000).toFixed(1)} s`;
}

function eventField(event: BackendIngestionEvent) {
  try {
    const details = JSON.parse(event.detailsJson) as { field?: string };
    return details.field ?? "";
  } catch {
    return "";
  }
}

export function ImportHistory({ ontologyId, sources, onUseSource }: Props) {
  const [jobs, setJobs] = useState<BackendIngestionJob[]>([]);
  const [selectedId, setSelectedId] = useState("");
  const [eventResult, setEventResult] = useState<{ jobId: string; events: BackendIngestionEvent[] }>({ jobId: "", events: [] });
  const [busy, setBusy] = useState(true);
  const [error, setError] = useState("");
  const selected = jobs.find((job) => job.id === selectedId) ?? jobs[0] ?? null;
  const events = selected?.id === eventResult.jobId ? eventResult.events : [];
  const sourceNames = useMemo(() => new Map(sources.map((source) => [source.id, source.name])), [sources]);

  const refresh = useCallback(async (preferredId?: string) => {
    setBusy(true);
    setError("");
    try {
      const loaded = await listOntologyIngestionJobs(ontologyId);
      setJobs(loaded);
      setSelectedId((current) => preferredId ?? (loaded.some((job) => job.id === current) ? current : loaded[0]?.id ?? ""));
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : "Import history could not be loaded.");
    } finally {
      setBusy(false);
    }
  }, [ontologyId]);

  useEffect(() => {
    const load = window.setTimeout(() => { void refresh(); }, 0);
    return () => window.clearTimeout(load);
  }, [refresh]);

  useEffect(() => {
    if (!selected?.id) {
      return;
    }
    let cancelled = false;
    void listIngestionEvents(selected.id)
      .then((loaded) => { if (!cancelled) setEventResult({ jobId: selected.id, events: loaded }); })
      .catch((cause) => { if (!cancelled) setError(cause instanceof Error ? cause.message : "Import events could not be loaded."); });
    return () => { cancelled = true; };
  }, [selected?.id]);

  async function retry() {
    if (!selected) return;
    setBusy(true);
    setError("");
    try {
      const retried = await retryIngestionJob(selected.id);
      await refresh(retried.id);
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : "The import could not be retried.");
      setBusy(false);
    }
  }

  return <div className="import-history">
    <header className="source-manager-header import-history-header">
      <div><span className="eyebrow">Ontology operations</span><h1><Clock3 size={22}/> Import history</h1><p>Inspect executions, rejected records, source lineage, and retry an import with the same mapping and ontology version.</p></div>
      <button className="button secondary" onClick={() => void refresh()} disabled={busy}><RefreshCw size={14}/> Refresh</button>
    </header>
    {error && <div className="import-history-error" role="alert"><AlertTriangle size={15}/>{error}</div>}
    {!jobs.length && !busy && <div className="source-empty"><DatabaseZap size={28}/><h2>No imports for this ontology</h2><p>Map a shared source to this ontology and run its first import.</p></div>}
    {!!jobs.length && <div className="import-history-layout">
      <section className="job-list" aria-label="Import jobs">
        {jobs.map((job) => <button className={selected?.id === job.id ? "job-card active" : "job-card"} key={job.id} onClick={() => setSelectedId(job.id)}>
          <span className={`job-state state-${stateLabel(job.state).toLowerCase()}`}>{job.state === IngestionState.SUCCEEDED ? <CheckCircle2 size={13}/> : job.state === IngestionState.FAILED ? <AlertTriangle size={13}/> : <Clock3 size={13}/>} {stateLabel(job.state)}</span>
          <strong>{sourceNames.get(job.dataSourceId) ?? "Unknown source"}</strong>
          <small>{dateLabel(job.createdAt)}</small>
          <span className="job-counts">{job.nodesWritten.toLocaleString("de-DE")} nodes · {job.edgesWritten.toLocaleString("de-DE")} links · {job.rowsRejected.toLocaleString("de-DE")} rejected</span>
        </button>)}
      </section>
      {selected && <section className="job-detail">
        <div className="job-detail-heading"><div><span className="eyebrow">Execution details</span><h2>{sourceNames.get(selected.dataSourceId) ?? "Unknown source"}</h2><code>{selected.id}</code></div><div><button className="button secondary" onClick={() => onUseSource(selected.dataSourceId)}><DatabaseZap size={14}/> Open mapping</button><button className="button primary" onClick={() => void retry()} disabled={busy || selected.state === IngestionState.RUNNING || selected.state === IngestionState.QUEUED}><RotateCcw size={14}/> Retry</button></div></div>
        <div className="job-metrics"><div><strong>{selected.rowsRead.toLocaleString("de-DE")}</strong><span>Rows read</span></div><div><strong>{selected.nodesWritten.toLocaleString("de-DE")}</strong><span>Nodes written</span></div><div><strong>{selected.edgesWritten.toLocaleString("de-DE")}</strong><span>Links written</span></div><div><strong>{selected.rowsRejected.toLocaleString("de-DE")}</strong><span>Rejected</span></div></div>
        <div className="job-metadata"><div><span>Created</span><strong>{dateLabel(selected.createdAt)}</strong></div><div><span>Started</span><strong>{dateLabel(selected.startedAt)}</strong></div><div><span>Completed</span><strong>{dateLabel(selected.completedAt)}</strong></div><div><span>Duration</span><strong>{durationLabel(selected)}</strong></div><div><span>Mapping</span><code>{selected.ontologyMappingId}</code></div><div><span>Ontology version</span><code>{selected.ontologyVersionId}</code></div></div>
        {selected.error && <div className="job-error"><AlertTriangle size={15}/><span>{selected.error}</span></div>}
        <div className="event-heading"><div><span className="eyebrow">Execution log</span><h3>{events.length} events</h3></div></div>
        <div className="event-list">
          {events.map((event, index) => <article className={`event-row event-${event.eventType}`} key={`${event.eventType}-${event.rowNumber}-${index}`}><span className="event-icon">{event.eventType === "completed" ? <CheckCircle2 size={14}/> : event.eventType === "failed" || event.eventType === "row_rejected" ? <FileWarning size={14}/> : <Clock3 size={14}/>}</span><div><strong>{event.eventType.replaceAll("_", " ")}</strong><span>{event.message}</span><small>{[event.objectType, eventField(event), event.rowNumber ? `row ${event.rowNumber}` : ""].filter(Boolean).join(" · ")}</small></div><time>{dateLabel(event.occurredAt)}</time></article>)}
          {!events.length && <p className="muted-copy">No detailed events were recorded for this execution.</p>}
        </div>
      </section>}
    </div>}
  </div>;
}
