import { createClient } from "@connectrpc/connect";
import { createGrpcWebTransport } from "@connectrpc/connect-web";
import {
  DataSourceService,
  SourceFileFormat,
} from "@/gen/context_hub/v1/context_hub_pb";

export const DEV_WORKSPACE_ID = "00000000-0000-0000-0000-000000000001";

const transport = createGrpcWebTransport({
  baseUrl: process.env.NEXT_PUBLIC_CONTEXT_HUB_API_URL ?? "http://localhost:50051",
  useBinaryFormat: true,
});

const dataSources = createClient(DataSourceService, transport);

function sourceFormat(fileName: string): SourceFileFormat {
  if (/\.(ndjson|jsonl)$/i.test(fileName)) return SourceFileFormat.NDJSON;
  if (/\.csv$/i.test(fileName)) return SourceFileFormat.CSV;
  return SourceFileFormat.JSON;
}

export async function uploadWorkspaceSource(file: File) {
  const response = await dataSources.upload({
    workspaceId: DEV_WORKSPACE_ID,
    name: file.name.replace(/\.[^.]+$/, "") || file.name,
    fileName: file.name,
    format: sourceFormat(file.name),
    content: new Uint8Array(await file.arrayBuffer()),
  });
  if (!response.dataSource) throw new Error("The backend did not return a data source.");
  return {
    id: response.dataSource.id,
    objectKey: response.objectKey,
    sizeBytes: response.sizeBytes,
    sha256: response.sha256,
  };
}
