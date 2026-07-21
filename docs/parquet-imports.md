# Parquet imports

Parquet files use `SOURCE_FILE_FORMAT_PARQUET` and the same workspace-level upload source as JSON, NDJSON, and CSV. Files up to 32 MiB use the unary RPC; larger files automatically use resumable 8 MiB multipart parts.

After upload, `DataSourceService.Preview` streams the Parquet object to temporary storage, reads its schema through Arrow, and returns at most 10,000 normalized JSON records to the visual mapping assistant. Boolean and numeric columns retain their value types. Ingestion processes the original Parquet file in 8,192-row Arrow batches and never converts it through JSON.

Invalid Parquet files are rejected before they are stored as data sources. Stored objects retain the same size and SHA-256 verification used by the other upload formats.

A small typed test file can be generated without installing additional tools:

```bash
cargo run -p context-hub-mapping --example generate_parquet_sample -- services.parquet
```

The generated file contains String, Float64, and Boolean columns and can be uploaded through **Data mapping → File**.
