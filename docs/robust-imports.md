# Robust imports

Files up to 32 MiB use the original unary upload. Larger files use resumable multipart sessions with 8 MiB parts, a 5 GiB maximum, per-part SHA-256 validation, strict sequential part numbers, and a final full-file checksum. The browser stores the active upload ID locally and resumes already accepted parts after a refresh. Sessions expire after 24 hours and may be explicitly aborted.

MinIO composition and ingestion are streaming operations. The API downloads an object into temporary disk storage while verifying its size and SHA-256 digest. JSON arrays are incrementally normalized to NDJSON; JSON/NDJSON, CSV, and Parquet are then decoded in Arrow batches of at most 8,192 rows. Browser previews stop after 10,000 records.

Every asynchronous ingestion worker owns an expiring 10-minute lease. A worker verifies and renews the fenced lease before each ClickHouse write batch. It persists `checkpoint_nodes` after each 5,000-node batch and `checkpoint_edges` after each 20,000-edge batch. On restart or lease expiry, another replica resumes at those offsets. Stable object and edge IDs keep retries idempotent.

REST and GraphQL secrets use a separate credential envelope. Set `CONNECTOR_CREDENTIAL_KEY` to a base64-encoded 32-byte key in every non-development deployment. ContextHub refuses to start outside `AUTH_MODE=dev` when the key is missing or invalid. Rotating this key requires decrypting and re-encrypting existing envelopes before switching all replicas.
