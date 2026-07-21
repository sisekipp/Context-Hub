# ContextHub Nova Commerce demo

This demo models a fictional commerce platform with 144 services, eight owning teams, and a dense service-dependency graph. It demonstrates ontology-bound data mapping, typed object identities, link creation, provenance, graph search, node expansion, query building, and the shared 2D/3D explorer model.

Generate the deterministic source again with:

```bash
node scripts/generate-demo-data.mjs
```

Import `demo/data/nova-commerce.json` through **Data mapping** in the `Service Map` ontology. The default mapping binds `service_id` and `service_name` to `Service`, creates `owned_by` links from `owner_team`, and creates `depends_on` links from the list-valued references.
