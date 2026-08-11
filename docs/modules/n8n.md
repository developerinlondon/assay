---
category: AI Agents & Workflow
tagline: n8n public REST API — workflows, executions, credentials, projects, variables, plus idempotent reconcilers (v0.18.0+)
---

## assay.n8n

n8n public REST API (`/api/v1`) client. Client: `n8n.client(url, {api_key="..."})` — `api_key` falls
back to `N8N_API_KEY`, and travels in the `X-N8N-API-KEY` header. `url` is the instance base URL
without `/api/v1`; override the API root with `opts.api_path` if the instance mounts it elsewhere.

Every collection is cursor-paginated. `:list(opts?)` returns the item array of the **first page**;
`:page(opts?)` returns the raw `{data, nextCursor}` envelope; `n8n.all(section, opts?)` walks every
page. All list methods accept `limit` (max 250) and `cursor` alongside their own filters.

### Workflows

- `c.workflows:list(opts?)` → [workflow] — `opts`:
  `{active, tags, name, projectId,
  excludePinnedData, limit, cursor}`. `name` is a substring match
  server-side.
- `c.workflows:page(opts?)` → `{data, nextCursor}`
- `c.workflows:get(id, opts?)` → workflow|nil — `opts`: `{excludePinnedData}`
- `c.workflows:create(wf)` → workflow — `wf`: `{name, nodes, connections, settings?, staticData?}`
- `c.workflows:update(id, wf)` → workflow — Full replacement, same body as create
- `c.workflows:delete(id)` → workflow
- `c.workflows:activate(id)` / `:deactivate(id)` → workflow
- `c.workflows:publish(id)` / `:unpublish(id)` → workflow
- `c.workflows:archive(id)` / `:unarchive(id)` → workflow
- `c.workflows:transfer(id, project_id)` → table — Move the workflow to another project
- `c.workflows:tags(id)` → [tag]
- `c.workflows:set_tags(id, tag_ids)` → [tag] — Replaces the whole set; accepts ID strings
- `c.workflows:history(id, opts?)` → table — Version history
- `c.workflows:version(id, version_id)` → table

`create`/`update` pin an empty `nodes` to a JSON array and an absent `connections`/`settings` to a
JSON object. An empty Lua table encodes as `{}`, which is correct for the latter two but is rejected
by n8n for the node list.

### Test Runs

- `c.test_runs:list(workflow_id, opts?)` → table — `opts`: `{status}`
- `c.test_runs:create(workflow_id, body?)` → table
- `c.test_runs:get(workflow_id, run_id)` → table
- `c.test_runs:cancel(workflow_id, run_id)` → table
- `c.test_runs:cases(workflow_id, run_id)` → table

### Executions

- `c.executions:list(opts?)` → [execution] — `opts`:
  `{status, workflowId, projectId, includeData,
  ignoreDataSizeLimit, limit, cursor}`. `status` is
  `error` | `success` | `waiting`.
- `c.executions:page(opts?)` → `{data, nextCursor}`
- `c.executions:get(id, opts?)` → execution|nil — `opts`: `{includeData, ignoreDataSizeLimit}`
- `c.executions:delete(id)` → execution
- `c.executions:retry(id, body?)` → table
- `c.executions:stop(id)` → table
- `c.executions:stop_all(body?)` → table
- `c.executions:tags(id)` → [tag] · `c.executions:set_tags(id, tag_ids)` → [tag]

### Credentials

- `c.credentials:list(opts?)` → [credential] · `c.credentials:page(opts?)` → envelope
- `c.credentials:get(id)` → credential|nil
- `c.credentials:create(cred)` → credential — `cred`: `{name, type, data}`
- `c.credentials:update(id, cred)` → credential
- `c.credentials:delete(id)` → credential
- `c.credentials:test(id, body?)` → table
- `c.credentials:schema(type_name)` → schema — JSON schema for a credential type
- `c.credentials:transfer(id, project_id)` → table

### Tags, Variables, Projects, Folders, Users

- `c.tags:list/page/get/create/update/delete` — `tag`: `{name}`
- `c.variables:list/page/create/update/delete` — `variable`: `{key, value, type?}`; list `opts`:
  `{projectId, state}`
- `c.projects:list/page/create/update/delete`, plus `:users(project_id)`,
  `:add_users(project_id, relations)`, `:remove_user(project_id, user_id)`,
  `:set_user_role(project_id, user_id, role)`
- `c.folders:list(project_id, opts?)` → [folder] — `opts`: `{filter, select, sortBy, skip, take}`;
  plus `:get`, `:create`, `:update`, `:delete(project_id, folder_id, opts?)` where `opts` may carry
  `transferToFolderId`
- `c.users:list/page/get/create/delete`, plus `:set_role(id_or_email, role)`. `create` takes an
  array of `{email, role}` invitations.

### Source Control, Audit

- `c.source_control:pull(body?)` → table — Pull from the configured git remote. `body`:
  `{force?, variables?}`
- `c.audit:generate(body?)` → report — Security audit. `body`:
  `{additionalOptions = {categories, daysAbandonedWorkflow}}`

### Data Tables

- `c.data_tables:list(opts?)/get/create/update/delete`
- Rows: `:rows(id, opts?)`, `:insert_rows(id, body)`, `:update_rows(id, body)`,
  `:upsert_rows(id, body)`, `:clear_rows(id)`, `:delete_rows(id, opts?)`
- Columns: `:columns(id)`, `:add_column(id, column)`, `:update_column(id, column_id, column)`,
  `:delete_column(id, column_id)`

### Instance Administration

- `c.community_packages:list/install/update/uninstall`
- `c.settings:security_policy()/set_security_policy(body)`, `:otel()/set_otel(body)`,
  `:test_otel_trace(body?)`, `:saml()/set_saml(body)`
- `c.log_streaming:event_types()`, `:destinations()`, `:get_destination(id)`,
  `:create_destination(body)`, `:update_destination(id, body)`, `:delete_destination(id)`,
  `:test_destination(id)`
- `c.packages:export(body?)` / `c.packages:import(body)` — Whole-instance package export/import
- `c.insights:summary(opts?)` — `opts`: `{startDate, endDate, projectId}`
- `c:discover(opts?)` — `opts`: `{include, resource, operation}`

### Module Helpers

Idempotent reconcilers. Each is safe to run repeatedly: it inspects current state first and writes
only what differs, so a script can be re-run without creating duplicates.

- `n8n.all(section, opts?)` → [item] — Follow `nextCursor` across every page. `section` is any
  client section exposing `:page` — `workflows`, `executions`, `credentials`, `tags`, `variables`,
  `projects`, `users`.
- `n8n.wait(url, opts?)` → true — Poll `/healthz`. `opts`: `{timeout = 60, interval = 2}` seconds.
- `n8n.find_workflow_by_name(client, name)` → workflow|nil — Exact-name lookup. The server-side
  `name` filter matches substrings, so the result is filtered again client-side.
- `n8n.set_active(client, id, active)` → workflow — Reconcile a workflow's active state. A workflow
  already in the requested state is returned without a write.
- `n8n.ensure_workflow(client, spec, opts?)` → workflow — Identity is `spec.name`. Updates the
  existing workflow of that name, otherwise creates it. `opts`: `{active}` also reconciles the
  active state. `spec` is sent as a full replacement body, so it should carry only writable fields
  (`name`, `nodes`, `connections`, `settings`, `staticData`).
- `n8n.ensure_tag(client, name)` → tag — Returns the tag with this name, creating it only if absent.
  n8n caps tag names at 24 characters and answers a longer one with a misleading
  `409 Tag already exists`, so treat a 409 here as a name-length problem first.
- `n8n.ensure_workflow_tags(client, workflow_id, names)` → [tag] — Creates any missing tags, then
  sets the workflow's tags to exactly this list. Tags not named are detached.
- `n8n.ensure_variable(client, key, value, opts?)` → variable — Identity is `key`. Updates only when
  the stored value differs.
- `n8n.ensure_project(client, name, opts?)` → project — Returns the project with this name, creating
  it only if absent.

Example:

```lua
local n8n = require("assay.n8n")
n8n.wait("http://n8n:5678")
local c = n8n.client("http://n8n:5678", { api_key = env.get("N8N_API_KEY") })

n8n.ensure_variable(c, "API_HOST", "https://api.example.com")

local wf = n8n.ensure_workflow(c, {
  name = "Nightly Sync",
  nodes = {
    {
      id = "trigger",
      name = "Schedule",
      type = "n8n-nodes-base.scheduleTrigger",
      typeVersion = 1.2,
      position = { 0, 0 },
      parameters = { rule = { interval = { { triggerAtHour = 2 } } } },
    },
  },
  connections = {},
  settings = { executionOrder = "v1" },
}, { active = true })

n8n.ensure_workflow_tags(c, wf.id, { "nightly", "owned-by-platform" })

for _, run in ipairs(c.executions:list({ workflowId = wf.id, status = "error", limit = 20 })) do
  log.warn("failed execution " .. tostring(run.id) .. " at " .. tostring(run.startedAt))
end
```

The API key must carry the scopes for what the script does — n8n answers `403 Forbidden` (not `401`)
when a valid key lacks a scope, so a read-only key fails every write.
