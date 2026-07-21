import type { GraphValue } from "@/lib/graph-data";

export type CastTarget = "string" | "boolean" | "int64" | "float64" | "decimal" | "date" | "timestamp";

export type MappingTransform =
  | { kind: "trim" }
  | { kind: "lowercase" }
  | { kind: "uppercase" }
  | { kind: "cast"; target: CastTarget }
  | { kind: "replace"; from: string; to: string }
  | { kind: "regex_replace"; pattern: string; replacement: string }
  | { kind: "default"; value: string }
  | { kind: "coalesce"; fields: string[] }
  | { kind: "concat"; fields: string[]; separator: string }
  | { kind: "add"; value: number }
  | { kind: "multiply"; value: number }
  | { kind: "parse_date"; format: string }
  | { kind: "parse_timestamp"; format: string };

export type TransformKind = MappingTransform["kind"];

export const transformOptions: Array<{ kind: TransformKind; label: string }> = [
  { kind: "trim", label: "Trim" },
  { kind: "lowercase", label: "Lowercase" },
  { kind: "uppercase", label: "Uppercase" },
  { kind: "cast", label: "Cast" },
  { kind: "replace", label: "Replace" },
  { kind: "regex_replace", label: "Regex replace" },
  { kind: "default", label: "Default" },
  { kind: "coalesce", label: "Coalesce" },
  { kind: "concat", label: "Concatenate" },
  { kind: "add", label: "Add" },
  { kind: "multiply", label: "Multiply" },
  { kind: "parse_date", label: "Parse date" },
  { kind: "parse_timestamp", label: "Parse timestamp" },
];

export function createTransform(kind: TransformKind): MappingTransform {
  switch (kind) {
    case "cast": return { kind, target: "string" };
    case "replace": return { kind, from: "", to: "" };
    case "regex_replace": return { kind, pattern: "", replacement: "" };
    case "default": return { kind, value: "" };
    case "coalesce": return { kind, fields: [] };
    case "concat": return { kind, fields: [], separator: " " };
    case "add": return { kind, value: 0 };
    case "multiply": return { kind, value: 1 };
    case "parse_date": return { kind, format: "%Y-%m-%d" };
    case "parse_timestamp": return { kind, format: "%Y-%m-%dT%H:%M:%S%z" };
    default: return { kind };
  }
}

export function migrateLegacyTransform(value: unknown): MappingTransform[] {
  if (Array.isArray(value)) return value as MappingTransform[];
  if (value === "Trim") return [{ kind: "trim" }];
  if (value === "Lowercase") return [{ kind: "lowercase" }];
  if (value === "Uppercase") return [{ kind: "uppercase" }];
  return [];
}

export function transformLabel(transform: MappingTransform) {
  return transformOptions.find((option) => option.kind === transform.kind)?.label ?? transform.kind;
}

function parseValue(value: string): GraphValue {
  if (!value.length) return "";
  try {
    return JSON.parse(value) as GraphValue;
  } catch {
    return value;
  }
}

function scalar(value: GraphValue) {
  if (value === null || Array.isArray(value) || typeof value === "object") throw new Error("A scalar value is required");
  return value;
}

function asNumber(value: GraphValue) {
  const parsed = Number(scalar(value));
  if (!Number.isFinite(parsed)) throw new Error("The value is not numeric");
  return parsed;
}

function cast(value: GraphValue, target: CastTarget): GraphValue {
  if (value === null) return null;
  if (target === "string") return typeof value === "object" ? JSON.stringify(value) : String(value);
  if (target === "boolean") {
    if (value === true || value === false) return value;
    const normalized = String(scalar(value)).trim().toLowerCase();
    if (["true", "1"].includes(normalized)) return true;
    if (["false", "0"].includes(normalized)) return false;
    throw new Error("The value is not boolean");
  }
  if (target === "int64") return Math.trunc(asNumber(value));
  if (target === "float64" || target === "decimal") return asNumber(value);
  const date = new Date(String(scalar(value)));
  if (Number.isNaN(date.valueOf())) throw new Error("The value is not a date");
  return target === "date" ? date.toISOString().slice(0, 10) : date.toISOString();
}

export function applyTransforms(value: GraphValue, transforms: MappingTransform[], record: Record<string, GraphValue>): GraphValue {
  return transforms.reduce<GraphValue>((current, transform) => {
    switch (transform.kind) {
      case "trim": return String(scalar(current)).trim();
      case "lowercase": return String(scalar(current)).toLowerCase();
      case "uppercase": return String(scalar(current)).toUpperCase();
      case "cast": return cast(current, transform.target);
      case "replace": return String(scalar(current)).split(transform.from).join(transform.to);
      case "regex_replace": return String(scalar(current)).replace(new RegExp(transform.pattern, "g"), transform.replacement);
      case "default": return current === null ? parseValue(transform.value) : current;
      case "coalesce": return current ?? transform.fields.map((field) => record[field]).find((candidate) => candidate !== null && candidate !== undefined) ?? null;
      case "concat": return [current, ...transform.fields.map((field) => record[field])].map((item) => String(scalar(item))).join(transform.separator);
      case "add": return asNumber(current) + transform.value;
      case "multiply": return asNumber(current) * transform.value;
      case "parse_date": return cast(current, "date");
      case "parse_timestamp": return cast(current, "timestamp");
    }
  }, value);
}

export function serializeTransform(transform: MappingTransform) {
  if (transform.kind === "default") return { kind: transform.kind, value: parseValue(transform.value) };
  return transform;
}
