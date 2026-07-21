import { describe, expect, it } from "vitest";
import { defaultOntologyCatalog } from "@/lib/ontology-catalog";
import { buildImportedGraph, parseSource, type LinkMapping, type ObjectMapping } from "./mapping-panel";

const serviceMapping: ObjectMapping = {
  id: "services",
  objectType: "service",
  displayProperty: "name",
  properties: [
    { id: "id", sourceField: "service_id", targetProperty: "id", transforms: [] },
    { id: "name", sourceField: "name", targetProperty: "name", transforms: [{ kind: "trim" }] },
  ],
};

describe("ontology-bound file mapping", () => {
  it("maps actual records and creates a declared cross-type ontology link", () => {
    const records = parseSource("services.json", JSON.stringify([
      { service_id: "svc-1", name: " Billing ", owner: "Payments" },
      { service_id: "svc-2", name: "Ledger", owner: "Payments" },
    ]));
    const links: LinkMapping[] = [{ id: "owner", sourceObjectMappingId: "services", sourceField: "owner", linkType: "owned_by", missingTarget: "create" }];
    const graph = buildImportedGraph({ records, objectMappings: [serviceMapping], linkMappings: links, ontology: defaultOntologyCatalog, fileName: "services.json" });

    expect(graph.recordCount).toBe(2);
    expect(graph.nodes).toHaveLength(3);
    expect(graph.links).toHaveLength(2);
    expect(graph.nodes.find((node) => node.id === "service:svc-1")?.properties.name).toBe("Billing");
    expect(graph.links[0]).toMatchObject({ source: "service:svc-1", target: "team:Payments", label: "owned_by" });
  });

  it("resolves list references between objects through the ontology identity", () => {
    const records = parseSource("services.json", JSON.stringify([
      { service_id: "svc-1", name: "Billing", depends_on: ["svc-2"] },
      { service_id: "svc-2", name: "Ledger", depends_on: ["svc-1"] },
    ]));
    const links: LinkMapping[] = [{ id: "dependency", sourceObjectMappingId: "services", sourceField: "depends_on", linkType: "depends_on", missingTarget: "error" }];
    const graph = buildImportedGraph({ records, objectMappings: [serviceMapping], linkMappings: links, ontology: defaultOntologyCatalog, fileName: "services.json" });

    expect(graph.nodes).toHaveLength(2);
    expect(graph.links).toEqual(expect.arrayContaining([
      expect.objectContaining({ source: "service:svc-1", target: "service:svc-2", label: "depends_on" }),
      expect.objectContaining({ source: "service:svc-2", target: "service:svc-1", label: "depends_on" }),
    ]));
    expect(graph.linkErrorCount).toBe(0);
  });

  it("deduplicates a second object mapping and resolves cross-type targets", () => {
    const records = parseSource("services.json", JSON.stringify([
      { service_id: "svc-1", name: "Billing", owner_team: "Payments" },
      { service_id: "svc-2", name: "Ledger", owner_team: "Payments" },
    ]));
    const teamMapping: ObjectMapping = {
      id: "teams", objectType: "team", displayProperty: "name",
      properties: [
        { id: "team-id", sourceField: "owner_team", targetProperty: "id", transforms: [] },
        { id: "team-name", sourceField: "owner_team", targetProperty: "name", transforms: [] },
      ],
    };
    const links: LinkMapping[] = [{ id: "owner", sourceObjectMappingId: "services", sourceField: "owner_team", linkType: "owned_by", missingTarget: "error" }];
    const graph = buildImportedGraph({ records, objectMappings: [serviceMapping, teamMapping], linkMappings: links, ontology: defaultOntologyCatalog, fileName: "services.json" });

    expect(graph.nodes.filter((node) => node.kind === "Team")).toHaveLength(1);
    expect(graph.links).toHaveLength(2);
    expect(graph.linkErrorCount).toBe(0);
  });
});
