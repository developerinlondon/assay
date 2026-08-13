---
category: AI Agents & Workflow
tagline: Plane REST API — projects, work items, cycles, modules, states, labels, comments, and links
---

## assay.plane

Plane client covering sprint execution on self-hosted or cloud Plane: projects, work items, Cycles
(Plane's sprints), Modules, workflow states, labels, members, comments, and links.

### Client

```lua
local plane = require("assay.plane")
local c = plane.client({
  api_key = env.get("PLANE_API_KEY"),
  workspace = "acme",
  base_url = "https://plane.example.com",
})
```

`plane.client(opts)` accepts:

- `api_key` — a Plane API key. Falls back to `PLANE_API_KEY`.
- `workspace` — the workspace slug, which appears in every path. Falls back to `PLANE_WORKSPACE`.
- `base_url` — falls back to `PLANE_BASE_URL`, then `https://api.plane.so`. Point this at your own
  host for a self-hosted instance.

The key travels in `X-API-Key`. Plane does not read `Authorization`, so a Bearer token is silently
ignored rather than rejected.

A client with a blank workspace slug errors on first use instead of building `/workspaces//`.

### Projects

- `c.projects:list(opts?)` -> `[project]`
- `c.projects:get(project_id)` -> `project`|nil
- `c.projects:create(project)` -> `project`
- `c.projects:update(project_id, patch)` -> `project`
- `c.projects:delete(project_id)` -> `true`

### Work items

- `c.items:page(list_opts?)` -> `{items, next_cursor, has_more}` — one page
- `c.items:list(project_id, opts?)` -> `[item]` — first page only
- `c.items:get(project_id, item_id)` -> `item`|nil
- `c.items:create(project_id, item)` -> `item`
- `c.items:update(project_id, item_id, patch)` -> `item`
- `c.items:delete(project_id, item_id)` -> `true`

Collections answer with a cursor envelope (`results`, `next_cursor`, `next_page_results`); a few
smaller endpoints answer with a bare array, and both are accepted.

### Cycles (sprints)

- `c.cycles:list(project_id, opts?)` -> `[cycle]`
- `c.cycles:get(project_id, cycle_id)` -> `cycle`|nil
- `c.cycles:create(project_id, cycle)` -> `cycle`
- `c.cycles:update(project_id, cycle_id, patch)` -> `cycle`
- `c.cycles:delete(project_id, cycle_id)` -> `true`
- `c.cycles:add_items(project_id, cycle_id, item_ids)` -> `true`

### Modules, states, labels, members

- `c.modules:list / :get / :create / :update / :delete`
- `c.states:list(project_id, opts?)` -> `[state]`, `c.states:create(project_id, state)`
- `c.labels:list(project_id, opts?)` -> `[label]`, `c.labels:create(project_id, label)`
- `c.members:list(opts?)` -> `[member]` — workspace-scoped

### Comments and links

Work items are served under `/work-items/`, but their comments and links are rooted under `/issues/`
— a leftover from the pre-rename API. The module keeps both paths straight.

- `c.comments:list(project_id, item_id, opts?)` -> `[comment]`
- `c.comments:create(project_id, item_id, body)` -> `comment` — a string is wrapped as
  `comment_html`; pass a table to send fields verbatim
- `c.links:list(project_id, item_id, opts?)` -> `[link]`
- `c.links:create(project_id, item_id, link)` -> `link`

### Intake

- `c.intake:list(opts?)` -> `[issue]` — workspace intake queue

### Helpers

- `plane.all_items(c, project_id, opts?)` -> `[item]` — follows `next_cursor` to the end, bounded by
  `opts.max_pages` (default 50)
- `plane.find_item_by_name(c, project_id, name, opts?)` -> `item`|nil — Plane has no name filter, so
  this walks the project
- `plane.ensure_item(c, project_id, spec, opts?)` -> `item` — creates unless the name is taken
- `plane.resolve_project(c, name?)` -> `project` — the only project, or the one matching a name.
  Errors rather than guessing when several are visible.

### Example — file this sprint's work

```lua
local plane = require("assay.plane")
local c = plane.client({ workspace = "acme" })

local project = plane.resolve_project(c, "Development")
local cycle = c.cycles:create(project.id, {
  name = "DEV Sprint 2026-W33",
  start_date = "2026-08-10",
  end_date = "2026-08-16",
})

local item = plane.ensure_item(c, project.id, { name = "Publish the Q3 pricing page" })
c.cycles:add_items(project.id, cycle.id, { item.id })
c.links:create(project.id, item.id, { url = "https://github.com/acme/site/issues/42" })
```
