import { describe, expect, it } from "vitest";
import { graphqlSourceInput, ontologyDefinition, restSourceInput, type BackendDataSource } from "./context-hub-client";
import type { OntologyCatalog } from "./ontology-catalog";

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

describe("ontology draft serialization", () => {
  it("preserves reusable types, interfaces, derived properties and cardinalities", () => {
    const catalog: OntologyCatalog = {
      objectTypes: [{ apiName: "service", displayName: "Service", properties: [
        { apiName: "id", displayName: "ID", type: "String", identity: true },
        { apiName: "label", displayName: "Label", type: "String", derived: true, expression: "upper(id)" },
      ], implements: ["deployable"] }],
      interfaces: [{ apiName: "deployable", displayName: "Deployable", properties: [], sharedProperties: ["lifecycle"], extends: [] }],
      valueTypes: [{ apiName: "score", displayName: "Score", baseType: "Float64" }],
      structTypes: [{ apiName: "owner", displayName: "Owner", fields: [{ apiName: "name", displayName: "Name", type: "String", required: true }] }],
      sharedProperties: [{ apiName: "lifecycle", displayName: "Lifecycle", type: "String", indexed: true }],
      linkTypes: [{ apiName: "depends_on", displayName: "Depends on", sourceType: "service", targetType: "service", sourceCardinality: "many", targetCardinality: "one", required: true, properties: [{ apiName: "reason", displayName: "Reason", description: "Why the dependency exists", type: "String", indexed: true }] }],
      functions: [],
    };
    const definition = ontologyDefinition("Services", "services", catalog);
    expect(definition.object_types[0].derived_properties[0]).toMatchObject({ api_name: "label", expression: "upper(id)" });
    expect(definition.object_types[0].implements).toEqual(["deployable"]);
    expect(definition.interfaces[0].shared_properties).toEqual(["lifecycle"]);
    expect(definition.value_types[0]).toMatchObject({ api_name: "score", base_type: "float64" });
    expect(definition.struct_types[0].fields[0]).toMatchObject({ api_name: "name", required: true });
    expect(definition.shared_properties[0]).toMatchObject({ api_name: "lifecycle", indexed: true });
    expect(definition.link_types[0].target_cardinality).toBe("one");
    expect(definition.link_types[0]).toMatchObject({ required: true, properties: [{ api_name: "reason", description: "Why the dependency exists", indexed: true }] });
  });
});
