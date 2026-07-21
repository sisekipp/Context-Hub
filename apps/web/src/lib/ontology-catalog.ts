export type OntologyProperty = {
  apiName: string;
  displayName: string;
  type: string;
  identity?: boolean;
  derived?: boolean;
  shared?: boolean;
  required?: boolean;
  indexed?: boolean;
  unique?: boolean;
  description?: string;
  expression?: string;
  reference?: string;
};

export type OntologyObjectType = {
  apiName: string;
  displayName: string;
  description?: string;
  properties: OntologyProperty[];
  sharedProperties?: string[];
  implements?: string[];
};

export type OntologyLinkType = {
  apiName: string;
  displayName: string;
  sourceType: string;
  targetType: string;
  properties: OntologyProperty[];
  description?: string;
  sourceCardinality?: "one" | "many";
  targetCardinality?: "one" | "many";
  required?: boolean;
};

export type OntologyInterface = {
  apiName: string;
  displayName: string;
  description?: string;
  properties: OntologyProperty[];
  sharedProperties: string[];
  extends: string[];
};

export type OntologyValueType = {
  apiName: string;
  displayName: string;
  description?: string;
  baseType: string;
};

export type OntologyStructType = {
  apiName: string;
  displayName: string;
  description?: string;
  fields: OntologyProperty[];
};

export type OntologySharedProperty = {
  apiName: string;
  displayName: string;
  description?: string;
  type: string;
  required?: boolean;
  indexed?: boolean;
  reference?: string;
};

export type OntologyCatalog = {
  objectTypes: OntologyObjectType[];
  linkTypes: OntologyLinkType[];
  interfaces: OntologyInterface[];
  valueTypes: OntologyValueType[];
  structTypes: OntologyStructType[];
  sharedProperties: OntologySharedProperty[];
  functions: OntologyFunction[];
};

export type OntologyFunction = {
  apiName: string;
  displayName: string;
  description: string;
  inputs: OntologyProperty[];
  output: string;
  outputReference?: string;
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
      implements: ["deployable"],
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
  interfaces: [{ apiName: "deployable", displayName: "Deployable", description: "Common fields of deployable objects", properties: [{ apiName: "environment", displayName: "environment", type: "String" }], sharedProperties: [], extends: [] }],
  valueTypes: [],
  structTypes: [],
  sharedProperties: [],
  functions: [],
};
