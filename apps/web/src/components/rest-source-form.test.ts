import { describe, expect, it } from "vitest";
import { validateRestSourceInput } from "./rest-source-form";
import type { RestSourceInput } from "@/lib/context-hub-client";

function source(patch: Partial<RestSourceInput> = {}): RestSourceInput {
  return {
    name: "Service API",
    url: "https://api.example.com/services",
    recordPath: "data.items",
    headers: [],
    query: [],
    pagination: "none",
    pageParameter: "page",
    pageStart: 1,
    pageSizeParameter: "limit",
    pageSize: 100,
    cursorParameter: "cursor",
    nextCursorPath: "meta.next",
    maxPages: 100,
    maxBytes: 32 * 1024 * 1024,
    timeoutSeconds: 30,
    retryAttempts: 2,
    ...patch,
  };
}

describe("REST source validation", () => {
  it("accepts a bounded page source", () => {
    expect(validateRestSourceInput(source({ pagination: "page" }))).toBe("");
  });

  it("rejects unsupported URLs and incomplete cursors", () => {
    expect(validateRestSourceInput(source({ url: "file:///tmp/data.json" }))).toContain("HTTP");
    expect(validateRestSourceInput(source({ pagination: "cursor", nextCursorPath: "" }))).toContain("next-cursor");
  });

  it("enforces the backend protection limits", () => {
    expect(validateRestSourceInput(source({ maxPages: 1001 }))).toContain("1,000");
    expect(validateRestSourceInput(source({ retryAttempts: 6 }))).toContain("0 and 5");
  });
});
