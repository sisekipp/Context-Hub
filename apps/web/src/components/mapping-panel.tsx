"use client";

import { ChangeEvent, useMemo, useState } from "react";
import { ArrowRight, Braces, CheckCircle2, FileJson2, GitBranch, Globe2, Play, Plus, Save, Trash2, Upload } from "lucide-react";
import type { GraphValue, ImportedGraph } from "@/lib/graph-data";
import type { OntologyCatalog, OntologyObjectType } from "@/lib/ontology-catalog";
import { IngestionState } from "@/gen/context_hub/v1/context_hub_pb";
import { publishOntologyCatalog, saveOntologyMapping, startIngestion, uploadWorkspaceSource } from "@/lib/context-hub-client";
import { RestSourceForm } from "@/components/rest-source-form";
import { GraphqlSourceForm } from "@/components/graphql-source-form";

export type SourceRecord = Record<string, GraphValue>;
export type BrowserDataSource = { id: string; fileName: string; kind: "upload" | "rest" | "graphql"; records: SourceRecord[] };
export type Transform = "None" | "Trim" | "Lowercase" | "Uppercase";
export type PropertyMapping = { id: string; sourceField: string; targetProperty: string; transform: Transform };
export type ObjectMapping = { id: string; objectType: string; displayProperty: string; properties: PropertyMapping[] };
export type LinkMapping = { id: string; sourceObjectMappingId: string; sourceField: string; linkType: string; missingTarget: "create" | "skip" | "error" };

const colors = ["#7c9cff", "#5ed3b5", "#f7b267", "#c792ea", "#ff7d9d"];
const newId = () => crypto.randomUUID();

function parseCsv(text: string): SourceRecord[] {
  const rows: string[][] = [];
  let row: string[] = [], value = "", quoted = false;
  for (let index = 0; index <= text.length; index += 1) {
    const char = text[index] ?? "\n";
    if (char === '"' && quoted && text[index + 1] === '"') { value += '"'; index += 1; }
    else if (char === '"') quoted = !quoted;
    else if (char === "," && !quoted) { row.push(value); value = ""; }
    else if ((char === "\n" || char === "\r") && !quoted) {
      if (char === "\r" && text[index + 1] === "\n") index += 1;
      row.push(value); value = "";
      if (row.some((cell) => cell.length)) rows.push(row);
      row = [];
    } else value += char;
  }
  const [headers = [], ...values] = rows;
  return values.map((cells) => Object.fromEntries(headers.map((header, index) => [header.trim(), cells[index] ?? null])));
}

function asRecords(value: unknown): SourceRecord[] {
  if (Array.isArray(value)) return value.filter((item): item is SourceRecord => typeof item === "object" && item !== null && !Array.isArray(item));
  if (typeof value === "object" && value !== null) {
    const object = value as Record<string, unknown>;
    for (const key of ["records", "data", "items", "results"]) if (Array.isArray(object[key])) return asRecords(object[key]);
    return [object as SourceRecord];
  }
  return [];
}

export function parseSource(fileName: string, text: string): SourceRecord[] {
  if (fileName.toLowerCase().endsWith(".csv")) return parseCsv(text);
  if (/\.(ndjson|jsonl)$/i.test(fileName)) return text.split(/\r?\n/).filter(Boolean).map((line) => JSON.parse(line) as SourceRecord);
  return asRecords(JSON.parse(text));
}

function detectType(records: SourceRecord[], field: string) {
  const value = records.find((record) => record[field] !== null && record[field] !== undefined)?.[field];
  if (Array.isArray(value)) return "List";
  if (value && typeof value === "object") return "Struct";
  return value === null || value === undefined ? "Unknown" : typeof value === "number" ? "Number" : typeof value === "boolean" ? "Boolean" : "String";
}

function applyTransform(value: GraphValue, transform: Transform): GraphValue {
  if (typeof value !== "string") return value;
  if (transform === "Trim") return value.trim();
  if (transform === "Lowercase") return value.toLowerCase();
  if (transform === "Uppercase") return value.toUpperCase();
  return value;
}

function matchingField(fields: string[], property: string, objectType: string) {
  return fields.find((field) => field === property)
    ?? fields.find((field) => field === `${objectType}_${property}`)
    ?? ((property === "id" || property === "name") ? fields.find((field) => field.endsWith(`_${objectType}`)) : undefined)
    ?? fields.find((field) => field.endsWith(`_${property}`))
    ?? "";
}

function matchingLinkField(fields: string[], link: OntologyCatalog["linkTypes"][number]) {
  return fields.find((field) => field === link.apiName)
    ?? fields.find((field) => field.endsWith(`_${link.targetType}`))
    ?? fields.find((field) => field.includes(link.targetType));
}

function makeObjectMapping(type: OntologyObjectType, fields: string[], records: SourceRecord[]): ObjectMapping {
  const properties = type.properties.filter((property) => !property.derived).map((property) => {
    const sourceField = matchingField(fields, property.apiName, type.apiName);
    return { id: newId(), sourceField, targetProperty: property.apiName, transform: sourceField && typeof records[0]?.[sourceField] === "string" ? "Trim" as const : "None" as const };
  }).filter((mapping) => mapping.sourceField);
  return { id: newId(), objectType: type.apiName, displayProperty: type.properties.find((property) => property.apiName === "name")?.apiName ?? type.properties[0]?.apiName ?? "", properties };
}

function prepareSourceMapping(ontologyId: string, ontology: OntologyCatalog, fileName: string, records: SourceRecord[]) {
  const fields = Array.from(new Set(records.slice(0, 200).flatMap((record) => Object.keys(record))));
  const firstType = ontology.objectTypes[0];
  if (!firstType) throw new Error("Create an Object Type in this ontology first");
  const firstMapping = makeObjectMapping(firstType, fields, records);
  const automaticLinks = ontology.linkTypes
    .filter((link) => link.sourceType === firstType.apiName && matchingLinkField(fields, link))
    .map((link) => ({ id: newId(), sourceObjectMappingId: firstMapping.id, sourceField: matchingLinkField(fields, link)!, linkType: link.apiName, missingTarget: "create" as const }));
  let objectMappings = [firstMapping];
  let linkMappings: LinkMapping[] = automaticLinks;
  let revision = 0;
  const saved = localStorage.getItem(`context-hub.mapping.${ontologyId}`);
  if (saved) {
    try {
      const candidate = JSON.parse(saved) as { revision?: number; backendMappingId?: string; fileName?: string; objectMappings?: ObjectMapping[]; linkMappings?: LinkMapping[] };
      const typesStillExist = candidate.objectMappings?.every((mapping) => ontology.objectTypes.some((type) => type.apiName === mapping.objectType));
      if (candidate.fileName === fileName && candidate.objectMappings?.length && typesStillExist) {
        objectMappings = candidate.objectMappings;
        linkMappings = candidate.linkMappings ?? [];
        revision = candidate.revision ?? 0;
        return { objectMappings, linkMappings, revision, backendMappingId: candidate.backendMappingId ?? "" };
      }
    } catch {
      // Ignore an unreadable local draft and create a new mapping from the detected schema.
    }
  }
  return { objectMappings, linkMappings, revision, backendMappingId: "" };
}

function linkForMapping(link: LinkMapping, objectMappings: ObjectMapping[], ontology: OntologyCatalog) {
  const sourceMapping = objectMappings.find((mapping) => mapping.id === link.sourceObjectMappingId);
  return ontology.linkTypes.find((type) => type.apiName === link.linkType && type.sourceType === sourceMapping?.objectType);
}

export function buildImportedGraph(options: {
  records: SourceRecord[]; objectMappings: ObjectMapping[]; linkMappings: LinkMapping[]; ontology: OntologyCatalog; fileName: string;
}): ImportedGraph {
  const { records, objectMappings, linkMappings, ontology, fileName } = options;
  const nodes = new Map<string, ImportedGraph["nodes"][number]>();
  const links: ImportedGraph["links"] = [];
  let skippedCount = 0;
  let linkErrorCount = 0;

  for (const [mappingIndex, mapping] of objectMappings.entries()) {
    const objectType = ontology.objectTypes.find((type) => type.apiName === mapping.objectType);
    const identityProperty = objectType?.properties.find((property) => property.identity);
    const identityMapping = mapping.properties.find((property) => property.targetProperty === identityProperty?.apiName);
    if (!objectType || !identityProperty || !identityMapping) continue;
    for (const record of records) {
      const identity = record[identityMapping.sourceField];
      if (identity === null || identity === undefined || String(identity).trim() === "") { skippedCount += 1; continue; }
      const properties = Object.fromEntries(mapping.properties.map((property) => [property.targetProperty, applyTransform(record[property.sourceField] ?? null, property.transform)])) as Record<string, GraphValue>;
      const id = `${objectType.apiName}:${String(identity)}`;
      nodes.set(id, { id, name: String(properties[mapping.displayProperty] ?? identity), kind: objectType.displayName, group: objectType.displayName, color: colors[mappingIndex % colors.length], properties });
    }
  }

  for (const mapping of linkMappings) {
    const sourceMapping = objectMappings.find((item) => item.id === mapping.sourceObjectMappingId);
    const linkType = linkForMapping(mapping, objectMappings, ontology);
    const sourceType = ontology.objectTypes.find((type) => type.apiName === sourceMapping?.objectType);
    const targetType = ontology.objectTypes.find((type) => type.apiName === linkType?.targetType);
    const sourceIdentity = sourceType?.properties.find((property) => property.identity);
    const targetIdentity = targetType?.properties.find((property) => property.identity);
    const sourceIdentityMapping = sourceMapping?.properties.find((property) => property.targetProperty === sourceIdentity?.apiName);
    if (!sourceMapping || !linkType || !sourceType || !targetType || !sourceIdentityMapping || !targetIdentity) continue;
    for (const record of records) {
      const sourceValue = record[sourceIdentityMapping.sourceField];
      if (sourceValue === null || sourceValue === undefined) continue;
      const values = Array.isArray(record[mapping.sourceField]) ? record[mapping.sourceField] as GraphValue[] : [record[mapping.sourceField]];
      for (const targetValue of values) {
        if (targetValue === null || targetValue === undefined || typeof targetValue === "object") continue;
        const sourceId = `${sourceType.apiName}:${String(sourceValue)}`;
        const targetId = `${targetType.apiName}:${String(targetValue)}`;
        if (!nodes.has(targetId)) {
          if (mapping.missingTarget === "skip") continue;
          if (mapping.missingTarget === "error") { linkErrorCount += 1; continue; }
          const colorIndex = Math.max(0, ontology.objectTypes.findIndex((type) => type.apiName === targetType.apiName));
          nodes.set(targetId, { id: targetId, name: String(targetValue), kind: targetType.displayName, group: targetType.displayName, color: colors[colorIndex % colors.length], properties: { [targetIdentity.apiName]: targetValue } });
        }
        if (nodes.has(sourceId)) links.push({ source: sourceId, target: targetId, label: linkType.apiName, properties: {} });
      }
    }
  }
  return {
    nodes: [...nodes.values()], links, sourceName: fileName, importedAt: new Date().toISOString(), recordCount: records.length, skippedCount, linkErrorCount,
    ontologyBindings: {
      objectTypes: [...new Set(objectMappings.map((mapping) => mapping.objectType))],
      linkTypes: [...new Set(linkMappings.map((mapping) => mapping.linkType))],
    },
  };
}

export function MappingPanel({ ontologyId, ontologyName, ontologySlug, ontology, dataSource, onDataSourceLoaded, onImport }: { ontologyId: string; ontologyName: string; ontologySlug: string; ontology: OntologyCatalog; dataSource: BrowserDataSource | null; onDataSourceLoaded: (source: BrowserDataSource) => void; onImport: (graph: ImportedGraph) => void }) {
  const prepared = useMemo(() => {
    if (!dataSource) return { objectMappings: [] as ObjectMapping[], linkMappings: [] as LinkMapping[], revision: 0, backendMappingId: "", error: "" };
    try {
      return { ...prepareSourceMapping(ontologyId, ontology, dataSource.fileName, dataSource.records), error: "" };
    } catch (error) {
      return { objectMappings: [] as ObjectMapping[], linkMappings: [] as LinkMapping[], revision: 0, backendMappingId: "", error: error instanceof Error ? error.message : "The source cannot be mapped." };
    }
  }, [dataSource, ontology, ontologyId]);
  const [fileName] = useState(dataSource?.fileName ?? "");
  const [records] = useState<SourceRecord[]>(dataSource?.records ?? []);
  const [objectMappings, setObjectMappings] = useState<ObjectMapping[]>(prepared.objectMappings);
  const [activeMappingId, setActiveMappingId] = useState(prepared.objectMappings[0]?.id ?? "");
  const [linkMappings, setLinkMappings] = useState<LinkMapping[]>(prepared.linkMappings);
  const [previewGraph, setPreviewGraph] = useState<ImportedGraph | null>(null);
  const [revision, setRevision] = useState(prepared.revision);
  const [backendMappingId, setBackendMappingId] = useState(prepared.backendMappingId);
  const [busy, setBusy] = useState(false);
  const [showRestSource, setShowRestSource] = useState(false);
  const [showGraphqlSource, setShowGraphqlSource] = useState(false);
  const [message, setMessage] = useState(prepared.error || (dataSource ? `${dataSource.records.length.toLocaleString("de-DE")} records ready from the shared workspace source.` : "Choose a JSON, NDJSON or CSV file."));
  const sourceFields = useMemo(() => Array.from(new Set(records.flatMap((record) => Object.keys(record)))), [records]);
  const activeMapping = objectMappings.find((mapping) => mapping.id === activeMappingId) ?? objectMappings[0];
  const activeType = ontology.objectTypes.find((type) => type.apiName === activeMapping?.objectType);

  async function loadFile(event: ChangeEvent<HTMLInputElement>) {
    const file = event.target.files?.[0];
    if (!file) return;
    try {
      const parsed = parseSource(file.name, await file.text());
      if (!parsed.length) throw new Error("No object records found");
      const uploaded = await uploadWorkspaceSource(file);
      onDataSourceLoaded({ id: uploaded.id, fileName: file.name, kind: "upload", records: parsed });
      setMessage(`${parsed.length.toLocaleString("de-DE")} records loaded and stored in MinIO.`);
    } catch (error) {
      setMessage(error instanceof Error ? error.message : "The file could not be parsed.");
    }
  }

  function changeObjectType(apiName: string) {
    const type = ontology.objectTypes.find((item) => item.apiName === apiName);
    if (!activeMapping || !type) return;
    const replacement = makeObjectMapping(type, sourceFields, records);
    replacement.id = activeMapping.id;
    setObjectMappings((items) => items.map((item) => item.id === activeMapping.id ? replacement : item));
    setLinkMappings((items) => items.filter((item) => item.sourceObjectMappingId !== activeMapping.id));
  }

  function updateObjectMapping(patch: Partial<ObjectMapping>) {
    if (!activeMapping) return;
    setObjectMappings((items) => items.map((item) => item.id === activeMapping.id ? { ...item, ...patch } : item));
  }

  function addObjectMapping() {
    const type = ontology.objectTypes.find((candidate) => !objectMappings.some((mapping) => mapping.objectType === candidate.apiName)) ?? ontology.objectTypes[0];
    if (!type) return;
    const mapping = makeObjectMapping(type, sourceFields, records);
    setObjectMappings((items) => [...items, mapping]); setActiveMappingId(mapping.id);
  }

  function addPropertyMapping() {
    if (!activeMapping || !activeType) return;
    const target = activeType.properties.find((property) => !property.derived && !activeMapping.properties.some((mapping) => mapping.targetProperty === property.apiName));
    const source = sourceFields.find((field) => !activeMapping.properties.some((mapping) => mapping.sourceField === field));
    if (!target || !source) return;
    updateObjectMapping({ properties: [...activeMapping.properties, { id: newId(), sourceField: source, targetProperty: target.apiName, transform: "None" }] });
  }

  function addLinkMapping() {
    const sourceMapping = activeMapping ?? objectMappings[0];
    const linkType = ontology.linkTypes.find((type) => type.sourceType === sourceMapping?.objectType);
    if (!sourceMapping || !linkType || !sourceFields.length) return;
    const sourceField = sourceFields.find((field) => field === linkType.apiName) ?? sourceFields[0];
    setLinkMappings((items) => [...items, { id: newId(), sourceObjectMappingId: sourceMapping.id, sourceField, linkType: linkType.apiName, missingTarget: "create" }]);
  }

  function createGraph() {
    return buildImportedGraph({ records, objectMappings, linkMappings, ontology, fileName });
  }

  function backendMapping() {
    if (!dataSource || !objectMappings.length) throw new Error("Add at least one object mapping.");
    const backendObjectMappings = objectMappings.map((objectMapping) => {
      const objectType = ontology.objectTypes.find((type) => type.apiName === objectMapping.objectType);
      const identityProperty = objectType?.properties.find((property) => property.identity)?.apiName;
      if (!objectType || !identityProperty) throw new Error(`Object mapping '${objectMapping.objectType}' needs an ontology identity property.`);
      if (!objectMapping.properties.some((property) => property.targetProperty === identityProperty)) {
        throw new Error(`Object mapping '${objectType.displayName}' must map its identity property '${identityProperty}'.`);
      }
      return { objectType: objectMapping.objectType, identityProperty, properties: objectMapping.properties };
    });
    const links = linkMappings.map((link) => {
      const sourceMapping = objectMappings.find((mapping) => mapping.id === link.sourceObjectMappingId);
      const linkType = linkForMapping(link, objectMappings, ontology);
      const targetType = ontology.objectTypes.find((type) => type.apiName === linkType?.targetType);
      const targetIdentityProperty = targetType?.properties.find((property) => property.identity)?.apiName;
      if (!sourceMapping || !linkType || !targetType || !targetIdentityProperty) throw new Error(`Link mapping '${link.linkType}' has no valid source or target identity.`);
      return { sourceObjectType: sourceMapping.objectType, sourceField: link.sourceField, linkType: link.linkType, targetObjectType: targetType.apiName, targetIdentityProperty, missingTarget: link.missingTarget };
    });
    return {
      id: backendMappingId || undefined,
      ontologyId,
      dataSourceId: dataSource.id,
      name: `${fileName} → ${backendObjectMappings.length} object type${backendObjectMappings.length === 1 ? "" : "s"}`,
      objectMappings: backendObjectMappings,
      links,
    };
  }

  async function persistMapping() {
    const saved = await saveOntologyMapping(backendMapping());
    const nextRevision = revision + 1;
    localStorage.setItem(`context-hub.mapping.${ontologyId}`, JSON.stringify({ revision: nextRevision, backendMappingId: saved.id, fileName, objectMappings, linkMappings }));
    setBackendMappingId(saved.id); setRevision(nextRevision);
    return saved;
  }

  async function saveMapping() {
    setBusy(true);
    try {
      await persistMapping();
      setMessage(`Mapping revision ${revision + 1} saved in ClickHouse.`);
    } catch (error) {
      setMessage(error instanceof Error ? error.message : "The mapping could not be saved.");
    } finally {
      setBusy(false);
    }
  }

  async function importRecords() {
    setBusy(true);
    try {
      setMessage("Publishing ontology and saving mapping…");
      const version = await publishOntologyCatalog({ id: ontologyId, name: ontologyName, slug: ontologySlug }, ontology);
      const mapping = await persistMapping();
      setMessage("DataFusion ingestion is running…");
      const job = await startIngestion(dataSource!.id, mapping.id, version.id);
      if (job.state !== IngestionState.SUCCEEDED) throw new Error(job.error || "The ingestion job did not complete successfully.");
      const graph = createGraph(); onImport(graph);
      setMessage(`${job.nodesWritten.toLocaleString("de-DE")} objects and ${job.edgesWritten.toLocaleString("de-DE")} links persisted in ClickHouse.`);
    } catch (error) {
      setMessage(error instanceof Error ? error.message : "The import could not be completed.");
    } finally {
      setBusy(false);
    }
  }

  return <div className="workspace-view mapping-view">
    <header className="stage-header"><div><span className="eyebrow">Ontology mapping</span><h1>Data import</h1><p>Bind file, REST, or GraphQL records to the current ontology draft.</p></div><div className="header-actions"><span className="save-state">Revision {revision}</span><button className="button secondary" onClick={() => setShowGraphqlSource(true)}><Braces size={15}/> GraphQL</button><button className="button secondary" onClick={() => setShowRestSource(true)}><Globe2 size={15}/> REST</button><label className="button secondary file-button"><Upload size={15}/> File<input type="file" accept=".json,.jsonl,.ndjson,.csv,application/json,text/csv" onChange={loadFile}/></label><button className="button secondary" disabled={!records.length || busy} onClick={() => setPreviewGraph(createGraph())}><Play size={15}/> Preview</button><button className="button secondary" disabled={!records.length || busy} onClick={saveMapping}><Save size={15}/> Save</button><button className="button primary" disabled={!records.length || !objectMappings.length || busy} onClick={importRecords}><CheckCircle2 size={15}/> {busy ? "Working…" : "Import"}</button></div></header>
    <div className="import-status" role="status">{message}</div>
    <div className="mapping-grid">
      <section className="source-card"><div className="card-title">{dataSource?.kind === "graphql" ? <Braces size={18}/> : dataSource?.kind === "rest" ? <Globe2 size={18}/> : <FileJson2 size={18}/>}<div><strong>{fileName || "No source selected"}</strong><span>{records.length ? `${records.length.toLocaleString("de-DE")} preview records` : "JSON · NDJSON · CSV · REST · GraphQL"}</span></div></div><span className="eyebrow">Detected fields</span>{sourceFields.map((field) => <div className="schema-field" key={field}><code>{field}</code><span>{detectType(records, field)}</span></div>)}</section>
      <section className="mapping-card">
        <div className="section-title"><div><span className="eyebrow">Object mappings</span><h2>Source record → ontology object</h2></div><button onClick={addObjectMapping} disabled={!records.length || !ontology.objectTypes.length}><Plus size={14}/> Object mapping</button></div>
        <div className="mapping-tabs">{objectMappings.map((mapping) => { const type = ontology.objectTypes.find((item) => item.apiName === mapping.objectType); return <button className={mapping.id === activeMapping?.id ? "active" : ""} key={mapping.id} onClick={() => setActiveMappingId(mapping.id)}>{type?.displayName ?? mapping.objectType}</button>; })}</div>
        {activeMapping && activeType ? <><div className="mapping-settings"><label>Ontology Object Type<select value={activeMapping.objectType} onChange={(event) => changeObjectType(event.target.value)}>{ontology.objectTypes.map((type) => <option value={type.apiName} key={type.apiName}>{type.displayName} ({type.apiName})</option>)}</select></label><label>Display property<select value={activeMapping.displayProperty} onChange={(event) => updateObjectMapping({ displayProperty: event.target.value })}>{activeType.properties.map((property) => <option value={property.apiName} key={property.apiName}>{property.displayName}</option>)}</select></label><label>Identity property<input value={activeType.properties.find((property) => property.identity)?.apiName ?? "Missing in ontology"} readOnly/></label></div>
          <div className="mapping-column-head"><span>Source field</span><span/><span>Ontology property</span><span>Transform</span><span/></div>
          {activeMapping.properties.map((mapping) => <div className="mapping-row ontology-bound" key={mapping.id}><select value={mapping.sourceField} onChange={(event) => updateObjectMapping({ properties: activeMapping.properties.map((item) => item.id === mapping.id ? { ...item, sourceField: event.target.value } : item) })}>{sourceFields.map((field) => <option key={field}>{field}</option>)}</select><ArrowRight size={15}/><select value={mapping.targetProperty} onChange={(event) => updateObjectMapping({ properties: activeMapping.properties.map((item) => item.id === mapping.id ? { ...item, targetProperty: event.target.value } : item) })}>{activeType.properties.filter((property) => !property.derived).map((property) => <option value={property.apiName} key={property.apiName}>{activeType.apiName}.{property.apiName}</option>)}</select><select value={mapping.transform} onChange={(event) => updateObjectMapping({ properties: activeMapping.properties.map((item) => item.id === mapping.id ? { ...item, transform: event.target.value as Transform } : item) })}><option>None</option><option>Trim</option><option>Lowercase</option><option>Uppercase</option></select><button title="Delete property mapping" onClick={() => updateObjectMapping({ properties: activeMapping.properties.filter((item) => item.id !== mapping.id) })}><Trash2 size={13}/></button></div>)}
          <div className="mapping-actions"><button onClick={addPropertyMapping}><Plus size={13}/> Property mapping</button>{objectMappings.length > 1 && <button className="danger-text" onClick={() => { setObjectMappings((items) => items.filter((item) => item.id !== activeMapping.id)); setLinkMappings((items) => items.filter((item) => item.sourceObjectMappingId !== activeMapping.id)); setActiveMappingId(objectMappings.find((item) => item.id !== activeMapping.id)?.id ?? ""); }}><Trash2 size={13}/> Delete object mapping</button>}</div></> : <div className="mapping-empty">Choose a file to create the first ontology mapping.</div>}

        <div className="link-mapping-section"><div className="section-title"><div><span className="eyebrow">Link mappings</span><h2>Source reference → ontology link</h2></div><button onClick={addLinkMapping} disabled={!objectMappings.length}><Plus size={14}/> Link mapping</button></div>
          {linkMappings.map((mapping) => {
            const sourceMapping = objectMappings.find((item) => item.id === mapping.sourceObjectMappingId);
            const availableLinks = ontology.linkTypes.filter((type) => type.sourceType === sourceMapping?.objectType);
            const linkType = linkForMapping(mapping, objectMappings, ontology);
            const targetType = ontology.objectTypes.find((type) => type.apiName === linkType?.targetType);
            return <div className="link-mapping-card" key={mapping.id}><div className="link-mapping-title"><GitBranch size={14}/><strong>{linkType?.displayName ?? "Select link type"}</strong><button title="Delete link mapping" onClick={() => setLinkMappings((items) => items.filter((item) => item.id !== mapping.id))}><Trash2 size={13}/></button></div><div className="mapping-settings four"><label>Source object<select value={mapping.sourceObjectMappingId} onChange={(event) => { const nextSource = objectMappings.find((item) => item.id === event.target.value); const nextLink = ontology.linkTypes.find((type) => type.sourceType === nextSource?.objectType); setLinkMappings((items) => items.map((item) => item.id === mapping.id ? { ...item, sourceObjectMappingId: event.target.value, linkType: nextLink?.apiName ?? "" } : item)); }}>{objectMappings.map((item) => <option value={item.id} key={item.id}>{ontology.objectTypes.find((type) => type.apiName === item.objectType)?.displayName}</option>)}</select></label><label>Reference field<select value={mapping.sourceField} onChange={(event) => setLinkMappings((items) => items.map((item) => item.id === mapping.id ? { ...item, sourceField: event.target.value } : item))}>{sourceFields.map((field) => <option key={field}>{field}</option>)}</select></label><label>Ontology Link Type<select value={mapping.linkType} onChange={(event) => setLinkMappings((items) => items.map((item) => item.id === mapping.id ? { ...item, linkType: event.target.value } : item))}>{availableLinks.map((type) => <option value={type.apiName} key={type.apiName}>{type.displayName} ({type.sourceType} → {type.targetType})</option>)}</select></label><label>Missing target<select value={mapping.missingTarget} onChange={(event) => setLinkMappings((items) => items.map((item) => item.id === mapping.id ? { ...item, missingTarget: event.target.value as LinkMapping["missingTarget"] } : item))}><option value="create">Create target</option><option value="skip">Skip link</option><option value="error">Report error</option></select></label></div><div className="ontology-binding"><span>Target Object Type</span><strong>{targetType?.displayName ?? "—"}</strong><span>Target identity</span><code>{targetType?.properties.find((property) => property.identity)?.apiName ?? "missing"}</code></div></div>;
          })}{!linkMappings.length && <div className="mapping-empty">No relationship is mapped. Add a Link Mapping to create graph edges.</div>}
        </div>
      </section>
    </div>
    {previewGraph && <section className="preview-table"><div className="section-title"><div><span className="eyebrow">Ontology preview</span><h2>{previewGraph.nodes.length.toLocaleString("de-DE")} objects · {previewGraph.links.length.toLocaleString("de-DE")} links</h2></div><span className="success-pill"><CheckCircle2 size={13}/> Based on {previewGraph.recordCount.toLocaleString("de-DE")} records</span></div><div className="preview-summary"><div><strong>{previewGraph.nodes.length.toLocaleString("de-DE")}</strong><span>Objects</span></div><div><strong>{previewGraph.links.length.toLocaleString("de-DE")}</strong><span>Links</span></div><div><strong>{previewGraph.linkErrorCount.toLocaleString("de-DE")}</strong><span>Link errors</span></div></div></section>}
    {showRestSource && <RestSourceForm onClose={() => setShowRestSource(false)} onCreated={onDataSourceLoaded}/>}
    {showGraphqlSource && <GraphqlSourceForm onClose={() => setShowGraphqlSource(false)} onCreated={onDataSourceLoaded}/>}
  </div>;
}
