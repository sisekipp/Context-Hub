import { mkdir, writeFile } from "node:fs/promises";
import { dirname, resolve } from "node:path";
import { performance } from "node:perf_hooks";

const options = Object.fromEntries(process.argv.slice(2).map((argument) => {
  const [key, value = "true"] = argument.replace(/^--/, "").split("=");
  return [key, value];
}));
const nodes = integer(options.nodes ?? "1000000", "nodes", 1, 10_000_000);
const edgesPerNode = integer(options["edges-per-node"] ?? "5", "edges-per-node", 1, 50);
const iterations = integer(options.iterations ?? "20", "iterations", 1, 10_000);
const concurrency = integer(options.concurrency ?? "8", "concurrency", 1, 128);
const assertBudgets = options.assert === "true";
const outputPath = resolve(options.output ?? "benchmark-results/graph.json");
const clickhouseUrl = process.env.CLICKHOUSE_URL ?? "http://127.0.0.1:8123";
const database = process.env.CLICKHOUSE_DATABASE ?? "context_hub";
const user = process.env.CLICKHOUSE_USER ?? "context_hub";
const password = process.env.CLICKHOUSE_PASSWORD ?? "context_hub";
const workspace = "00000000-0000-0000-0000-00000000b001";
const version = "00000000-0000-0000-0000-00000000b002";
const source = "00000000-0000-0000-0000-00000000b003";

function integer(value, name, minimum, maximum) {
  const parsed = Number(value);
  if (!Number.isSafeInteger(parsed) || parsed < minimum || parsed > maximum) throw new Error(`${name} must be an integer from ${minimum} to ${maximum}`);
  return parsed;
}

async function sql(query) {
  const url = new URL(clickhouseUrl);
  url.searchParams.set("database", database);
  const response = await fetch(url, {
    method: "POST",
    headers: { Authorization: `Basic ${Buffer.from(`${user}:${password}`).toString("base64")}` },
    body: query,
    signal: AbortSignal.timeout(Number(process.env.BENCHMARK_SQL_TIMEOUT_MS ?? 900_000)),
  });
  if (!response.ok) throw new Error(`ClickHouse ${response.status}: ${await response.text()}`);
  return response.text();
}

const tables = ["property_string_index", "graph_edges", "graph_nodes"];
console.log(`Preparing ${nodes.toLocaleString()} nodes and ${(nodes * edgesPerNode).toLocaleString()} edges…`);
for (const table of tables) await sql(`ALTER TABLE ${table} DELETE WHERE workspace_id = toUUID('${workspace}') SETTINGS mutations_sync = 1`);

const loadStarted = performance.now();
await sql(`INSERT INTO graph_nodes (workspace_id, ontology_version_id, object_type, object_id, source_id, external_id, properties, version, deleted)
SELECT toUUID('${workspace}'), toUUID('${version}'), 'service', concat('service:', toString(number)), toUUID('${source}'), toString(number), CAST(concat('{\"id\":\"', toString(number), '\",\"name\":\"Service ', toString(number), '\",\"tier\":\"tier-', toString(number % 4), '\"}') AS JSON), 1, false FROM numbers(${nodes})`);
await sql(`INSERT INTO property_string_index
SELECT toUUID('${workspace}'), toUUID('${version}'), 'service', 'tier', concat('tier-', toString(number % 4)), concat('service:', toString(number)), 1, false FROM numbers(${nodes})`);
await sql(`INSERT INTO graph_edges (workspace_id, ontology_version_id, link_type, edge_id, source_type, source_id, target_type, target_id, data_source_id, properties, version, deleted)
SELECT toUUID('${workspace}'), toUUID('${version}'), 'depends_on', concat('edge:', toString(number)), 'service', concat('service:', toString(intDiv(number, ${edgesPerNode}))), 'service', concat('service:', toString((intDiv(number, ${edgesPerNode}) + (number % ${edgesPerNode}) + 1) % ${nodes})), toUUID('${source}'), CAST('{}' AS JSON), 1, false FROM numbers(${nodes * edgesPerNode})`);
await sql("SYSTEM FLUSH LOGS");
const loadSeconds = (performance.now() - loadStarted) / 1000;

const cases = [
  {
    name: "property-index",
    budgetMs: Number(process.env.BENCHMARK_INDEX_P95_MS ?? 500),
    query: `SELECT count() FROM property_string_index FINAL WHERE workspace_id = toUUID('${workspace}') AND ontology_version_id = toUUID('${version}') AND object_type = 'service' AND property = 'tier' AND value = 'tier-2' AND deleted = false FORMAT JSON`,
  },
  {
    name: "one-hop-traversal",
    budgetMs: Number(process.env.BENCHMARK_TRAVERSAL_P95_MS ?? 1000),
    query: `SELECT count() FROM graph_edges FINAL WHERE workspace_id = toUUID('${workspace}') AND ontology_version_id = toUUID('${version}') AND link_type = 'depends_on' AND source_id = 'service:42' AND deleted = false FORMAT JSON`,
  },
  {
    name: "two-hop-traversal",
    budgetMs: Number(process.env.BENCHMARK_TWO_HOP_P95_MS ?? 3500),
    query: `SELECT count() FROM graph_edges AS first FINAL INNER JOIN graph_edges AS second FINAL ON second.workspace_id = first.workspace_id AND second.ontology_version_id = first.ontology_version_id AND second.source_id = first.target_id WHERE first.workspace_id = toUUID('${workspace}') AND first.ontology_version_id = toUUID('${version}') AND first.link_type = 'depends_on' AND second.link_type = 'depends_on' AND first.source_id = 'service:42' AND first.deleted = false AND second.deleted = false FORMAT JSON`,
  },
];

async function measure(testCase) {
  await sql(testCase.query);
  const durations = [];
  for (let offset = 0; offset < iterations; offset += concurrency) {
    await Promise.all(Array.from({ length: Math.min(concurrency, iterations - offset) }, async () => {
      const started = performance.now();
      await sql(testCase.query);
      durations.push(performance.now() - started);
    }));
  }
  durations.sort((a, b) => a - b);
  const percentile = (ratio) => durations[Math.min(durations.length - 1, Math.ceil(durations.length * ratio) - 1)];
  return { name: testCase.name, samples: durations.length, concurrency, p50Ms: percentile(0.5), p95Ms: percentile(0.95), maxMs: durations.at(-1), budgetMs: testCase.budgetMs };
}

const results = [];
for (const testCase of cases) {
  const result = await measure(testCase);
  results.push(result);
  console.log(`${result.name}: p50 ${result.p50Ms.toFixed(1)} ms · p95 ${result.p95Ms.toFixed(1)} ms · max ${result.maxMs.toFixed(1)} ms`);
}
const report = { generatedAt: new Date().toISOString(), nodes, edges: nodes * edgesPerNode, loadSeconds, iterations, concurrency, results };
await mkdir(dirname(outputPath), { recursive: true });
await writeFile(outputPath, `${JSON.stringify(report, null, 2)}\n`);
console.log(`Loaded in ${loadSeconds.toFixed(1)} s. Report: ${outputPath}`);
if (assertBudgets && results.some((result) => result.p95Ms > result.budgetMs)) {
  console.error("One or more p95 latency budgets were exceeded.");
  process.exitCode = 1;
}
