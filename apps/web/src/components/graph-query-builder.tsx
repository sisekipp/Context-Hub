"use client";

import { useMemo, useState } from "react";
import { BarChart3, Filter, ListFilter, Plus, Route, X } from "lucide-react";
import type { GraphQuerySpec } from "@/lib/context-hub-client";
import type { OntologyCatalog, OntologyProperty } from "@/lib/ontology-catalog";

type FilterRow = GraphQuerySpec["filters"][number] & { id: string };
type AggregationRow = GraphQuerySpec["aggregations"][number] & { id: string };

const operatorLabels: Array<{ value: FilterRow["operator"]; label: string }> = [
  { value: "equal", label: "Equals" },
  { value: "not_equal", label: "Does not equal" },
  { value: "contains", label: "Contains" },
  { value: "greater_than", label: "Greater than" },
  { value: "less_than", label: "Less than" },
];

const aggregationLabels: Array<{ value: AggregationRow["function"]; label: string }> = [
  { value: "count", label: "Count" },
  { value: "distinct_count", label: "Distinct count" },
  { value: "sum", label: "Sum" },
  { value: "average", label: "Average" },
  { value: "minimum", label: "Minimum" },
  { value: "maximum", label: "Maximum" },
];

function id() {
  return crypto.randomUUID();
}

function isNumeric(property: OntologyProperty | undefined) {
  return !!property && ["Int64", "Float64", "Decimal"].includes(property.type);
}

export function validateGraphQuerySpec(spec: GraphQuerySpec, catalog: OntologyCatalog) {
  const root = catalog.objectTypes.find((type) => type.apiName === spec.rootType);
  if (!root) return "Choose a valid root object type.";
  if (spec.filters.some((filter) => !filter.property || !filter.value.trim())) return "Every filter needs a property and value.";
  if (spec.sort && spec.traversal.length) return "Sorting and traversal cannot be combined in V1.";
  if (spec.aggregations.some((aggregation) => !/^[_a-z][_a-z0-9]*$/.test(aggregation.alias))) return "Aggregation aliases must be lowercase API names.";
  if (spec.aggregations.some((aggregation) => ["sum", "average", "minimum", "maximum"].includes(aggregation.function) && !isNumeric(root.properties.find((property) => property.apiName === aggregation.property)))) return "Numeric aggregations require a numeric property.";
  if (spec.limit < 1 || spec.limit > 5_000) return "The result limit must be between 1 and 5,000.";
  return "";
}

export function GraphQueryBuilder({ catalog, onClose, onRun, busy }: { catalog: OntologyCatalog; onClose: () => void; onRun: (spec: GraphQuerySpec) => Promise<void>; busy: boolean }) {
  const firstType = catalog.objectTypes[0]?.apiName ?? "";
  const [rootType, setRootType] = useState(firstType);
  const [filters, setFilters] = useState<FilterRow[]>([]);
  const [traversal, setTraversal] = useState<GraphQuerySpec["traversal"]>([]);
  const [projection, setProjection] = useState<string[]>([]);
  const [sortProperty, setSortProperty] = useState("");
  const [sortDirection, setSortDirection] = useState<"ascending" | "descending">("ascending");
  const [aggregations, setAggregations] = useState<AggregationRow[]>([]);
  const [limit, setLimit] = useState(500);
  const [message, setMessage] = useState("");
  const root = catalog.objectTypes.find((type) => type.apiName === rootType);
  const currentType = traversal.at(-1)?.targetType ?? rootType;
  const traversalOptions = useMemo(() => catalog.linkTypes.flatMap((link) => {
    const options: Array<{ key: string; label: string; linkType: string; targetType: string; reverse: boolean }> = [];
    if (link.sourceType === currentType) options.push({ key: `${link.apiName}:out`, label: `${link.displayName} → ${link.targetType}`, linkType: link.apiName, targetType: link.targetType, reverse: false });
    if (link.targetType === currentType) options.push({ key: `${link.apiName}:in`, label: `${link.displayName} ← ${link.sourceType}`, linkType: link.apiName, targetType: link.sourceType, reverse: true });
    return options;
  }), [catalog.linkTypes, currentType]);

  function changeRoot(value: string) {
    setRootType(value); setFilters([]); setTraversal([]); setProjection([]); setSortProperty(""); setAggregations([]); setMessage("");
  }

  function addFilter() {
    const property = root?.properties[0]?.apiName ?? "";
    setFilters((items) => [...items, { id: id(), property, operator: "equal", value: "" }]);
  }

  function addTraversal(key: string) {
    const option = traversalOptions.find((entry) => entry.key === key);
    if (option && traversal.length < 6) setTraversal((items) => [...items, { linkType: option.linkType, targetType: option.targetType, reverse: option.reverse }]);
  }

  function addAggregation() {
    setAggregations((items) => [...items, { id: id(), property: "", function: "count", alias: `count_${items.length + 1}` }]);
  }

  async function run() {
    const spec: GraphQuerySpec = {
      rootType,
      filters: filters.map(({ property, operator, value }) => ({ property, operator, value })),
      traversal,
      projection,
      sort: sortProperty ? { property: sortProperty, direction: sortDirection } : undefined,
      aggregations: aggregations.map(({ property, function: aggregationFunction, alias }) => ({ property, function: aggregationFunction, alias })),
      limit,
    };
    const error = validateGraphQuerySpec(spec, catalog);
    if (error) { setMessage(error); return; }
    setMessage("Running ontology-validated graph query…");
    try { await onRun(spec); onClose(); } catch (error) { setMessage(error instanceof Error ? error.message : "The graph query failed."); }
  }

  return <div className="query-builder-overlay" role="presentation"><aside className="query-builder" aria-label="Graph query builder">
    <header><div><span className="eyebrow">Graph DSL</span><h2><ListFilter size={18}/> Query builder</h2><p>Build a bounded, ontology-validated query without raw SQL.</p></div><button aria-label="Close query builder" onClick={onClose}><X size={16}/></button></header>
    <div className="query-builder-body">
      <section><h3>Root and result</h3><div className="query-fields"><label>Root object type<select value={rootType} onChange={(event) => changeRoot(event.target.value)}>{catalog.objectTypes.map((type) => <option key={type.apiName} value={type.apiName}>{type.displayName}</option>)}</select></label><label>Limit<input type="number" min={1} max={5000} value={limit} onChange={(event) => setLimit(Number(event.target.value))}/></label></div></section>
      <section><div className="query-section-title"><h3><Filter size={14}/> Filters</h3><button onClick={addFilter}><Plus size={12}/> Add filter</button></div>{filters.map((filter) => <div className="query-row filter-row" key={filter.id}><select aria-label="Filter property" value={filter.property} onChange={(event) => setFilters((items) => items.map((item) => item.id === filter.id ? { ...item, property: event.target.value } : item))}>{root?.properties.map((property) => <option value={property.apiName} key={property.apiName}>{property.displayName}</option>)}</select><select aria-label="Filter operator" value={filter.operator} onChange={(event) => setFilters((items) => items.map((item) => item.id === filter.id ? { ...item, operator: event.target.value as FilterRow["operator"] } : item))}>{operatorLabels.map((operator) => <option value={operator.value} key={operator.value}>{operator.label}</option>)}</select><input aria-label="Filter value" value={filter.value} onChange={(event) => setFilters((items) => items.map((item) => item.id === filter.id ? { ...item, value: event.target.value } : item))} placeholder="Value"/><button aria-label="Remove filter" onClick={() => setFilters((items) => items.filter((item) => item.id !== filter.id))}><X size={13}/></button></div>)}{!filters.length && <p className="query-empty">No filters. All root objects are eligible.</p>}</section>
      <section><div className="query-section-title"><h3><Route size={14}/> Traversal</h3>{traversalOptions.length > 0 && traversal.length < 6 && <select aria-label="Add traversal" value="" onChange={(event) => addTraversal(event.target.value)}><option value="">Add step…</option>{traversalOptions.map((option) => <option value={option.key} key={option.key}>{option.label}</option>)}</select>}</div>{traversal.map((step, index) => <div className="traversal-chip" key={`${step.linkType}-${index}`}><span>{index + 1}</span><strong>{step.reverse ? "←" : "→"} {step.linkType}</strong><small>{step.targetType}</small><button aria-label={`Remove traversal ${index + 1}`} onClick={() => setTraversal((items) => items.slice(0, index))}><X size={12}/></button></div>)}{!traversal.length && <p className="query-empty">No traversal. Query only the root type.</p>}</section>
      <section><h3>Projection</h3><div className="projection-grid">{root?.properties.map((property) => <label key={property.apiName}><input type="checkbox" checked={projection.includes(property.apiName)} onChange={(event) => setProjection((items) => event.target.checked ? [...items, property.apiName] : items.filter((item) => item !== property.apiName))}/><span>{property.displayName}</span></label>)}</div><p className="query-hint">No selection returns all properties.</p></section>
      <section><h3>Sort</h3><div className="query-fields"><label>Property<select aria-label="Sort property" disabled={traversal.length > 0} value={sortProperty} onChange={(event) => setSortProperty(event.target.value)}><option value="">Object ID (default)</option>{root?.properties.map((property) => <option value={property.apiName} key={property.apiName}>{property.displayName}</option>)}</select></label><label>Direction<select aria-label="Sort direction" disabled={!sortProperty} value={sortDirection} onChange={(event) => setSortDirection(event.target.value as typeof sortDirection)}><option value="ascending">Ascending</option><option value="descending">Descending</option></select></label></div>{traversal.length > 0 && <p className="query-hint">Custom property sorting is disabled for traversals in V1.</p>}</section>
      <section><div className="query-section-title"><h3><BarChart3 size={14}/> Aggregations</h3><button onClick={addAggregation}><Plus size={12}/> Add aggregation</button></div>{aggregations.map((aggregation) => { const numeric = ["sum", "average", "minimum", "maximum"].includes(aggregation.function); return <div className="query-row aggregation-row" key={aggregation.id}><select aria-label="Aggregation function" value={aggregation.function} onChange={(event) => setAggregations((items) => items.map((item) => item.id === aggregation.id ? { ...item, function: event.target.value as AggregationRow["function"], property: event.target.value === "count" ? item.property : item.property || root?.properties[0]?.apiName || "" } : item))}>{aggregationLabels.map((entry) => <option value={entry.value} key={entry.value}>{entry.label}</option>)}</select><select aria-label="Aggregation property" value={aggregation.property} onChange={(event) => setAggregations((items) => items.map((item) => item.id === aggregation.id ? { ...item, property: event.target.value } : item))}><option value="">{aggregation.function === "count" ? "All objects" : "Choose property"}</option>{root?.properties.filter((property) => !numeric || isNumeric(property)).map((property) => <option value={property.apiName} key={property.apiName}>{property.displayName}</option>)}</select><input aria-label="Aggregation alias" value={aggregation.alias} onChange={(event) => setAggregations((items) => items.map((item) => item.id === aggregation.id ? { ...item, alias: event.target.value } : item))}/><button aria-label="Remove aggregation" onClick={() => setAggregations((items) => items.filter((item) => item.id !== aggregation.id))}><X size={13}/></button></div>; })}{!aggregations.length && <p className="query-empty">No result summaries configured.</p>}</section>
    </div>
    {message && <div className="query-message" role="status">{message}</div>}
    <footer><button className="button secondary" onClick={onClose}>Cancel</button><button className="button primary" disabled={busy || !rootType} onClick={() => void run()}>{busy ? "Running…" : "Run query"}</button></footer>
  </aside></div>;
}
