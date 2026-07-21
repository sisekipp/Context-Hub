import { describe, expect, it } from "vitest";
import { graphqlSourceInput, restSourceInput, type BackendDataSource } from "./context-hub-client";

function source(kind: BackendDataSource["kind"], configuration: unknown): BackendDataSource {
  return { id: "source-1", name: "Catalog", fileName: "Catalog.json", kind, configurationJson: JSON.stringify(configuration) };
}

describe("saved connector configuration", () => {
  it("hydrates REST editing fields including pagination and key-value maps", () => {
    const input = restSourceInput(source("rest", {
      url: "https://api.example.com/services",
      headers: { "X-Tenant": "development" },
      query: { active: true },
      record_path: "data.items",
      pagination: { mode: "page", parameter: "page", start: 2, page_size_parameter: "limit", page_size: 50 },
      max_pages: 12,
      max_bytes: 1024,
      timeout_seconds: 9,
      retry_attempts: 1,
    }));

    expect(input).toMatchObject({ id: "source-1", name: "Catalog", pagination: "page", pageStart: 2, pageSize: 50, recordPath: "data.items" });
    expect(input.headers).toEqual([{ key: "X-Tenant", value: "development" }]);
    expect(input.query).toEqual([{ key: "active", value: "true" }]);
  });

  it("hydrates GraphQL variables and cursor settings", () => {
    const input = graphqlSourceInput(source("graphql", {
      url: "https://api.example.com/graphql",
      query: "query Catalog { services { id } }",
      variables: { tenant: "development" },
      headers: {},
      record_path: "data.services",
      pagination: { mode: "cursor", variable: "after", next_cursor_path: "data.pageInfo.endCursor" },
    }));

    expect(input.cursorEnabled).toBe(true);
    expect(input.cursorVariable).toBe("after");
    expect(JSON.parse(input.variables)).toEqual({ tenant: "development" });
  });
});
