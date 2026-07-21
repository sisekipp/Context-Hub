# Graph query builder

The Explorer's **Query** action opens a visual builder for the public `GraphQuery` contract. It never exposes raw SQL. The API validates every object type, property, link, direction, operator, aggregation, and limit against the selected immutable ontology version before compiling parameterized ClickHouse SQL.

The V1 builder supports:

- One root object type and a result limit between 1 and 5,000 objects.
- Multiple equality, inequality, contains, and range filters on root properties.
- Up to six directed or reverse ontology traversal steps.
- Root-property projections. Traversed context objects retain their properties.
- Ascending or descending typed property sorting for non-traversal queries.
- Count, distinct count, sum, average, minimum, and maximum result summaries.

Aggregations describe the bounded root-object result returned by the query. Numeric functions accept only `Int64`, `Float64`, and `Decimal` properties. Custom property sorting currently cannot be combined with traversal or continuation cursors; the builder disables that combination.

## Incremental exploration

`GraphService.Expand` loads up to 500 incident edges for one selected object and a controlled set of ontology link types. Both object type and object ID scope the lookup, so equal external identifiers in different types cannot cross-match. The API returns the center object, connected objects, and their edges. The frontend merges them into the current graph by stable object and relationship keys.

In both 2D and 3D, double-clicking a node or choosing **Load connected objects** in the inspector executes this one-hop expansion. Local focus, back/forward history, zoom, pan, rotation, labels, and type visibility remain available after the merge.
