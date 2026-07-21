# Parquet imports

Parquet files use `SOURCE_FILE_FORMAT_PARQUET` and the same workspace-level upload source as JSON, NDJSON, and CSV. The current unary development upload limit remains 32 MiB.

After upload, `DataSourceService.Preview` reads the Parquet schema through Arrow and returns at most 10,000 normalized JSON records to the visual mapping assistant. Boolean and numeric columns retain their value types. The preview conversion is a UI boundary only: an ingestion job reads the original Parquet object from MinIO into Arrow `RecordBatch` values and executes the saved DataFusion mapping without converting the source through JSON.

Invalid Parquet files are rejected before they are stored as data sources. Stored objects retain the same size and SHA-256 verification used by the other upload formats.

A small typed test file can be generated without installing additional tools:

```bash
cargo run -p context-hub-mapping --example generate_parquet_sample -- services.parquet
```

The generated file contains String, Float64, and Boolean columns and can be uploaded through **Data mapping → File**.
