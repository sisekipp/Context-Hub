import { mkdir, writeFile } from "node:fs/promises";
import { dirname, resolve } from "node:path";

const outputPath = resolve(process.argv[2] ?? "demo/data/nova-commerce.json");
const serviceCount = 144;
const teams = [
  ["atlas", "Atlas Platform"],
  ["aurora", "Aurora Experience"],
  ["beacon", "Beacon Reliability"],
  ["comet", "Comet Data"],
  ["forge", "Forge Payments"],
  ["helix", "Helix Identity"],
  ["nimbus", "Nimbus Fulfillment"],
  ["pulse", "Pulse Growth"],
];
const domains = ["Checkout", "Identity", "Catalog", "Payments", "Fulfillment", "Observability", "Customer", "Analytics"];
const runtimes = ["Rust", "TypeScript", "Go", "Kotlin", "Python", "Java"];
const regions = ["eu-central-1", "eu-west-1", "us-east-1"];
const heroNames = [
  "Checkout Gateway", "Payment Orchestrator", "Fraud Signal Engine", "Customer Identity", "Product Galaxy",
  "Order Conductor", "Inventory Pulse", "Delivery Navigator", "Merchant Portal", "Pricing Studio",
  "Event Horizon", "Telemetry Core", "Recommendation Lab", "Loyalty Engine", "Revenue Intelligence",
  "Search Nebula", "Edge Router", "Notification Relay", "Data Lake Bridge", "Feature Control Plane",
  "Secrets Guardian", "Audit Stream", "Support Copilot", "Experiment Engine",
];

const ids = Array.from({ length: serviceCount }, (_, index) => `svc-${String(index + 1).padStart(3, "0")}`);
const records = ids.map((serviceId, index) => {
  const team = teams[index % teams.length];
  const domain = domains[index % domains.length];
  const dependencyOffsets = index < 8 ? [8, 16, 24] : [1, 3 + (index % 5), 11 + (index % 7)];
  const dependsOn = [...new Set(dependencyOffsets.map((offset) => ids[(index + offset) % serviceCount]).filter((id) => id !== serviceId))];
  const hero = heroNames[index];
  const number = String(index + 1).padStart(3, "0");
  const tier = index < 12 ? "critical" : index < 48 ? "high" : index < 104 ? "standard" : "internal";
  return {
    service_id: serviceId,
    service_name: hero ?? `${domain} Service ${number}`,
    owner_team: team[0],
    owner_team_name: team[1],
    depends_on: dependsOn,
    domain,
    runtime: runtimes[(index * 5) % runtimes.length],
    environment: index % 9 === 0 ? "staging" : "production",
    region: regions[index % regions.length],
    tier,
    health: index % 29 === 0 ? "degraded" : index % 41 === 0 ? "incident" : "healthy",
    sla_percent: tier === "critical" ? 99.99 : tier === "high" ? 99.95 : 99.9,
    monthly_cost_eur: 850 + ((index * 977) % 41_000),
    requests_per_minute: 1_500 + ((index * 7_919) % 280_000),
    error_rate: Number((((index * 17) % 75) / 10_000).toFixed(4)),
    repository: `nova-commerce/${domain.toLowerCase()}-${serviceId}`,
    description: `${hero ?? `${domain} service ${number}`} is operated by ${team[1]} and runs in ${regions[index % regions.length]}.`,
    tags: [domain.toLowerCase(), tier, runtimes[(index * 5) % runtimes.length].toLowerCase()],
  };
});

await mkdir(dirname(outputPath), { recursive: true });
await writeFile(outputPath, `${JSON.stringify(records, null, 2)}\n`, "utf8");
console.log(`Generated ${records.length} Nova Commerce services with ${records.reduce((sum, record) => sum + record.depends_on.length + 1, 0)} relationships at ${outputPath}`);
