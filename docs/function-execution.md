# Function execution

Functions are immutable parts of a published ontology version. Draft Function nodes can be duplicated and deleted in the editor. `FunctionService.Execute` accepts a workspace, ontology version, Function API name, and a JSON object. The backend rejects unknown arguments, missing required inputs, input type mismatches, and output type mismatches before returning a result. V1 Functions are always read-only.

## Controlled expressions

Expressions can reference input names and use literals, parentheses, arithmetic, comparisons, `&&`, `||`, and the fixed functions `concat`, `coalesce`, `lower`, `upper`, `trim`, and `length`. Publish validates the syntax, referenced inputs, supported functions, and fixed function arities without executing the expression. Expressions cannot execute SQL, access environment variables, open files, or call the network.

## External gRPC

Remote providers implement `context_hub.v1.ExternalFunctionService.Invoke`. The request contains the Function API name and validated arguments JSON; the response contains result JSON, which ContextHub validates against the published output type.

Production endpoints must use HTTPS and their exact origin must appear in the comma-separated `FUNCTION_GRPC_ALLOWLIST`, for example:

```text
FUNCTION_GRPC_ALLOWLIST=https://functions.example.com:443,https://internal-functions.example.net:8443
```

With `AUTH_MODE=dev`, loopback endpoints may use HTTP. Connections and calls time out after five seconds, and results are limited to 1 MiB.

The editor's **Test provider without publish** action calls the same bounded provider contract with the current endpoint, method, Function name, and arguments. It validates the transport configuration and shows the provider response or detailed error without changing an ontology version.

## WASM

Artifacts can be uploaded from the Function inspector. ContextHub validates the WASM module before writing it to MinIO, stores its checksum and metadata in ClickHouse, and fills the selected Function's `object://` URI. Unreferenced artifacts can be deleted from the same list; artifacts referenced by any draft or immutable version are protected. Modules receive no WASI and no host imports. Each invocation has a 16 MiB memory limit, one million fuel units, and a 1 MiB result limit.

The module must export:

- `memory`
- `alloc(length: i32) -> i32`
- the configured entrypoint `(input_pointer: i32, input_length: i32) -> i64`

The input is UTF-8 JSON. The entrypoint returns the result pointer in the upper 32 bits and the UTF-8 JSON result length in the lower 32 bits.

## Execution history

Every attempted runtime execution that reaches a Function implementation creates a ClickHouse history record with its ontology version, Function API name, executor, state, duration, timestamp, arguments, result, and detailed failure text. The inspector lists the most recent attempts for the selected published Function. Invalid JSON and requests for unknown or unpublished Functions are rejected before execution and therefore do not create a history entry.

Arguments and results may contain sensitive application data. A production deployment should apply an organization-specific retention and redaction policy before enabling Functions for such payloads.
