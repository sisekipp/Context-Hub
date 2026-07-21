import { describe, expect, it } from "vitest";
import { validateGraphqlSourceInput } from "./graphql-source-form";
import type { GraphqlSourceInput } from "@/lib/context-hub-client";

function source(patch: Partial<GraphqlSourceInput> = {}): GraphqlSourceInput {
  return {
    name: "Service Graph",
    url: "https://api.example.com/graphql",
    query: "query Services { services { id name } }",
    variables: "{}",
    recordPath: "data.services",
    headers: [],
    cursorEnabled: false,
    cursorVariable: "after",
    nextCursorPath: "data.services.pageInfo.endCursor",
    maxPages: 100,
    maxBytes: 32 * 1024 * 1024,
    timeoutSeconds: 30,
    retryAttempts: 2,
    ...patch,
  };
}

describe("GraphQL source validation", () => {
  it("accepts a bounded query", () => {
    expect(validateGraphqlSourceInput(source())).toBe("");
  });

  it("rejects invalid variables and cursor names", () => {
    expect(validateGraphqlSourceInput(source({ variables: "[]" }))).toContain("JSON object");
    expect(validateGraphqlSourceInput(source({ cursorEnabled: true, cursorVariable: "after.value" }))).toContain("valid variable");
  });

  it("requires a response record path", () => {
    expect(validateGraphqlSourceInput(source({ recordPath: "" }))).toContain("response path");
  });
});
