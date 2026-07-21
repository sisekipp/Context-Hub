"use client";

import { useCallback, useEffect, useState } from "react";
import { FileCode2, FileUp, History, PlugZap, RefreshCw, Trash2 } from "lucide-react";
import { FunctionExecutionState } from "@/gen/context_hub/v1/context_hub_pb";
import {
  deleteFunctionArtifact,
  executeOntologyFunction,
  listFunctionArtifacts,
  listFunctionExecutions,
  testExternalFunctionProvider,
  uploadFunctionArtifact,
  type BackendFunctionArtifact,
  type BackendFunctionExecution,
} from "@/lib/context-hub-client";

type Props = {
  publishedVersionId: string;
  functionApiName: string;
  implementation: "expression" | "external_grpc" | "wasm";
  endpoint: string;
  method: string;
  artifactUri: string;
  onArtifactSelect: (uri: string) => void;
};

function dateLabel(value: Date | null) {
  return value?.toLocaleString("de-DE") ?? "—";
}

export function FunctionTools({ publishedVersionId, functionApiName, implementation, endpoint, method, artifactUri, onArtifactSelect }: Props) {
  const [argumentsJson, setArgumentsJson] = useState("{}");
  const [result, setResult] = useState("");
  const [artifacts, setArtifacts] = useState<BackendFunctionArtifact[]>([]);
  const [executions, setExecutions] = useState<BackendFunctionExecution[]>([]);
  const [busy, setBusy] = useState("");

  const refreshArtifacts = useCallback(async () => {
    setArtifacts(await listFunctionArtifacts());
  }, []);

  const refreshExecutions = useCallback(async () => {
    if (!publishedVersionId) {
      setExecutions([]);
      return;
    }
    setExecutions(await listFunctionExecutions(publishedVersionId, functionApiName));
  }, [functionApiName, publishedVersionId]);

  useEffect(() => {
    const timer = window.setTimeout(() => { void refreshArtifacts().catch(() => undefined); }, 0);
    return () => window.clearTimeout(timer);
  }, [refreshArtifacts]);

  useEffect(() => {
    const timer = window.setTimeout(() => { void refreshExecutions().catch(() => undefined); }, 0);
    return () => window.clearTimeout(timer);
  }, [refreshExecutions]);

  async function run() {
    if (!publishedVersionId) return;
    setBusy("run"); setResult("");
    try {
      JSON.parse(argumentsJson);
      const response = await executeOntologyFunction(publishedVersionId, functionApiName, argumentsJson);
      setResult(`${response.resultJson}\n\n${response.executor} · ${response.durationMillis} ms`);
    } catch (error) {
      setResult(error instanceof Error ? error.message : "Function execution failed.");
    } finally {
      setBusy("");
      await refreshExecutions().catch(() => undefined);
    }
  }

  async function testProvider() {
    setBusy("provider"); setResult("");
    try {
      JSON.parse(argumentsJson);
      const response = await testExternalFunctionProvider(endpoint, method, functionApiName, argumentsJson);
      setResult(`${response.resultJson}\n\nprovider check · ${response.durationMillis} ms`);
    } catch (error) {
      setResult(error instanceof Error ? error.message : "Provider test failed.");
    } finally { setBusy(""); }
  }

  async function upload(file: File) {
    setBusy("upload"); setResult("");
    try {
      const artifact = await uploadFunctionArtifact(file);
      await refreshArtifacts();
      onArtifactSelect(artifact.artifactUri);
      setResult(`Uploaded ${artifact.fileName} · ${artifact.artifactUri}`);
    } catch (error) {
      setResult(error instanceof Error ? error.message : "WASM upload failed.");
    } finally { setBusy(""); }
  }

  async function removeArtifact(artifact: BackendFunctionArtifact) {
    setBusy(artifact.id); setResult("");
    try {
      await deleteFunctionArtifact(artifact.id);
      await refreshArtifacts();
      setResult(`Deleted ${artifact.fileName}.`);
    } catch (error) {
      setResult(error instanceof Error ? error.message : "Artifact deletion failed.");
    } finally { setBusy(""); }
  }

  return <>
    {implementation === "wasm" && <div className="inspector-section function-assets">
      <div className="section-title"><span>WASM artifacts</span><label className="function-upload"><FileUp size={13}/> Upload<input type="file" accept=".wasm,application/wasm" disabled={!!busy} onChange={(event) => { const file = event.target.files?.[0]; if (file) void upload(file); event.target.value = ""; }}/></label></div>
      <div className="artifact-list">{artifacts.map((artifact) => <div className={artifact.artifactUri === artifactUri ? "artifact-row active" : "artifact-row"} key={artifact.id}><button onClick={() => onArtifactSelect(artifact.artifactUri)}><FileCode2 size={13}/><span><strong>{artifact.name}</strong><small>{(artifact.sizeBytes / 1024).toFixed(1)} KiB · {artifact.sha256.slice(0, 10)}…</small></span></button><button aria-label={`Delete artifact ${artifact.name}`} disabled={busy === artifact.id} onClick={() => void removeArtifact(artifact)}><Trash2 size={12}/></button></div>)}{!artifacts.length && <p className="muted-copy">No WASM artifacts uploaded.</p>}</div>
    </div>}
    <div className="inspector-section function-runner">
      <div className="section-title"><span>Test function</span></div>
      <label>Arguments JSON<textarea aria-label="Function arguments JSON" value={argumentsJson} onChange={(event) => setArgumentsJson(event.target.value)}/></label>
      <button className="button primary" disabled={!publishedVersionId || !!busy} onClick={() => void run()}>{busy === "run" ? "Running…" : "Run published version"}</button>
      {implementation === "external_grpc" && <button className="button secondary" disabled={!endpoint || !!busy} onClick={() => void testProvider()}><PlugZap size={13}/>{busy === "provider" ? "Testing…" : "Test provider without publish"}</button>}
      {!publishedVersionId && <small>Publish the current ontology before running this function.</small>}
      {result && <pre aria-label="Function result">{result}</pre>}
    </div>
    <div className="inspector-section function-history">
      <div className="section-title"><span><History size={12}/> Execution history</span><button aria-label="Refresh function history" disabled={!publishedVersionId} onClick={() => void refreshExecutions()}><RefreshCw size={12}/></button></div>
      {executions.map((execution) => <details key={execution.id}><summary><span className={execution.state === FunctionExecutionState.SUCCEEDED ? "execution-ok" : "execution-failed"}>{execution.state === FunctionExecutionState.SUCCEEDED ? "Succeeded" : "Failed"}</span><strong>{execution.executor}</strong><small>{execution.durationMillis} ms · {dateLabel(execution.executedAt)}</small></summary><code>{execution.id}</code><pre>{execution.error || execution.resultJson}</pre><small>Arguments: {execution.argumentsJson}</small></details>)}
      {!executions.length && <p className="muted-copy">No executions for this published function.</p>}
    </div>
  </>;
}
