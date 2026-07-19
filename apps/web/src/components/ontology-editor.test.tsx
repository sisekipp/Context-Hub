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
    render(<OntologyEditor />);
    expect(screen.getByRole("heading", { name: "Service map" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /Publish/ })).toBeInTheDocument();
  });
});

