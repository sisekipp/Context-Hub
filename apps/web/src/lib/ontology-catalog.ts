export type OntologyProperty = {
  apiName: string;
  displayName: string;
  type: string;
  identity?: boolean;
  derived?: boolean;
  required?: boolean;
};

export type OntologyObjectType = {
  apiName: string;
  displayName: string;
  properties: OntologyProperty[];
};

export type OntologyLinkType = {
  apiName: string;
  displayName: string;
  sourceType: string;
  targetType: string;
  properties: OntologyProperty[];
};

export type OntologyCatalog = {
  objectTypes: OntologyObjectType[];
  linkTypes: OntologyLinkType[];
  functions: OntologyFunction[];
};

export type OntologyFunction = {
  apiName: string;
  displayName: string;
  description: string;
  inputs: OntologyProperty[];
  output: string;
  implementation: "expression" | "external_grpc" | "wasm";
  expression: string;
  endpoint: string;
  method: string;
  artifactUri: string;
  entrypoint: string;
};

export const defaultOntologyCatalog: OntologyCatalog = {
  objectTypes: [
    {
      apiName: "service",
      displayName: "Service",
      properties: [
        { apiName: "id", displayName: "ID", type: "String", identity: true },
        { apiName: "name", displayName: "Name", type: "String" },
        { apiName: "display_label", displayName: "Display label", type: "String", derived: true },
      ],
    },
    {
      apiName: "team",
      displayName: "Team",
      properties: [
        { apiName: "id", displayName: "ID", type: "String", identity: true },
        { apiName: "name", displayName: "Name", type: "String" },
      ],
    },
  ],
  linkTypes: [
    { apiName: "owned_by", displayName: "Owned by", sourceType: "service", targetType: "team", properties: [] },
    { apiName: "depends_on", displayName: "Depends on", sourceType: "service", targetType: "service", properties: [] },
  ],
  functions: [],
};
