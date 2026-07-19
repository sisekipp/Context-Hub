export type GraphValue = string | number | boolean | null | GraphValue[] | { [key: string]: GraphValue };

export type ImportedGraphNode = {
  id: string;
  name: string;
  kind: string;
  group: string;
  color: string;
  properties: Record<string, GraphValue>;
};

export type ImportedGraphLink = {
  source: string;
  target: string;
  label: string;
  properties: Record<string, GraphValue>;
};

export type ImportedGraph = {
  nodes: ImportedGraphNode[];
  links: ImportedGraphLink[];
  sourceName: string;
  importedAt: string;
  recordCount: number;
  skippedCount: number;
  linkErrorCount: number;
  ontologyBindings: { objectTypes: string[]; linkTypes: string[] };
};

export const emptyGraph: ImportedGraph = {
  nodes: [],
  links: [],
  sourceName: "",
  importedAt: "",
  recordCount: 0,
  skippedCount: 0,
  linkErrorCount: 0,
  ontologyBindings: { objectTypes: [], linkTypes: [] },
};
