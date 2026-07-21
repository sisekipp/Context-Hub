import "@testing-library/jest-dom/vitest";
import { fireEvent, render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@xyflow/react", async () => {
  const React = await import("react");
  return {
    ReactFlow: ({ children }: { children: React.ReactNode }) => <div>{children}</div>,
    Background: () => null, Controls: () => null, MiniMap: () => null, Handle: () => null,
    Position: { Left: "left", Right: "right" }, BackgroundVariant: { Dots: "dots" },
    addEdge: vi.fn(),
    useNodesState: (nodes: unknown[]) => {
      const [value, setValue] = React.useState(nodes);
      return [value, setValue, vi.fn()];
    },
    useEdgesState: (edges: unknown[]) => {
      const [value, setValue] = React.useState(edges);
      return [value, setValue, vi.fn()];
    },
  };
});

import { OntologyEditor } from "./ontology-editor";

describe("OntologyEditor", () => {
  beforeEach(() => localStorage.clear());

  it("renders the seeded ontology workflow", () => {
    render(<OntologyEditor ontologyId="service-map" ontologyName="Service map" seedTemplate onRename={vi.fn()} />);
    expect(screen.getByRole("textbox", { name: "Ontology name" })).toHaveValue("Service map");
    expect(screen.getByRole("button", { name: /Publish/ })).toBeInTheDocument();
  });

  it("starts a newly created ontology without the service-map template", () => {
    render(<OntologyEditor ontologyId="new-ontology" ontologyName="New ontology" seedTemplate={false} onRename={vi.fn()} />);
    expect(screen.queryByText("Service")).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: /Object type/ })).toBeInTheDocument();
  });

  it("edits controlled function implementations and explains the publish boundary", () => {
    localStorage.setItem("context-hub.ontology.functions", JSON.stringify({
      nodes: [{ id: "greeting", type: "ontology", position: { x: 0, y: 0 }, data: {
        kind: "function", displayName: "Greeting", apiName: "greeting", description: "",
        properties: [{ name: "name", type: "String", required: true }], functionOutput: "String",
        implementation: "expression", functionExpression: "concat('Hello ', name)",
      } }], edges: [],
    }));
    render(<OntologyEditor ontologyId="functions" ontologyName="Functions" seedTemplate={false} onRename={vi.fn()} />);
    expect(screen.getByRole("textbox", { name: "Controlled expression" })).toHaveValue("concat('Hello ', name)");
    expect(screen.getByRole("button", { name: "Run published version" })).toBeDisabled();
    expect(screen.getByText(/Publish the current ontology/)).toBeInTheDocument();
  });

  it("duplicates and deletes function nodes in the draft", () => {
    localStorage.setItem("context-hub.ontology.functions", JSON.stringify({
      nodes: [{ id: "greeting", type: "ontology", position: { x: 0, y: 0 }, data: {
        kind: "function", displayName: "Greeting", apiName: "greeting", description: "",
        properties: [{ name: "name", type: "String", required: true }], functionOutput: "String",
        implementation: "expression", functionExpression: "concat('Hello ', name)",
      } }], edges: [],
    }));
    render(<OntologyEditor ontologyId="functions" ontologyName="Functions" seedTemplate={false} onRename={vi.fn()} />);

    fireEvent.click(screen.getByRole("button", { name: "Duplicate function" }));
    expect(screen.getByRole("textbox", { name: /API name/ })).toHaveValue("greeting_copy");
    expect(screen.getByRole("textbox", { name: "Display name" })).toHaveValue("Greeting copy");

    fireEvent.click(screen.getByRole("button", { name: "Delete function" }));
    expect(screen.getByText("Select a node or a link to inspect it.")).toBeInTheDocument();
  });
});
