import "@testing-library/jest-dom/vitest";
import { render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

vi.mock("@xyflow/react", async () => {
  const React = await import("react");
  return {
    ReactFlow: ({ children }: { children: React.ReactNode }) => <div>{children}</div>,
    Background: () => null, Controls: () => null, MiniMap: () => null, Handle: () => null,
    Position: { Left: "left", Right: "right" }, BackgroundVariant: { Dots: "dots" },
    addEdge: vi.fn(), useNodesState: (nodes: unknown[]) => [nodes, vi.fn(), vi.fn()], useEdgesState: (edges: unknown[]) => [edges, vi.fn(), vi.fn()],
  };
});

import { OntologyEditor } from "./ontology-editor";

describe("OntologyEditor", () => {
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
});
