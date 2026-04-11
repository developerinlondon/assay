## assay.kargo

Kargo continuous promotion. Stages, freight, promotions, warehouses, pipeline status.
Client: `kargo.client(url, token)`.

- `c:stages(namespace)` → [stage] — List stages in namespace
- `c:stage(namespace, name)` → stage — Get stage by name
- `c:stage_status(namespace, name)` → `{phase, current_freight_id, health, conditions}` — Get stage status
- `c:is_stage_healthy(namespace, name)` → bool — Check if stage is healthy (phase "Steady" or condition "Healthy")
- `c:wait_stage_healthy(namespace, name, timeout_secs?)` → true — Wait for stage health. Default 60s.
- `c:freight_list(namespace, opts?)` → [freight] — List freight. `opts`: `{stage, warehouse}` for label filters
- `c:freight(namespace, name)` → freight — Get freight by name
- `c:freight_status(namespace, name)` → status — Get freight status
- `c:promotions(namespace, opts?)` → [promotion] — List promotions. `opts`: `{stage}` filter
- `c:promotion(namespace, name)` → promotion — Get promotion by name
- `c:promotion_status(namespace, name)` → `{phase, message, freight_id}` — Get promotion status
- `c:promote(namespace, stage, freight)` → promotion — Create a promotion to promote freight to stage
- `c:warehouses(namespace)` → [warehouse] — List warehouses
- `c:warehouse(namespace, name)` → warehouse — Get warehouse by name
- `c:projects()` → [project] — List Kargo projects
- `c:project(name)` → project — Get project by name
- `c:pipeline_status(namespace)` → `[{name, phase, freight, healthy}]` — Get pipeline overview of all stages

Example:
```lua
local kargo = require("assay.kargo")
local c = kargo.client("https://kargo.example.com", env.get("KARGO_TOKEN"))
c:promote("my-project", "staging", "freight-abc123")
c:wait_stage_healthy("my-project", "staging", 300)
```
