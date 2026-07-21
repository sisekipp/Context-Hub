import { readdir, stat } from "node:fs/promises";
import { resolve } from "node:path";

const target = resolve(process.env.CARGO_TARGET_DIR ?? "target");
const hardLimitGiB = Number(process.env.TARGET_BUDGET_GIB ?? 16);
const warningLimitGiB = Number(process.env.TARGET_WARNING_GIB ?? 8);

async function size(path) {
  let total = 0;
  let entries;
  try { entries = await readdir(path, { withFileTypes: true }); } catch (error) {
    if (error?.code === "ENOENT") return 0;
    throw error;
  }
  for (const entry of entries) {
    const child = resolve(path, entry.name);
    if (entry.isDirectory()) total += await size(child);
    else if (entry.isFile()) total += (await stat(child)).size;
  }
  return total;
}

if (!Number.isFinite(hardLimitGiB) || hardLimitGiB <= 0) throw new Error("TARGET_BUDGET_GIB must be positive");
const entries = await readdir(target, { withFileTypes: true }).catch(() => []);
const parts = [];
for (const entry of entries.filter((item) => item.isDirectory())) {
  parts.push({ name: entry.name, bytes: await size(resolve(target, entry.name)) });
}
parts.sort((left, right) => right.bytes - left.bytes);
const total = parts.reduce((sum, part) => sum + part.bytes, 0);
const gib = total / 1024 ** 3;
console.log(`Rust target: ${gib.toFixed(2)} GiB / ${hardLimitGiB.toFixed(2)} GiB hard limit`);
for (const part of parts.slice(0, 6)) console.log(`  ${part.name.padEnd(18)} ${(part.bytes / 1024 ** 3).toFixed(2)} GiB`);
if (gib > hardLimitGiB) {
  console.error("Target budget exceeded. Run `cargo clean`, or remove an unused build profile before continuing.");
  process.exitCode = 1;
} else if (gib > warningLimitGiB) {
  console.warn(`Warning: target exceeds the ${warningLimitGiB.toFixed(2)} GiB maintenance threshold.`);
}
