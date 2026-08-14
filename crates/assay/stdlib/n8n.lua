--- @module assay.n8n
--- @description n8n public REST API (/api/v1) — workflows, executions, credentials, tags, variables, projects, folders, users, data tables, source control, audit, settings.
--- @category saas
--- @icon n8n
--- @keywords n8n, workflow, automation, execution, credential, tag, variable, project, folder, node, trigger, webhook, source-control, audit, data-table, low-code
--- @quickref c.workflows:list(opts?) -> [workflow] | List workflows (first page)
--- @quickref c.workflows:page(opts?) -> {data, nextCursor} | One cursor page of workflows
--- @quickref c.workflows:get(id, opts?) -> workflow|nil | Get a workflow
--- @quickref c.workflows:create(wf) -> workflow | Create a workflow
--- @quickref c.workflows:update(id, wf) -> workflow | Replace a workflow
--- @quickref c.workflows:delete(id) -> workflow | Delete a workflow
--- @quickref c.workflows:activate(id) -> workflow | Activate a workflow
--- @quickref c.workflows:deactivate(id) -> workflow | Deactivate a workflow
--- @quickref c.workflows:archive(id) -> workflow | Archive a workflow
--- @quickref c.workflows:transfer(id, project_id) -> table | Move a workflow to a project
--- @quickref c.workflows:tags(id) -> [tag] | List a workflow's tags
--- @quickref c.workflows:set_tags(id, tag_ids) -> [tag] | Replace a workflow's tags
--- @quickref c.executions:list(opts?) -> [execution] | List executions (status, workflowId, limit)
--- @quickref c.executions:get(id, opts?) -> execution|nil | Get an execution
--- @quickref c.executions:delete(id) -> execution | Delete an execution
--- @quickref c.executions:retry(id, body?) -> table | Retry an execution
--- @quickref c.executions:stop(id) -> table | Stop a running execution
--- @quickref c.credentials:create(cred) -> credential | Create a credential
--- @quickref c.credentials:delete(id) -> credential | Delete a credential
--- @quickref c.credentials:schema(type_name) -> schema | Get the schema for a credential type
--- @quickref c.tags:list(opts?) -> [tag] | List tags
--- @quickref c.tags:create(tag) -> tag | Create a tag
--- @quickref c.variables:list(opts?) -> [variable] | List environment variables
--- @quickref c.projects:list(opts?) -> [project] | List projects
--- @quickref c.folders:list(project_id, opts?) -> [folder] | List folders in a project
--- @quickref c.users:list(opts?) -> [user] | List users
--- @quickref c.source_control:pull(body?) -> table | Pull from the configured git remote
--- @quickref c.audit:generate(body?) -> report | Generate a security audit report
--- @quickref M.all(section, opts?) -> [item] | Follow nextCursor across every page
--- @quickref M.wait(url, opts?) -> true | Wait for n8n to answer /healthz
--- @quickref M.ensure_workflow(c, spec, opts?) -> workflow | Create or update a workflow by name
--- @quickref M.ensure_tag(c, name) -> tag | Create a tag unless one already has that name
--- @quickref M.ensure_variable(c, key, value) -> variable | Create or update a variable by key
--- @quickref M.ensure_project(c, name, opts?) -> project | Create a project unless the name exists
--- @quickref M.set_active(c, id, active) -> workflow | Reconcile a workflow's active state
--- @quickref M.find_workflow_by_name(c, name) -> workflow|nil | Exact-name workflow lookup

local M = {}

--- An empty Lua table encodes as `{}`, which is right for `connections` and
--- `settings` but wrong for `nodes` — n8n rejects a workflow whose node list
--- arrived as a JSON object. Pin the empty cases before sending.
local function normalize_workflow(wf)
  if type(wf) ~= "table" then return wf end
  local out = {}
  for k, v in pairs(wf) do out[k] = v end
  if out.nodes == nil or next(out.nodes) == nil then
    out.nodes = json.array({})
  end
  if out.connections == nil then
    out.connections = json.object({})
  end
  if out.settings == nil then
    out.settings = json.object({})
  end
  return out
end

function M.client(url, opts)
  opts = opts or {}
  local base_url = url:gsub("/+$", "")
  local api_key = opts.api_key or env.get("N8N_API_KEY")
  local api_path = opts.api_path or "/api/v1"

  -- Shared HTTP helpers (captured by all sub-object methods as upvalues)

  local function headers()
    local h = { ["Content-Type"] = "application/json", ["Accept"] = "application/json" }
    if api_key then h["X-N8N-API-KEY"] = api_key end
    return h
  end

  local function urlencode(str)
    return tostring(str):gsub("([^%w%-%.%_%~])", function(ch)
      return string.format("%%%02X", string.byte(ch))
    end)
  end

  local function build_query(params)
    if not params then return "" end
    local parts = {}
    for k, v in pairs(params) do
      if v ~= nil then
        parts[#parts + 1] = urlencode(tostring(k)) .. "=" .. urlencode(tostring(v))
      end
    end
    table.sort(parts)
    return #parts > 0 and "?" .. table.concat(parts, "&") or ""
  end

  local function decode(resp)
    if resp.body and resp.body ~= "" then
      local ok, parsed = pcall(json.parse, resp.body)
      if ok then return parsed end
    end
    return nil
  end

  local function api_get(path_str, query_params)
    local resp = http.get(base_url .. api_path .. path_str .. build_query(query_params),
      { headers = headers() })
    if resp.status == 404 then return nil end
    if resp.status ~= 200 then
      error("n8n: GET " .. path_str .. " HTTP " .. resp.status .. ": " .. (resp.body or ""))
    end
    return decode(resp)
  end

  local function api_send(method, fn, path_str, payload, query_params)
    local resp = fn(base_url .. api_path .. path_str .. build_query(query_params), payload or {},
      { headers = headers() })
    if resp.status ~= 200 and resp.status ~= 201 and resp.status ~= 204 then
      error("n8n: " .. method .. " " .. path_str .. " HTTP " .. resp.status .. ": " ..
        (resp.body or ""))
    end
    return decode(resp)
  end

  local function api_post(path_str, payload, query_params)
    return api_send("POST", http.post, path_str, payload, query_params)
  end

  local function api_put(path_str, payload, query_params)
    return api_send("PUT", http.put, path_str, payload, query_params)
  end

  local function api_patch(path_str, payload, query_params)
    return api_send("PATCH", http.patch, path_str, payload, query_params)
  end

  local function api_delete(path_str, query_params)
    local resp = http.delete(base_url .. api_path .. path_str .. build_query(query_params),
      { headers = headers() })
    if resp.status ~= 200 and resp.status ~= 204 then
      error("n8n: DELETE " .. path_str .. " HTTP " .. resp.status .. ": " .. (resp.body or ""))
    end
    return decode(resp)
  end

  -- A cursor collection answers `{ data = [...], nextCursor = "..." }`.
  local function items(envelope)
    if not envelope then return {} end
    return envelope.data or {}
  end

  local function tag_refs(tag_ids)
    local refs = {}
    for _, id in ipairs(tag_ids or {}) do
      refs[#refs + 1] = type(id) == "table" and id or { id = tostring(id) }
    end
    return json.array(refs)
  end

  -- ===== Client =====

  local c = {}

  -- ===== Workflows =====

  c.workflows = {}

  function c.workflows:page(query_opts)
    return api_get("/workflows", query_opts) or { data = {} }
  end

  function c.workflows:list(query_opts)
    return items(self:page(query_opts))
  end

  function c.workflows:get(id, query_opts)
    return api_get("/workflows/" .. urlencode(id), query_opts)
  end

  function c.workflows:create(wf)
    return api_post("/workflows", normalize_workflow(wf))
  end

  function c.workflows:update(id, wf)
    return api_put("/workflows/" .. urlencode(id), normalize_workflow(wf))
  end

  function c.workflows:delete(id)
    return api_delete("/workflows/" .. urlencode(id))
  end

  function c.workflows:activate(id)
    return api_post("/workflows/" .. urlencode(id) .. "/activate")
  end

  function c.workflows:deactivate(id)
    return api_post("/workflows/" .. urlencode(id) .. "/deactivate")
  end

  function c.workflows:publish(id)
    return api_post("/workflows/" .. urlencode(id) .. "/publish")
  end

  function c.workflows:unpublish(id)
    return api_post("/workflows/" .. urlencode(id) .. "/unpublish")
  end

  function c.workflows:archive(id)
    return api_post("/workflows/" .. urlencode(id) .. "/archive")
  end

  function c.workflows:unarchive(id)
    return api_post("/workflows/" .. urlencode(id) .. "/unarchive")
  end

  function c.workflows:transfer(id, project_id)
    return api_put("/workflows/" .. urlencode(id) .. "/transfer",
      { destinationProjectId = project_id })
  end

  function c.workflows:tags(id)
    return api_get("/workflows/" .. urlencode(id) .. "/tags") or {}
  end

  function c.workflows:set_tags(id, tag_ids)
    return api_put("/workflows/" .. urlencode(id) .. "/tags", tag_refs(tag_ids))
  end

  function c.workflows:history(id, query_opts)
    return api_get("/workflows/" .. urlencode(id) .. "/history", query_opts)
  end

  function c.workflows:version(id, version_id)
    return api_get("/workflows/" .. urlencode(id) .. "/" .. urlencode(version_id))
  end

  -- ===== Test Runs =====

  c.test_runs = {}

  function c.test_runs:list(workflow_id, query_opts)
    return api_get("/workflows/" .. urlencode(workflow_id) .. "/test-runs", query_opts)
  end

  function c.test_runs:create(workflow_id, body)
    return api_post("/workflows/" .. urlencode(workflow_id) .. "/test-runs", body)
  end

  function c.test_runs:get(workflow_id, run_id)
    return api_get("/workflows/" .. urlencode(workflow_id) .. "/test-runs/" .. urlencode(run_id))
  end

  function c.test_runs:cancel(workflow_id, run_id)
    return api_post("/workflows/" .. urlencode(workflow_id) .. "/test-runs/" ..
      urlencode(run_id) .. "/cancel")
  end

  function c.test_runs:cases(workflow_id, run_id)
    return api_get("/workflows/" .. urlencode(workflow_id) .. "/test-runs/" ..
      urlencode(run_id) .. "/test-cases")
  end

  -- ===== Executions =====

  c.executions = {}

  function c.executions:page(query_opts)
    return api_get("/executions", query_opts) or { data = {} }
  end

  function c.executions:list(query_opts)
    return items(self:page(query_opts))
  end

  function c.executions:get(id, query_opts)
    return api_get("/executions/" .. urlencode(id), query_opts)
  end

  function c.executions:delete(id)
    return api_delete("/executions/" .. urlencode(id))
  end

  function c.executions:retry(id, body)
    return api_post("/executions/" .. urlencode(id) .. "/retry", body)
  end

  function c.executions:stop(id)
    return api_post("/executions/" .. urlencode(id) .. "/stop")
  end

  function c.executions:stop_all(body)
    return api_post("/executions/stop", body)
  end

  function c.executions:tags(id)
    return api_get("/executions/" .. urlencode(id) .. "/tags") or {}
  end

  function c.executions:set_tags(id, tag_ids)
    return api_put("/executions/" .. urlencode(id) .. "/tags", tag_refs(tag_ids))
  end

  -- ===== Credentials =====

  c.credentials = {}

  function c.credentials:page(query_opts)
    return api_get("/credentials", query_opts) or { data = {} }
  end

  function c.credentials:list(query_opts)
    return items(self:page(query_opts))
  end

  function c.credentials:get(id)
    return api_get("/credentials/" .. urlencode(id))
  end

  function c.credentials:create(cred)
    return api_post("/credentials", cred)
  end

  function c.credentials:update(id, cred)
    return api_patch("/credentials/" .. urlencode(id), cred)
  end

  function c.credentials:delete(id)
    return api_delete("/credentials/" .. urlencode(id))
  end

  function c.credentials:test(id, body)
    return api_post("/credentials/" .. urlencode(id) .. "/test", body)
  end

  function c.credentials:schema(type_name)
    return api_get("/credentials/schema/" .. urlencode(type_name))
  end

  function c.credentials:transfer(id, project_id)
    return api_put("/credentials/" .. urlencode(id) .. "/transfer",
      { destinationProjectId = project_id })
  end

  -- ===== Tags =====

  c.tags = {}

  function c.tags:page(query_opts)
    return api_get("/tags", query_opts) or { data = {} }
  end

  function c.tags:list(query_opts)
    return items(self:page(query_opts))
  end

  function c.tags:get(id)
    return api_get("/tags/" .. urlencode(id))
  end

  function c.tags:create(tag)
    return api_post("/tags", tag)
  end

  function c.tags:update(id, tag)
    return api_put("/tags/" .. urlencode(id), tag)
  end

  function c.tags:delete(id)
    return api_delete("/tags/" .. urlencode(id))
  end

  -- ===== Variables =====

  c.variables = {}

  function c.variables:page(query_opts)
    return api_get("/variables", query_opts) or { data = {} }
  end

  function c.variables:list(query_opts)
    return items(self:page(query_opts))
  end

  function c.variables:create(variable)
    return api_post("/variables", variable)
  end

  function c.variables:update(id, variable)
    return api_put("/variables/" .. urlencode(id), variable)
  end

  function c.variables:delete(id)
    return api_delete("/variables/" .. urlencode(id))
  end

  -- ===== Projects =====

  c.projects = {}

  function c.projects:page(query_opts)
    return api_get("/projects", query_opts) or { data = {} }
  end

  function c.projects:list(query_opts)
    return items(self:page(query_opts))
  end

  function c.projects:create(project)
    return api_post("/projects", project)
  end

  function c.projects:update(id, project)
    return api_put("/projects/" .. urlencode(id), project)
  end

  function c.projects:delete(id)
    return api_delete("/projects/" .. urlencode(id))
  end

  function c.projects:users(project_id)
    return api_get("/projects/" .. urlencode(project_id) .. "/users")
  end

  function c.projects:add_users(project_id, relations)
    return api_post("/projects/" .. urlencode(project_id) .. "/users", relations)
  end

  function c.projects:remove_user(project_id, user_id)
    return api_delete("/projects/" .. urlencode(project_id) .. "/users/" .. urlencode(user_id))
  end

  function c.projects:set_user_role(project_id, user_id, role)
    return api_patch("/projects/" .. urlencode(project_id) .. "/users/" .. urlencode(user_id),
      type(role) == "table" and role or { role = role })
  end

  -- ===== Folders =====

  c.folders = {}

  function c.folders:list(project_id, query_opts)
    local data = api_get("/projects/" .. urlencode(project_id) .. "/folders", query_opts)
    if not data then return {} end
    return data.data or data
  end

  function c.folders:get(project_id, folder_id)
    return api_get("/projects/" .. urlencode(project_id) .. "/folders/" .. urlencode(folder_id))
  end

  function c.folders:create(project_id, folder)
    return api_post("/projects/" .. urlencode(project_id) .. "/folders", folder)
  end

  function c.folders:update(project_id, folder_id, folder)
    return api_patch("/projects/" .. urlencode(project_id) .. "/folders/" ..
      urlencode(folder_id), folder)
  end

  function c.folders:delete(project_id, folder_id, query_opts)
    return api_delete("/projects/" .. urlencode(project_id) .. "/folders/" ..
      urlencode(folder_id), query_opts)
  end

  -- ===== Users =====

  c.users = {}

  function c.users:page(query_opts)
    return api_get("/users", query_opts) or { data = {} }
  end

  function c.users:list(query_opts)
    return items(self:page(query_opts))
  end

  function c.users:get(id_or_email)
    return api_get("/users/" .. urlencode(id_or_email))
  end

  function c.users:create(invitations)
    return api_post("/users", json.array(invitations))
  end

  function c.users:delete(id_or_email)
    return api_delete("/users/" .. urlencode(id_or_email))
  end

  function c.users:set_role(id_or_email, role)
    return api_patch("/users/" .. urlencode(id_or_email) .. "/role",
      type(role) == "table" and role or { newRoleName = role })
  end

  -- ===== Source Control =====

  c.source_control = {}

  function c.source_control:pull(body)
    return api_post("/source-control/pull", body)
  end

  -- ===== Audit =====

  c.audit = {}

  function c.audit:generate(body)
    return api_post("/audit", body)
  end

  -- ===== Data Tables =====

  c.data_tables = {}

  function c.data_tables:list(query_opts)
    return items(api_get("/data-tables", query_opts))
  end

  function c.data_tables:get(id)
    return api_get("/data-tables/" .. urlencode(id))
  end

  function c.data_tables:create(table_def)
    return api_post("/data-tables", table_def)
  end

  function c.data_tables:update(id, table_def)
    return api_patch("/data-tables/" .. urlencode(id), table_def)
  end

  function c.data_tables:delete(id)
    return api_delete("/data-tables/" .. urlencode(id))
  end

  function c.data_tables:rows(id, query_opts)
    return items(api_get("/data-tables/" .. urlencode(id) .. "/rows", query_opts))
  end

  function c.data_tables:insert_rows(id, body)
    return api_post("/data-tables/" .. urlencode(id) .. "/rows", body)
  end

  function c.data_tables:update_rows(id, body)
    return api_patch("/data-tables/" .. urlencode(id) .. "/rows/update", body)
  end

  function c.data_tables:upsert_rows(id, body)
    return api_post("/data-tables/" .. urlencode(id) .. "/rows/upsert", body)
  end

  function c.data_tables:clear_rows(id)
    return api_delete("/data-tables/" .. urlencode(id) .. "/rows/clear")
  end

  function c.data_tables:delete_rows(id, query_opts)
    return api_delete("/data-tables/" .. urlencode(id) .. "/rows/delete", query_opts)
  end

  function c.data_tables:columns(id)
    return items(api_get("/data-tables/" .. urlencode(id) .. "/columns"))
  end

  function c.data_tables:add_column(id, column)
    return api_post("/data-tables/" .. urlencode(id) .. "/columns", column)
  end

  function c.data_tables:update_column(id, column_id, column)
    return api_patch("/data-tables/" .. urlencode(id) .. "/columns/" ..
      urlencode(column_id), column)
  end

  function c.data_tables:delete_column(id, column_id)
    return api_delete("/data-tables/" .. urlencode(id) .. "/columns/" .. urlencode(column_id))
  end

  -- ===== Community Packages =====

  c.community_packages = {}

  function c.community_packages:list()
    return items(api_get("/community-packages"))
  end

  function c.community_packages:install(body)
    return api_post("/community-packages", body)
  end

  function c.community_packages:update(name, body)
    return api_patch("/community-packages/" .. urlencode(name), body)
  end

  function c.community_packages:uninstall(name)
    return api_delete("/community-packages/" .. urlencode(name))
  end

  -- ===== Settings =====

  c.settings = {}

  function c.settings:security_policy()
    return api_get("/settings/security-policy")
  end

  function c.settings:set_security_policy(body)
    return api_put("/settings/security-policy", body)
  end

  function c.settings:otel()
    return api_get("/settings/otel")
  end

  function c.settings:set_otel(body)
    return api_put("/settings/otel", body)
  end

  function c.settings:test_otel_trace(body)
    return api_post("/settings/otel/test-trace", body)
  end

  function c.settings:saml()
    return api_get("/settings/sso/saml")
  end

  function c.settings:set_saml(body)
    return api_put("/settings/sso/saml", body)
  end

  -- ===== Log Streaming =====

  c.log_streaming = {}

  function c.log_streaming:event_types()
    return api_get("/settings/log-streaming/event-types")
  end

  function c.log_streaming:destinations()
    return api_get("/settings/log-streaming/destinations")
  end

  function c.log_streaming:get_destination(id)
    return api_get("/settings/log-streaming/destinations/" .. urlencode(id))
  end

  function c.log_streaming:create_destination(body)
    return api_post("/settings/log-streaming/destinations", body)
  end

  function c.log_streaming:update_destination(id, body)
    return api_put("/settings/log-streaming/destinations/" .. urlencode(id), body)
  end

  function c.log_streaming:delete_destination(id)
    return api_delete("/settings/log-streaming/destinations/" .. urlencode(id))
  end

  function c.log_streaming:test_destination(id)
    return api_post("/settings/log-streaming/destinations/" .. urlencode(id) .. "/test")
  end

  -- ===== Packages (whole-instance export / import) =====

  c.packages = {}

  function c.packages:export(body)
    return api_post("/n8n-packages/export", body)
  end

  function c.packages:import(body)
    return api_post("/n8n-packages/import", body)
  end

  -- ===== Insights / Discover =====

  c.insights = {}

  function c.insights:summary(query_opts)
    return api_get("/insights/summary", query_opts)
  end

  function c:discover(query_opts)
    return api_get("/discover", query_opts)
  end

  return c
end

--- Follow `nextCursor` across every page of a cursor collection.
--- @param section table A client section exposing `:page(opts)` — `workflows`,
---   `executions`, `credentials`, `tags`, `variables`, `projects`, `users`.
--- @param opts table? Query options forwarded to each page request.
--- @return table items Every item across all pages, in server order.
function M.all(section, opts)
  local query = {}
  for k, v in pairs(opts or {}) do query[k] = v end
  local collected = {}
  repeat
    local page = section:page(query)
    for _, item in ipairs(page.data or {}) do
      collected[#collected + 1] = item
    end
    query.cursor = page.nextCursor
  until query.cursor == nil or query.cursor == ""
  return collected
end

--- Wait for an n8n instance to answer `/healthz`.
--- @param url string Base URL, without `/api/v1`.
--- @param opts table? `{ timeout = 60, interval = 2 }` (seconds).
--- @return true
function M.wait(url, opts)
  opts = opts or {}
  local timeout = opts.timeout or 60
  local interval = opts.interval or 2
  local max_attempts = math.ceil(timeout / interval)
  local base_url = url:gsub("/+$", "")

  for i = 1, max_attempts do
    local ok, resp = pcall(http.get, base_url .. "/healthz")
    if ok and resp.status == 200 then
      log.info("n8n healthy after " .. tostring(i * interval) .. "s")
      return true
    end
    if i == max_attempts then
      error("n8n.wait: not reachable at " .. base_url .. " after " .. tostring(timeout) .. "s")
    end
    log.info("Waiting for n8n... (" .. tostring(i) .. "/" .. tostring(max_attempts) .. ")")
    sleep(interval)
  end
end

--- Look up a workflow by exact name. The `name` list filter is a substring
--- match server-side, so the result is filtered again here.
--- @param client table Client from `M.client`.
--- @param name string Workflow name.
--- @return table|nil workflow The first exact-name match.
function M.find_workflow_by_name(client, name)
  for _, wf in ipairs(client.workflows:list({ name = name, limit = 250 })) do
    if wf.name == name then return wf end
  end
  return nil
end

--- Reconcile a workflow's active state. A workflow already in the requested
--- state is returned untouched, so this is safe to call on every run.
--- @param client table Client from `M.client`.
--- @param id string Workflow ID.
--- @param active boolean Desired state.
--- @return table workflow
function M.set_active(client, id, active)
  local wf = client.workflows:get(id)
  if not wf then
    error("n8n.set_active: workflow not found: " .. tostring(id))
  end
  if wf.active == active then return wf end
  if active then return client.workflows:activate(id) end
  return client.workflows:deactivate(id)
end

--- Create a workflow, or replace the existing one with the same name.
--- Identity is the workflow **name**, because the caller does not know the
--- server-assigned ID on a first run. `spec` is sent as the full replacement
--- body, so it should carry only the writable fields n8n accepts —
--- `name`, `nodes`, `connections`, `settings`, `staticData`.
--- @param client table Client from `M.client`.
--- @param spec table Workflow body, `spec.name` required.
--- @param opts table? `{ active = true|false }` to also reconcile active state.
--- @return table workflow
function M.ensure_workflow(client, spec, opts)
  opts = opts or {}
  assert.not_nil(spec, "n8n.ensure_workflow: spec is required")
  assert.not_nil(spec.name, "n8n.ensure_workflow: spec.name is required")

  local existing = M.find_workflow_by_name(client, spec.name)
  local wf
  if existing then
    wf = client.workflows:update(existing.id, spec)
    log.info("Updated workflow: " .. spec.name .. " (" .. tostring(existing.id) .. ")")
  else
    wf = client.workflows:create(spec)
    log.info("Created workflow: " .. spec.name .. " (" .. tostring(wf.id) .. ")")
  end

  if opts.active ~= nil then
    wf = M.set_active(client, wf.id, opts.active)
  end
  return wf
end

--- Return the tag with this name, creating it only if no tag has it.
--- n8n caps a tag name at 24 characters and reports a longer one as
--- `409 Tag already exists`, so a 409 here is as likely to be an over-long
--- name as a genuine race.
--- @param client table Client from `M.client`.
--- @param name string Tag name, 24 characters or fewer.
--- @return table tag
function M.ensure_tag(client, name)
  assert.not_nil(name, "n8n.ensure_tag: name is required")
  for _, tag in ipairs(M.all(client.tags)) do
    if tag.name == name then
      log.info("Tag already exists: " .. name)
      return tag
    end
  end
  local created = client.tags:create({ name = name })
  log.info("Created tag: " .. name)
  return created
end

--- Attach exactly this set of tag names to a workflow, creating any that are
--- missing. Tags not named here are detached — the list is the desired state.
--- @param client table Client from `M.client`.
--- @param workflow_id string Workflow ID.
--- @param names table Array of tag names.
--- @return table tags The workflow's tags after the call.
function M.ensure_workflow_tags(client, workflow_id, names)
  local ids = {}
  for _, name in ipairs(names or {}) do
    ids[#ids + 1] = M.ensure_tag(client, name).id
  end
  return client.workflows:set_tags(workflow_id, ids)
end

--- Create a variable, or update it when the stored value differs.
--- Identity is `key`.
--- @param client table Client from `M.client`.
--- @param key string Variable key.
--- @param value string Variable value.
--- @param opts table? Extra fields merged into the create/update body.
--- @return table variable
function M.ensure_variable(client, key, value, opts)
  assert.not_nil(key, "n8n.ensure_variable: key is required")
  local body = { key = key, value = value }
  for k, v in pairs(opts or {}) do body[k] = v end

  for _, variable in ipairs(M.all(client.variables)) do
    if variable.key == key then
      if variable.value == value then
        log.info("Variable already current: " .. key)
        return variable
      end
      local updated = client.variables:update(variable.id, body)
      log.info("Updated variable: " .. key)
      return updated or variable
    end
  end

  local created = client.variables:create(body)
  log.info("Created variable: " .. key)
  return created
end

--- Return the project with this name, creating it only if no project has it.
--- @param client table Client from `M.client`.
--- @param name string Project name.
--- @param opts table? Extra fields merged into the create body.
--- @return table project
function M.ensure_project(client, name, opts)
  assert.not_nil(name, "n8n.ensure_project: name is required")
  for _, project in ipairs(M.all(client.projects)) do
    if project.name == name then
      log.info("Project already exists: " .. name)
      return project
    end
  end

  local body = { name = name }
  for k, v in pairs(opts or {}) do body[k] = v end
  local created = client.projects:create(body)
  log.info("Created project: " .. name)
  return created
end

return M
