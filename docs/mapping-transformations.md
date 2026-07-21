# Mapping transformations

Every property mapping owns an ordered transformation pipeline. The output of one step is the input of the next step, and the saved pipeline is compiled into the restricted DataFusion expression set; users cannot submit SQL.

The visual editor supports:

- Trim, lowercase, and uppercase
- Cast to String, Boolean, Int64, Float64, Decimal, Date, or Timestamp
- Literal and regular-expression replacement
- Typed or textual default values
- Coalesce with other source fields
- Concatenation with other source fields and a configurable separator
- Addition and multiplication
- Date and timestamp parsing with a format string

Fields used by Coalesce and Concatenation are comma-separated source-field names. Default values are interpreted as JSON when possible: `42`, `true`, `null`, and quoted JSON strings retain their JSON type; any other input is stored as plain text.

Transformations on the ontology identity property are also applied when the stable object ID is constructed. This keeps the browser preview, persisted nodes, and generated links aligned.

Every mapped field also defines one failure strategy:

- **Skip row** (`reject_row`) discards the affected source row for that object mapping and increments `rows_rejected`.
- **Use null** (`use_null`) keeps the object and writes an explicit JSON null for that property. Identity values that become null still reject the row because an object cannot be created without an identity.
- **Abort import** (`abort_job`) fails the durable ingestion job with the target field and one-based source-row number in its error message.

DataFusion casts use `TRY_CAST`, and the restricted projection emits an internal per-field error indicator. The worker applies the selected strategy after each Arrow batch is evaluated. Configuration and planning errors, such as invalid identifiers or unsupported expressions, always fail the job because they are not record-level data errors. The browser preview mirrors the same three choices and reports skipped rows before import.

Existing browser drafts that contain the earlier `None`, `Trim`, `Lowercase`, or `Uppercase` single-transform value are migrated automatically when loaded. Mappings without a stored error strategy default to **Skip row**.
