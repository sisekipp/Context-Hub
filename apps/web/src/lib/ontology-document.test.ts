import { describe, expect, it } from "vitest";
import { ontologyDocumentFromBackend } from "./ontology-document";

describe("backend ontology canvas reconstruction", () => {
  it("reconstructs every reusable node and semantic edge from a definition", () => {
    const document = ontologyDocumentFromBackend("{}", JSON.stringify({
      api_name: "services", display_name: "Services", description: null,
      object_types: [{ api_name: "service", display_name: "Service", description: null, properties: [], shared_properties: [], derived_properties: [], implements: ["deployable"] }],
      interfaces: [{ api_name: "deployable", display_name: "Deployable", description: null, properties: [], shared_properties: [], extends: ["resource"] }, { api_name: "resource", display_name: "Resource", description: null, properties: [], shared_properties: [], extends: [] }],
      value_types: [{ api_name: "score", display_name: "Score", base_type: "float64" }],
      struct_types: [{ api_name: "owner", display_name: "Owner", fields: [{ api_name: "name", display_name: "Name", value_type: { kind: "scalar", value_type: { scalar: "string", list: false } }, required: true }] }],
      shared_properties: [{ api_name: "lifecycle", display_name: "Lifecycle", value_type: { kind: "scalar", value_type: { scalar: "string", list: false } } }],
      link_types: [{ api_name: "depends_on", display_name: "Depends on", description: "Dependency", source_type: "service", target_type: "service", source_cardinality: "many", target_cardinality: "one", required: true, properties: [{ api_name: "reason", display_name: "Reason", description: "Explanation", value_type: { scalar: "string", list: false }, required: false, unique: false, identity: false, indexed: true }] }], functions: [],
    }));
    expect(document.nodes.map((node) => node.data.kind)).toEqual(["object", "interface", "interface", "value_type", "struct", "shared_property"]);
    expect(document.edges.map((edge) => edge.data.apiName)).toEqual(["implements", "extends", "depends_on"]);
    expect(document.nodes.find((node) => node.data.apiName === "score")?.data.properties[0].type).toBe("Float64");
    expect(document.edges[2].data).toMatchObject({ required: true, targetCardinality: "one", properties: [{ name: "reason", description: "Explanation", indexed: true }] });
  });
});
