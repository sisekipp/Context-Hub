import { describe, expect, it } from "vitest";
import { validateGraphQuerySpec } from "./graph-query-builder";
import { defaultOntologyCatalog } from "@/lib/ontology-catalog";
import type { GraphQuerySpec } from "@/lib/context-hub-client";

function spec(patch: Partial<GraphQuerySpec> = {}): GraphQuerySpec {
  return { rootType: "service", filters: [], traversal: [], projection: [], aggregations: [], limit: 500, ...patch };
}

describe("graph query builder validation", () => {
  it("accepts a bounded ontology query", () => {
    expect(validateGraphQuerySpec(spec({ filters: [{ property: "name", operator: "contains", value: "billing" }], aggregations: [{ property: "name", function: "distinct_count", alias: "names" }] }), defaultOntologyCatalog)).toBe("");
  });

  it("rejects sorting combined with traversal", () => {
    expect(validateGraphQuerySpec(spec({ traversal: [{ linkType: "owned_by", targetType: "team", reverse: false }], sort: { property: "name", direction: "ascending" } }), defaultOntologyCatalog)).toContain("cannot be combined");
  });

  it("rejects numeric aggregation over a string property", () => {
    expect(validateGraphQuerySpec(spec({ aggregations: [{ property: "name", function: "average", alias: "average_name" }] }), defaultOntologyCatalog)).toContain("numeric property");
  });
});
