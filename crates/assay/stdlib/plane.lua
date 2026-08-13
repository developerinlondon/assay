--- @module assay.plane
--- @description Plane REST API — projects, work items, cycles, modules, states, labels, members, comments, and links. Covers sprint execution on self-hosted or cloud Plane.
--- @keywords plane, work-item, issue, task, cycle, sprint, module, initiative, state, label, project, workspace, backlog, comment, link, intake, member, project-management, tracker, self-hosted
--- @quickref c.projects:list() -> [project] | Projects in the workspace
--- @quickref c.projects:get(project_id) -> project|nil | Get a project
--- @quickref c.projects:create(project) -> project | Create a project
--- @quickref c.projects:update(project_id, patch) -> project | Update a project
--- @quickref c.projects:delete(project_id) -> true | Delete a project
--- @quickref c.items:page(project_id, opts?) -> {items, next_cursor} | One page of work items
--- @quickref c.items:list(project_id, opts?) -> [item] | First page of work items
--- @quickref c.items:get(project_id, item_id) -> item|nil | Get a work item
--- @quickref c.items:create(project_id, item) -> item | Create a work item
--- @quickref c.items:update(project_id, item_id, patch) -> item | Update a work item
--- @quickref c.items:delete(project_id, item_id) -> true | Delete a work item
--- @quickref c.states:list(project_id) -> [state] | Workflow states in a project
--- @quickref c.states:create(project_id, state) -> state | Create a state
--- @quickref c.labels:list(project_id) -> [label] | Labels in a project
--- @quickref c.labels:create(project_id, label) -> label | Create a label
--- @quickref c.cycles:list(project_id) -> [cycle] | Cycles (sprints) in a project
--- @quickref c.cycles:create(project_id, cycle) -> cycle | Create a cycle
--- @quickref c.cycles:update(project_id, cycle_id, patch) -> cycle | Update a cycle
--- @quickref c.cycles:delete(project_id, cycle_id) -> true | Delete a cycle
--- @quickref c.cycles:add_items(project_id, cycle_id, item_ids) -> true | Put work items into a cycle
--- @quickref c.modules:list(project_id) -> [module] | Modules in a project
--- @quickref c.modules:create(project_id, mod) -> module | Create a module
--- @quickref c.members:list() -> [member] | Workspace members
--- @quickref c.comments:list(project_id, item_id) -> [comment] | Comments on a work item
--- @quickref c.comments:create(project_id, item_id, body) -> comment | Comment on a work item
--- @quickref c.links:list(project_id, item_id) -> [link] | Links attached to a work item
--- @quickref c.links:create(project_id, item_id, link) -> link | Attach a link to a work item
--- @quickref M.all_items(c, project_id, opts?) -> [item] | Walk every work-item page to the end
--- @quickref M.find_item_by_name(c, project_id, name, opts?) -> item|nil | Exact-name work-item lookup
--- @quickref M.ensure_item(c, project_id, spec, opts?) -> item | Create a work item unless the name exists
--- @quickref M.resolve_project(c, name?) -> project | The only project, or the one matching a name

local M = {}

local V1 = "/api/v1"

function M.client(opts)
  opts = opts or {}
  local api_key = opts.api_key or env.get("PLANE_API_KEY")
  local workspace = opts.workspace or env.get("PLANE_WORKSPACE")
  local base_url = (opts.base_url or env.get("PLANE_BASE_URL") or "https://api.plane.so"):gsub("/+$", "")

  -- Plane authenticates with its own header. There is no Authorization header
  -- and no Bearer prefix.
  local function headers()
    local h = { ["Content-Type"] = "application/json", ["Accept"] = "application/json" }
    if api_key then h["X-API-Key"] = api_key end
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
      if type(v) == "table" then
        parts[#parts + 1] = urlencode(k) .. "=" .. urlencode(table.concat(v, ","))
      elseif v ~= nil then
        parts[#parts + 1] = urlencode(k) .. "=" .. urlencode(v)
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

  local function ws_path(suffix)
    if not workspace or workspace == "" then
      error("plane: no workspace slug; pass workspace= or set PLANE_WORKSPACE")
    end
    return "/workspaces/" .. urlencode(workspace) .. suffix
  end

  local function proj_path(project_id, suffix)
    return ws_path("/projects/" .. urlencode(project_id) .. suffix)
  end

  local function api_get(path_str, query_params)
    local resp = http.get(base_url .. V1 .. path_str .. build_query(query_params),
      { headers = headers() })
    if resp.status == 404 then return nil end
    if resp.status ~= 200 then
      error("plane: GET " .. path_str .. " HTTP " .. resp.status .. ": " .. (resp.body or ""))
    end
    return decode(resp)
  end

  local function api_send(verb, fn, path_str, payload, query_params)
    local resp = fn(base_url .. V1 .. path_str .. build_query(query_params), payload or {},
      { headers = headers() })
    if resp.status ~= 200 and resp.status ~= 201 and resp.status ~= 204 then
      error("plane: " .. verb .. " " .. path_str .. " HTTP " .. resp.status .. ": " ..
        (resp.body or ""))
    end
    return decode(resp)
  end

  local function api_post(path_str, payload, query_params)
    return api_send("POST", http.post, path_str, payload, query_params)
  end

  local function api_patch(path_str, payload, query_params)
    return api_send("PATCH", http.patch, path_str, payload, query_params)
  end

  local function api_delete(path_str, query_params)
    local resp = http.delete(base_url .. V1 .. path_str .. build_query(query_params),
      { headers = headers() })
    if resp.status ~= 200 and resp.status ~= 204 then
      error("plane: DELETE " .. path_str .. " HTTP " .. resp.status .. ": " .. (resp.body or ""))
    end
    return true
  end

  -- Collection endpoints answer with a cursor envelope; a few of the smaller
  -- ones answer with a bare array instead, so accept both.
  local function rows(payload)
    if not payload then return {} end
    if payload.results ~= nil then return payload.results end
    return payload
  end

  local c = {}

  -- ===== Projects =====

  c.projects = {}

  function c.projects:list(query_opts)
    return rows(api_get(ws_path("/projects/"), query_opts))
  end

  function c.projects:get(project_id)
    return api_get(ws_path("/projects/" .. urlencode(project_id) .. "/"))
  end

  function c.projects:create(project)
    return api_post(ws_path("/projects/"), project)
  end

  function c.projects:update(project_id, patch)
    return api_patch(ws_path("/projects/" .. urlencode(project_id) .. "/"), patch)
  end

  function c.projects:delete(project_id)
    return api_delete(ws_path("/projects/" .. urlencode(project_id) .. "/"))
  end

  -- ===== Work items =====

  c.items = {}

  function c.items:page(project_id, query_opts)
    local payload = api_get(proj_path(project_id, "/work-items/"), query_opts)
    return {
      items = rows(payload),
      next_cursor = payload and payload.next_cursor or nil,
      has_more = payload and payload.next_page_results or false,
    }
  end

  function c.items:list(project_id, query_opts)
    return self:page(project_id, query_opts).items
  end

  function c.items:get(project_id, item_id)
    return api_get(proj_path(project_id, "/work-items/" .. urlencode(item_id) .. "/"))
  end

  function c.items:create(project_id, item)
    return api_post(proj_path(project_id, "/work-items/"), item)
  end

  function c.items:update(project_id, item_id, patch)
    return api_patch(proj_path(project_id, "/work-items/" .. urlencode(item_id) .. "/"), patch)
  end

  function c.items:delete(project_id, item_id)
    return api_delete(proj_path(project_id, "/work-items/" .. urlencode(item_id) .. "/"))
  end

  -- ===== States =====

  c.states = {}

  function c.states:list(project_id, query_opts)
    return rows(api_get(proj_path(project_id, "/states/"), query_opts))
  end

  function c.states:create(project_id, state)
    return api_post(proj_path(project_id, "/states/"), state)
  end

  -- ===== Labels =====

  c.labels = {}

  function c.labels:list(project_id, query_opts)
    return rows(api_get(proj_path(project_id, "/labels/"), query_opts))
  end

  function c.labels:create(project_id, label)
    return api_post(proj_path(project_id, "/labels/"), label)
  end

  -- ===== Cycles (sprints) =====

  c.cycles = {}

  function c.cycles:list(project_id, query_opts)
    return rows(api_get(proj_path(project_id, "/cycles/"), query_opts))
  end

  function c.cycles:get(project_id, cycle_id)
    return api_get(proj_path(project_id, "/cycles/" .. urlencode(cycle_id) .. "/"))
  end

  function c.cycles:create(project_id, cycle)
    return api_post(proj_path(project_id, "/cycles/"), cycle)
  end

  function c.cycles:update(project_id, cycle_id, patch)
    return api_patch(proj_path(project_id, "/cycles/" .. urlencode(cycle_id) .. "/"), patch)
  end

  function c.cycles:delete(project_id, cycle_id)
    return api_delete(proj_path(project_id, "/cycles/" .. urlencode(cycle_id) .. "/"))
  end

  function c.cycles:add_items(project_id, cycle_id, item_ids)
    api_post(proj_path(project_id, "/cycles/" .. urlencode(cycle_id) .. "/cycle-issues/"),
      { issues = item_ids })
    return true
  end

  -- ===== Modules =====

  c.modules = {}

  function c.modules:list(project_id, query_opts)
    return rows(api_get(proj_path(project_id, "/modules/"), query_opts))
  end

  function c.modules:get(project_id, module_id)
    return api_get(proj_path(project_id, "/modules/" .. urlencode(module_id) .. "/"))
  end

  function c.modules:create(project_id, mod)
    return api_post(proj_path(project_id, "/modules/"), mod)
  end

  function c.modules:update(project_id, module_id, patch)
    return api_patch(proj_path(project_id, "/modules/" .. urlencode(module_id) .. "/"), patch)
  end

  function c.modules:delete(project_id, module_id)
    return api_delete(proj_path(project_id, "/modules/" .. urlencode(module_id) .. "/"))
  end

  -- ===== Members =====

  c.members = {}

  function c.members:list(query_opts)
    return rows(api_get(ws_path("/members/"), query_opts))
  end

  -- ===== Comments and links =====
  --
  -- Plane serves work items under /work-items/ but roots their comments and
  -- links under /issues/, a leftover from the pre-rename API.

  c.comments = {}

  function c.comments:list(project_id, item_id, query_opts)
    return rows(api_get(proj_path(project_id, "/issues/" .. urlencode(item_id) .. "/comments/"),
      query_opts))
  end

  function c.comments:create(project_id, item_id, body)
    local payload = type(body) == "table" and body or { comment_html = "<p>" .. tostring(body) .. "</p>" }
    return api_post(proj_path(project_id, "/issues/" .. urlencode(item_id) .. "/comments/"), payload)
  end

  c.links = {}

  function c.links:list(project_id, item_id, query_opts)
    return rows(api_get(proj_path(project_id, "/issues/" .. urlencode(item_id) .. "/links/"),
      query_opts))
  end

  function c.links:create(project_id, item_id, link)
    return api_post(proj_path(project_id, "/issues/" .. urlencode(item_id) .. "/links/"), link)
  end

  -- ===== Intake =====

  c.intake = {}

  function c.intake:list(query_opts)
    return rows(api_get(ws_path("/intake-issues/"), query_opts))
  end

  c.workspace = workspace
  c.base_url = base_url
  c.authenticated = api_key ~= nil

  return c
end

-- ===== Module-level helpers =====

local MAX_PAGES = 50

--- Walk every work-item page until the cursor runs out.
function M.all_items(c, project_id, opts)
  opts = opts or {}
  local out = {}
  local cursor = nil
  for _ = 1, (opts.max_pages or MAX_PAGES) do
    local query = {}
    for k, v in pairs(opts.query or {}) do query[k] = v end
    if cursor then query.cursor = cursor end
    local page = c.items:page(project_id, query)
    for _, item in ipairs(page.items) do out[#out + 1] = item end
    if not page.has_more or not page.next_cursor then break end
    cursor = page.next_cursor
  end
  return out
end

--- Exact-name lookup. Plane has no name filter, so this walks the project.
function M.find_item_by_name(c, project_id, name, opts)
  for _, item in ipairs(M.all_items(c, project_id, opts)) do
    if item.name == name then return item end
  end
  return nil
end

--- Idempotent create: returns the existing work item when the name is taken.
function M.ensure_item(c, project_id, spec, opts)
  local found = M.find_item_by_name(c, project_id, spec.name, opts)
  if found then return found end
  return c.items:create(project_id, spec)
end

--- The only project, or the one whose name matches. Refuses to guess.
function M.resolve_project(c, name)
  local projects = c.projects:list()
  if name then
    for _, p in ipairs(projects) do
      if p.name == name then return p end
    end
    error("plane: no project named " .. tostring(name))
  end
  if #projects == 0 then error("plane: api key can see no projects") end
  if #projects > 1 then
    error("plane: api key sees " .. #projects .. " projects; pass a name to disambiguate")
  end
  return projects[1]
end

return M
