import { mkdir, writeFile } from "node:fs/promises";
import { dirname, resolve } from "node:path";

const recordCount = Number.parseInt(process.argv[2] ?? "25000", 10);
const outputPath = resolve(process.argv[3] ?? "samples/services-large.json");

if (!Number.isSafeInteger(recordCount) || recordCount < 1 || recordCount > 1_000_000) {
  throw new Error("Record count must be an integer between 1 and 1,000,000");
}

const teams = ["payments", "identity", "platform", "experience", "analytics", "fulfillment", "security", "observability"];
const runtimes = ["rust", "typescript", "java", "python", "go", "kotlin"];
const environments = ["production", "staging", "development"];
const regions = ["eu-central-1", "eu-west-1", "us-east-1", "us-west-2"];
const tiers = ["critical", "high", "standard", "internal"];

const records = Array.from({ length: recordCount }, (_, index) => {
  const number = index + 1;
  const team = teams[index % teams.length];
  const runtime = runtimes[(index * 3) % runtimes.length];
  const environment = environments[(index * 5) % environments.length];
  const padded = String(number).padStart(6, "0");
  const dependencyCount = 1 + (index % 3);
  const dependsOn = [1, 7, 31]
    .slice(0, dependencyCount)
    .map((offset) => `svc-${String(((index + offset) % recordCount) + 1).padStart(6, "0")}`);

  return {
    service_id: `svc-${padded}`,
    service_name: `${team}-${runtime}-service-${padded}`,
    owner_team: team,
    depends_on: dependsOn,
    runtime,
    environment,
    region: regions[(index * 7) % regions.length],
    tier: tiers[(index * 11) % tiers.length],
    is_active: index % 17 !== 0,
    monthly_cost_eur: Number((180 + ((index * 37) % 8200) + (index % 100) / 100).toFixed(2)),
    instance_count: 1 + (index % 48),
    error_rate: Number((((index * 13) % 250) / 10_000).toFixed(4)),
    created_at: new Date(Date.UTC(2022 + (index % 4), index % 12, 1 + (index % 28), index % 24)).toISOString(),
    tags: [team, runtime, environment, `tier:${tiers[(index * 11) % tiers.length]}`],
    metadata: {
      repository: `context-hub-example/${team}-${runtime}-${padded}`,
      version: `${1 + (index % 5)}.${index % 20}.${index % 50}`,
      managed: index % 9 !== 0,
    },
  };
});

await mkdir(dirname(outputPath), { recursive: true });
await writeFile(outputPath, `${JSON.stringify(records, null, 2)}\n`, "utf8");

console.log(`Generated ${records.length.toLocaleString("en-US")} records at ${outputPath}`);
