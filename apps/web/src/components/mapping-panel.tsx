"use client";

import { ChangeEvent, useMemo, useState } from "react";
import { ArrowDown, ArrowRight, ArrowUp, Braces, CheckCircle2, FileJson2, GitBranch, Globe2, Play, Plus, Save, Trash2, Upload, WandSparkles } from "lucide-react";
import type { GraphValue, ImportedGraph } from "@/lib/graph-data";
import type { OntologyCatalog, OntologyObjectType } from "@/lib/ontology-catalog";
import { IngestionState } from "@/gen/context_hub/v1/context_hub_pb";
import { previewWorkspaceSource, publishOntologyCatalog, saveOntologyMapping, startIngestion, uploadWorkspaceSource } from "@/lib/context-hub-client";
import { RestSourceForm } from "@/components/rest-source-form";
import { GraphqlSourceForm } from "@/components/graphql-source-form";
import { applyTransforms, createTransform, migrateLegacyTransform, transformLabel, transformOptions, type MappingTransform, type TransformKind } from "@/lib/mapping-transforms";

export type SourceRecord = Record<string, GraphValue>;
export type BrowserDataSource = { id: string; fileName: string; kind: "upload" | "rest" | "graphql"; records: SourceRecord[] };
export type FieldErrorStrategy = "reject_row" | "use_null" | "abort_job";
export type PropertyMapping = { id: string; sourceField: string; targetProperty: string; transforms: MappingTransform[]; onError: FieldErrorStrategy };
export type ObjectMapping = { id: string; objectType: string; displayProperty: string; properties: PropertyMapping[] };
export type LinkMapping = { id: string; sourceObjectMappingId: string; sourceField: string; linkType: string; missingTarget: "create" | "skip" | "error" };

class PreviewAbortError extends Error {}

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
    return { id: newId(), sourceField, targetProperty: property.apiName, transforms: sourceField && typeof records[0]?.[sourceField] === "string" ? [{ kind: "trim" as const }] : [], onError: "reject_row" as const };
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
        objectMappings = candidate.objectMappings.map((mapping) => ({
          ...mapping,
          properties: mapping.properties.map((property) => {
            const legacy = property as PropertyMapping & { transform?: unknown };
            return { ...property, transforms: migrateLegacyTransform(property.transforms ?? legacy.transform), onError: property.onError ?? "reject_row" };
          }),
        }));
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
      let properties: Record<string, GraphValue>;
      try {
        properties = {};
        for (const property of mapping.properties) {
          try {
            properties[property.targetProperty] = applyTransforms(record[property.sourceField] ?? null, property.transforms, record);
          } catch (error) {
            if (property.onError === "use_null") properties[property.targetProperty] = null;
            else if (property.onError === "abort_job") throw new PreviewAbortError(`Field '${property.targetProperty}' failed during preview`, { cause: error });
            else throw error;
          }
        }
      } catch (error) {
        if (error instanceof PreviewAbortError) throw error;
        skippedCount += 1;
        continue;
      }
      const identity = properties[identityProperty.apiName];
      if (identity === null || identity === undefined || String(identity).trim() === "") { skippedCount += 1; continue; }
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
      let sourceValue: GraphValue;
      try {
        sourceValue = applyTransforms(record[sourceIdentityMapping.sourceField] ?? null, sourceIdentityMapping.transforms, record);
      } catch {
        continue;
      }
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

function fieldsFromText(value: string) {
  return value.split(",").map((field) => field.trim()).filter(Boolean);
}

function TransformParameters({ transform, onChange }: { transform: MappingTransform; onChange: (value: MappingTransform) => void }) {
  if (transform.kind === "cast") return <label>Target type<select aria-label="Cast target" value={transform.target} onChange={(event) => onChange({ ...transform, target: event.target.value as typeof transform.target })}><option value="string">String</option><option value="boolean">Boolean</option><option value="int64">Int64</option><option value="float64">Float64</option><option value="decimal">Decimal</option><option value="date">Date</option><option value="timestamp">Timestamp</option></select></label>;
  if (transform.kind === "replace") return <><label>Find<input aria-label="Find text" value={transform.from} onChange={(event) => onChange({ ...transform, from: event.target.value })}/></label><label>Replace with<input aria-label="Replacement text" value={transform.to} onChange={(event) => onChange({ ...transform, to: event.target.value })}/></label></>;
  if (transform.kind === "regex_replace") return <><label>Pattern<input aria-label="Regex pattern" value={transform.pattern} onChange={(event) => onChange({ ...transform, pattern: event.target.value })}/></label><label>Replacement<input aria-label="Regex replacement" value={transform.replacement} onChange={(event) => onChange({ ...transform, replacement: event.target.value })}/></label></>;
  if (transform.kind === "default") return <label>JSON or text value<input aria-label="Default value" value={transform.value} onChange={(event) => onChange({ ...transform, value: event.target.value })}/></label>;
  if (transform.kind === "coalesce") return <label>Fallback fields<input aria-label="Fallback fields" list="mapping-source-fields" placeholder="field_a, field_b" value={transform.fields.join(", ")} onChange={(event) => onChange({ ...transform, fields: fieldsFromText(event.target.value) })}/></label>;
  if (transform.kind === "concat") return <><label>Additional fields<input aria-label="Concatenation fields" list="mapping-source-fields" placeholder="field_a, field_b" value={transform.fields.join(", ")} onChange={(event) => onChange({ ...transform, fields: fieldsFromText(event.target.value) })}/></label><label>Separator<input aria-label="Concatenation separator" value={transform.separator} onChange={(event) => onChange({ ...transform, separator: event.target.value })}/></label></>;
  if (transform.kind === "add" || transform.kind === "multiply") return <label>Number<input aria-label={`${transformLabel(transform)} value`} type="number" step="any" value={transform.value} onChange={(event) => onChange({ ...transform, value: Number(event.target.value) })}/></label>;
  if (transform.kind === "parse_date" || transform.kind === "parse_timestamp") return <label>Format<input aria-label={`${transformLabel(transform)} format`} placeholder="%Y-%m-%d" value={transform.format} onChange={(event) => onChange({ ...transform, format: event.target.value })}/></label>;
  return <span className="transform-no-parameters">No parameters</span>;
}

function TransformEditor({ transforms, onChange, onError, onErrorChange }: { transforms: MappingTransform[]; onChange: (value: MappingTransform[]) => void; onError: FieldErrorStrategy; onErrorChange: (value: FieldErrorStrategy) => void }) {
  function replace(index: number, transform: MappingTransform) {
    onChange(transforms.map((item, itemIndex) => itemIndex === index ? transform : item));
  }
  function move(index: number, offset: number) {
    const next = [...transforms];
    const target = index + offset;
    if (target < 0 || target >= next.length) return;
    [next[index], next[target]] = [next[target], next[index]];
    onChange(next);
  }
  return <div className="transform-editor">
    <div className="transform-editor-title"><span><WandSparkles size={12}/> Transformation pipeline</span><select aria-label="Field error strategy" value={onError} onChange={(event) => onErrorChange(event.target.value as FieldErrorStrategy)}><option value="reject_row">Skip row</option><option value="use_null">Use null</option><option value="abort_job">Abort import</option></select><select aria-label="Add transformation" value="" onChange={(event) => { if (event.target.value) onChange([...transforms, createTransform(event.target.value as TransformKind)]); }}><option value="">+ Add transform</option>{transformOptions.map((option) => <option value={option.kind} key={option.kind}>{option.label}</option>)}</select></div>
    {!transforms.length && <span className="transform-empty">Source value is used unchanged.</span>}
    {transforms.map((transform, index) => <div className="transform-step" key={`${index}-${transform.kind}`}>
      <span className="transform-order">{index + 1}</span>
      <select aria-label={`Transformation ${index + 1}`} value={transform.kind} onChange={(event) => replace(index, createTransform(event.target.value as TransformKind))}>{transformOptions.map((option) => <option value={option.kind} key={option.kind}>{option.label}</option>)}</select>
      <div className="transform-parameters"><TransformParameters transform={transform} onChange={(value) => replace(index, value)}/></div>
      <div className="transform-step-actions"><button aria-label="Move transformation up" disabled={index === 0} onClick={() => move(index, -1)}><ArrowUp size={11}/></button><button aria-label="Move transformation down" disabled={index === transforms.length - 1} onClick={() => move(index, 1)}><ArrowDown size={11}/></button><button aria-label="Delete transformation" onClick={() => onChange(transforms.filter((_, itemIndex) => itemIndex !== index))}><Trash2 size={11}/></button></div>
    </div>)}
  </div>;
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
  const [message, setMessage] = useState(prepared.error || (dataSource ? `${dataSource.records.length.toLocaleString("de-DE")} records ready from the shared workspace source.` : "Choose a JSON, NDJSON, CSV or Parquet file."));
  const sourceFields = useMemo(() => Array.from(new Set(records.flatMap((record) => Object.keys(record)))), [records]);
  const activeMapping = objectMappings.find((mapping) => mapping.id === activeMappingId) ?? objectMappings[0];
  const activeType = ontology.objectTypes.find((type) => type.apiName === activeMapping?.objectType);

  async function loadFile(event: ChangeEvent<HTMLInputElement>) {
    const file = event.target.files?.[0];
    if (!file) return;
    try {
      const isParquet = /\.parquet$/i.test(file.name);
      const uploaded = await uploadWorkspaceSource(file, (uploadedBytes, totalBytes) => {
        setMessage(`Uploading ${file.name}: ${Math.round((uploadedBytes / totalBytes) * 100)}%`);
      });
      const preview = await previewWorkspaceSource(uploaded.id);
      const parsed = parseSource(file.name, new TextDecoder().decode(preview.content));
      if (!parsed.length) throw new Error("No object records found");
      onDataSourceLoaded({ id: uploaded.id, fileName: file.name, kind: "upload", records: parsed });
      setMessage(`${parsed.length.toLocaleString("de-DE")} preview records loaded; the original ${isParquet ? "Parquet file" : "file"} is stored in MinIO.`);
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
    updateObjectMapping({ properties: [...activeMapping.properties, { id: newId(), sourceField: source, targetProperty: target.apiName, transforms: [], onError: "reject_row" }] });
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

  function previewRecords() {
    try {
      const graph = createGraph();
      setPreviewGraph(graph);
      setMessage(`${graph.nodes.length.toLocaleString("de-DE")} objects previewed; ${graph.skippedCount.toLocaleString("de-DE")} source rows skipped.`);
    } catch (error) {
      setPreviewGraph(null);
      setMessage(error instanceof Error ? error.message : "The preview failed.");
    }
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
      setMessage(`${job.nodesWritten.toLocaleString("de-DE")} objects and ${job.edgesWritten.toLocaleString("de-DE")} links persisted in ClickHouse · ${job.rowsRejected.toLocaleString("de-DE")} rows rejected from ${job.rowsRead.toLocaleString("de-DE")} read.`);
    } catch (error) {
      setMessage(error instanceof Error ? error.message : "The import could not be completed.");
    } finally {
      setBusy(false);
    }
  }

  return <div className="workspace-view mapping-view">
    <header className="stage-header"><div><span className="eyebrow">Ontology mapping</span><h1>Data import</h1><p>Bind file, REST, or GraphQL records to the current ontology draft.</p></div><div className="header-actions"><span className="save-state">Revision {revision}</span><button className="button secondary" onClick={() => setShowGraphqlSource(true)}><Braces size={15}/> GraphQL</button><button className="button secondary" onClick={() => setShowRestSource(true)}><Globe2 size={15}/> REST</button><label className="button secondary file-button"><Upload size={15}/> File<input type="file" accept=".json,.jsonl,.ndjson,.csv,.parquet,application/json,text/csv,application/vnd.apache.parquet" onChange={loadFile}/></label><button className="button secondary" disabled={!records.length || busy} onClick={previewRecords}><Play size={15}/> Preview</button><button className="button secondary" disabled={!records.length || busy} onClick={saveMapping}><Save size={15}/> Save</button><button className="button primary" disabled={!records.length || !objectMappings.length || busy} onClick={importRecords}><CheckCircle2 size={15}/> {busy ? "Working…" : "Import"}</button></div></header>
    <div className="import-status" role="status">{message}</div>
    <div className="mapping-grid">
      <section className="source-card"><div className="card-title">{dataSource?.kind === "graphql" ? <Braces size={18}/> : dataSource?.kind === "rest" ? <Globe2 size={18}/> : <FileJson2 size={18}/>}<div><strong>{fileName || "No source selected"}</strong><span>{records.length ? `${records.length.toLocaleString("de-DE")} preview records` : "JSON · NDJSON · CSV · Parquet · REST · GraphQL"}</span></div></div><span className="eyebrow">Detected fields</span>{sourceFields.map((field) => <div className="schema-field" key={field}><code>{field}</code><span>{detectType(records, field)}</span></div>)}</section>
      <section className="mapping-card">
        <div className="section-title"><div><span className="eyebrow">Object mappings</span><h2>Source record → ontology object</h2></div><button onClick={addObjectMapping} disabled={!records.length || !ontology.objectTypes.length}><Plus size={14}/> Object mapping</button></div>
        <div className="mapping-tabs">{objectMappings.map((mapping) => { const type = ontology.objectTypes.find((item) => item.apiName === mapping.objectType); return <button className={mapping.id === activeMapping?.id ? "active" : ""} key={mapping.id} onClick={() => setActiveMappingId(mapping.id)}>{type?.displayName ?? mapping.objectType}</button>; })}</div>
        {activeMapping && activeType ? <><div className="mapping-settings"><label>Ontology Object Type<select value={activeMapping.objectType} onChange={(event) => changeObjectType(event.target.value)}>{ontology.objectTypes.map((type) => <option value={type.apiName} key={type.apiName}>{type.displayName} ({type.apiName})</option>)}</select></label><label>Display property<select value={activeMapping.displayProperty} onChange={(event) => updateObjectMapping({ displayProperty: event.target.value })}>{activeType.properties.map((property) => <option value={property.apiName} key={property.apiName}>{property.displayName}</option>)}</select></label><label>Identity property<input value={activeType.properties.find((property) => property.identity)?.apiName ?? "Missing in ontology"} readOnly/></label></div>
          <div className="mapping-column-head"><span>Source field</span><span/><span>Ontology property</span><span>Pipeline</span><span/></div>
          {activeMapping.properties.map((mapping) => <div className="mapping-row ontology-bound" key={mapping.id}><select value={mapping.sourceField} onChange={(event) => updateObjectMapping({ properties: activeMapping.properties.map((item) => item.id === mapping.id ? { ...item, sourceField: event.target.value } : item) })}>{sourceFields.map((field) => <option key={field}>{field}</option>)}</select><ArrowRight size={15}/><select value={mapping.targetProperty} onChange={(event) => updateObjectMapping({ properties: activeMapping.properties.map((item) => item.id === mapping.id ? { ...item, targetProperty: event.target.value } : item) })}>{activeType.properties.filter((property) => !property.derived).map((property) => <option value={property.apiName} key={property.apiName}>{activeType.apiName}.{property.apiName}</option>)}</select><span className="transform-count">{mapping.transforms.length ? `${mapping.transforms.length} step${mapping.transforms.length === 1 ? "" : "s"}` : "Unchanged"}</span><button title="Delete property mapping" onClick={() => updateObjectMapping({ properties: activeMapping.properties.filter((item) => item.id !== mapping.id) })}><Trash2 size={13}/></button><TransformEditor transforms={mapping.transforms} onChange={(transforms) => updateObjectMapping({ properties: activeMapping.properties.map((item) => item.id === mapping.id ? { ...item, transforms } : item) })} onError={mapping.onError} onErrorChange={(onError) => updateObjectMapping({ properties: activeMapping.properties.map((item) => item.id === mapping.id ? { ...item, onError } : item) })}/></div>)}
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
    <datalist id="mapping-source-fields">{sourceFields.map((field) => <option value={field} key={field}/>)}</datalist>
    {showRestSource && <RestSourceForm onClose={() => setShowRestSource(false)} onCreated={onDataSourceLoaded}/>}
    {showGraphqlSource && <GraphqlSourceForm onClose={() => setShowGraphqlSource(false)} onCreated={onDataSourceLoaded}/>}
  </div>;
}
