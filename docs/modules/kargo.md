## assay.kargo

Kargo continuous promotion. Stages, freight, promotions, warehouses, pipeline status.
Client: `kargo.client(url, token)`.

### Stages

- `c.stages:list(namespace)` -> [stage] -- List stages in namespace
- `c.stages:get(namespace, name)` -> stage -- Get stage by name
- `c.stages:status(namespace, name)` -> `{phase, current_freight_id, health, conditions}` -- Get stage status
- `c.stages:is_healthy(namespace, name)` -> bool -- Check if stage is healthy (phase "Steady" or condition "Healthy")
- `c.stages:wait_healthy(namespace, name, timeout_secs?)` -> true -- Wait for stage health. Default 60s.
- `c.stages:pipeline_status(namespace)` -> `[{name, phase, freight, healthy}]` -- Get pipeline overview of all stages

### Freight

- `c.freight:list(namespace, opts?)` -> [freight] -- List freight. `opts`: `{stage, warehouse}` for label filters
- `c.freight:get(namespace, name)` -> freight -- Get freight by name
- `c.freight:status(namespace, name)` -> status -- Get freight status

### Promotions

- `c.promotions:list(namespace, opts?)` -> [promotion] -- List promotions. `opts`: `{stage}` filter
- `c.promotions:get(namespace, name)` -> promotion -- Get promotion by name
- `c.promotions:status(namespace, name)` -> `{phase, message, freight_id}` -- Get promotion status
- `c.promotions:create(namespace, stage, freight)` -> promotion -- Create a promotion to promote freight to stage

### Warehouses

- `c.warehouses:list(namespace)` -> [warehouse] -- List warehouses
- `c.warehouses:get(namespace, name)` -> warehouse -- Get warehouse by name

### Projects

- `c.projects:list()` -> [project] -- List Kargo projects
- `c.projects:get(name)` -> project -- Get project by name

Example:
```lua
local kargo = require("assay.kargo")
local c = kargo.client("https://kargo.example.com", env.get("KARGO_TOKEN"))
c.promotions:create("my-project", "staging", "freight-abc123")
c.stages:wait_healthy("my-project", "staging", 300)
```
