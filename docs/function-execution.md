# Function execution

Functions are immutable parts of a published ontology version. `FunctionService.Execute` accepts a workspace, ontology version, Function API name, and a JSON object. The backend rejects unknown arguments, missing required inputs, input type mismatches, and output type mismatches before returning a result. V1 Functions are always read-only.

## Controlled expressions

Expressions can reference input names and use literals, parentheses, arithmetic, comparisons, `&&`, `||`, and the fixed functions `concat`, `coalesce`, `lower`, `upper`, `trim`, and `length`. They cannot execute SQL, access environment variables, open files, or call the network.

## External gRPC

Remote providers implement `context_hub.v1.ExternalFunctionService.Invoke`. The request contains the Function API name and validated arguments JSON; the response contains result JSON, which ContextHub validates against the published output type.

Production endpoints must use HTTPS and their exact origin must appear in the comma-separated `FUNCTION_GRPC_ALLOWLIST`, for example:

```text
FUNCTION_GRPC_ALLOWLIST=https://functions.example.com:443,https://internal-functions.example.net:8443
```

With `AUTH_MODE=dev`, loopback endpoints may use HTTP. Connections and calls time out after five seconds, and results are limited to 1 MiB.

## WASM

Artifacts use `object://path/to/module.wasm` or `s3://context-hub/path/to/module.wasm` and are loaded from the configured MinIO bucket. Modules receive no WASI and no host imports. Each invocation has a 16 MiB memory limit, one million fuel units, and a 1 MiB result limit.

The module must export:

- `memory`
- `alloc(length: i32) -> i32`
- the configured entrypoint `(input_pointer: i32, input_length: i32) -> i64`

The input is UTF-8 JSON. The entrypoint returns the result pointer in the upper 32 bits and the UTF-8 JSON result length in the lower 32 bits.
