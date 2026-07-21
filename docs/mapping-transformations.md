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

Existing browser drafts that contain the earlier `None`, `Trim`, `Lowercase`, or `Uppercase` single-transform value are migrated automatically when loaded.
