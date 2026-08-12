--- @module assay.clickup
--- @description ClickUp REST API — tasks, lists, folders, spaces, goals, custom fields, time tracking, comments, and Docs. Covers sprint execution, quarterly goals, and KPI custom fields.
--- @keywords clickup, task, subtask, sprint, cycle, goal, okr, kpi, doc, page, list, folder, space, workspace, team, backlog, comment, time-tracking, custom-field, assignee, project-management, tracker
--- @quickref c.teams:list() -> [team] | Workspaces the token can see (v2 calls them teams)
--- @quickref c.spaces:list(team_id, opts?) -> [space] | Spaces in a workspace
--- @quickref c.spaces:get(space_id) -> space|nil | Get a space
--- @quickref c.folders:list(space_id, opts?) -> [folder] | Folders in a space
--- @quickref c.folders:get(folder_id) -> folder|nil | Get a folder
--- @quickref c.lists:list(folder_id, opts?) -> [list] | Lists in a folder
--- @quickref c.lists:folderless(space_id, opts?) -> [list] | Lists that sit directly in a space
--- @quickref c.lists:get(list_id) -> list|nil | Get a list
--- @quickref c.lists:create(folder_id, list) -> list | Create a list in a folder
--- @quickref c.tasks:page(list_id, opts?) -> {tasks, last_page} | One page of tasks (100 max)
--- @quickref c.tasks:list(list_id, opts?) -> [task] | First page of tasks in a list
--- @quickref c.tasks:get(task_id, opts?) -> task|nil | Get a task
--- @quickref c.tasks:create(list_id, task) -> task | Create a task
--- @quickref c.tasks:update(task_id, patch) -> task | Update a task
--- @quickref c.tasks:delete(task_id) -> true | Delete a task
--- @quickref c.tasks:filtered(team_id, opts?) -> {tasks, last_page} | Workspace-wide filtered task page
--- @quickref c.comments:list(task_id, opts?) -> [comment] | Comments on a task
--- @quickref c.comments:create(task_id, body) -> comment | Comment on a task
--- @quickref c.goals:list(team_id, opts?) -> [goal] | Goals in a workspace (quarterly targets)
--- @quickref c.goals:get(goal_id) -> goal|nil | Get a goal
--- @quickref c.goals:create(team_id, goal) -> goal | Create a goal
--- @quickref c.goals:update(goal_id, patch) -> goal | Update a goal
--- @quickref c.fields:list(list_id) -> [field] | Custom fields available on a list
--- @quickref c.fields:set(task_id, field_id, value) -> true | Set a custom field value on a task
--- @quickref c.time:entries(team_id, opts?) -> [entry] | Time entries in a date range
--- @quickref c.time:running(team_id, opts?) -> entry|nil | Currently running timer
--- @quickref c.docs:search(workspace_id, opts?) -> {docs, next_cursor} | Search Docs (API v3)
--- @quickref c.docs:pages(doc_id, opts?) -> [page] | Pages in a Doc (API v3)
--- @quickref c.docs:create_page(doc_id, page) -> page | Add a page to a Doc (API v3)
--- @quickref M.all_tasks(c, list_id, opts?) -> [task] | Walk every task page until last_page
--- @quickref M.find_task_by_name(c, list_id, name, opts?) -> task|nil | Exact-name task lookup in a list
--- @quickref M.ensure_task(c, list_id, spec, opts?) -> task | Create a task unless the name already exists
--- @quickref M.resolve_team(c, name?) -> team | The only workspace, or the one matching a name

local M = {}

local V2 = "/v2"
local V3 = "/v3"

function M.client(opts)
  opts = opts or {}
  local token = opts.token or env.get("CLICKUP_TOKEN")
  local base_url = (opts.base_url or "https://api.clickup.com/api"):gsub("/+$", "")

  -- Shared HTTP helpers (captured by all sub-object methods as upvalues)

  -- ClickUp takes the raw token in Authorization. A "Bearer " prefix is
  -- rejected for personal `pk_` tokens, unlike most APIs.
  local function headers()
    local h = { ["Content-Type"] = "application/json", ["Accept"] = "application/json" }
    if token then h["Authorization"] = token end
    return h
  end

  local function urlencode(str)
    return tostring(str):gsub("([^%w%-%.%_%~])", function(ch)
      return string.format("%%%02X", string.byte(ch))
    end)
  end

  -- List-valued filters (statuses, assignees, tags) repeat as `key[]=v`.
  local function build_query(params)
    if not params then return "" end
    local parts = {}
    for k, v in pairs(params) do
      if type(v) == "table" then
        for _, item in ipairs(v) do
          parts[#parts + 1] = urlencode(k) .. "%5B%5D=" .. urlencode(item)
        end
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

  local function api_get(version, path_str, query_params)
    local resp = http.get(base_url .. version .. path_str .. build_query(query_params),
      { headers = headers() })
    if resp.status == 404 then return nil end
    if resp.status ~= 200 then
      error("clickup: GET " .. path_str .. " HTTP " .. resp.status .. ": " .. (resp.body or ""))
    end
    return decode(resp)
  end

  local function api_send(method, fn, version, path_str, payload, query_params)
    local resp = fn(base_url .. version .. path_str .. build_query(query_params), payload or {},
      { headers = headers() })
    if resp.status ~= 200 and resp.status ~= 201 and resp.status ~= 204 then
      error("clickup: " .. method .. " " .. path_str .. " HTTP " .. resp.status .. ": " ..
        (resp.body or ""))
    end
    return decode(resp)
  end

  local function api_post(version, path_str, payload, query_params)
    return api_send("POST", http.post, version, path_str, payload, query_params)
  end

  local function api_put(version, path_str, payload, query_params)
    return api_send("PUT", http.put, version, path_str, payload, query_params)
  end

  local function api_delete(version, path_str, query_params)
    local resp = http.delete(base_url .. version .. path_str .. build_query(query_params),
      { headers = headers() })
    if resp.status ~= 200 and resp.status ~= 204 then
      error("clickup: DELETE " .. path_str .. " HTTP " .. resp.status .. ": " .. (resp.body or ""))
    end
    return true
  end

  local function items(envelope, key)
    if not envelope then return {} end
    return envelope[key] or {}
  end

  -- Goal endpoints wrap their single objects in a `goal` key. Every other v2
  -- resource returns the object at the top level.
  local function one(envelope, key)
    if not envelope then return nil end
    return envelope[key]
  end

  -- ===== Client =====

  local c = {}

  -- ===== Teams (workspaces) =====

  c.teams = {}

  function c.teams:list()
    return items(api_get(V2, "/team"), "teams")
  end

  -- ===== Spaces =====

  c.spaces = {}

  function c.spaces:list(team_id, query_opts)
    return items(api_get(V2, "/team/" .. urlencode(team_id) .. "/space", query_opts), "spaces")
  end

  function c.spaces:get(space_id)
    return api_get(V2, "/space/" .. urlencode(space_id))
  end

  function c.spaces:create(team_id, space)
    return api_post(V2, "/team/" .. urlencode(team_id) .. "/space", space)
  end

  function c.spaces:tags(space_id)
    return items(api_get(V2, "/space/" .. urlencode(space_id) .. "/tag"), "tags")
  end

  -- ===== Folders =====

  c.folders = {}

  function c.folders:list(space_id, query_opts)
    return items(api_get(V2, "/space/" .. urlencode(space_id) .. "/folder", query_opts), "folders")
  end

  function c.folders:get(folder_id)
    return api_get(V2, "/folder/" .. urlencode(folder_id))
  end

  function c.folders:create(space_id, folder)
    return api_post(V2, "/space/" .. urlencode(space_id) .. "/folder", folder)
  end

  function c.folders:delete(folder_id)
    return api_delete(V2, "/folder/" .. urlencode(folder_id))
  end

  -- ===== Lists =====

  c.lists = {}

  function c.lists:list(folder_id, query_opts)
    return items(api_get(V2, "/folder/" .. urlencode(folder_id) .. "/list", query_opts), "lists")
  end

  function c.lists:folderless(space_id, query_opts)
    return items(api_get(V2, "/space/" .. urlencode(space_id) .. "/list", query_opts), "lists")
  end

  function c.lists:get(list_id)
    return api_get(V2, "/list/" .. urlencode(list_id))
  end

  function c.lists:create(folder_id, list)
    return api_post(V2, "/folder/" .. urlencode(folder_id) .. "/list", list)
  end

  function c.lists:create_folderless(space_id, list)
    return api_post(V2, "/space/" .. urlencode(space_id) .. "/list", list)
  end

  function c.lists:update(list_id, patch)
    return api_put(V2, "/list/" .. urlencode(list_id), patch)
  end

  function c.lists:delete(list_id)
    return api_delete(V2, "/list/" .. urlencode(list_id))
  end

  function c.lists:members(list_id)
    return items(api_get(V2, "/list/" .. urlencode(list_id) .. "/member"), "members")
  end

  -- ===== Tasks =====

  c.tasks = {}

  function c.tasks:page(list_id, query_opts)
    return api_get(V2, "/list/" .. urlencode(list_id) .. "/task", query_opts)
      or { tasks = {}, last_page = true }
  end

  function c.tasks:list(list_id, query_opts)
    return items(self:page(list_id, query_opts), "tasks")
  end

  function c.tasks:get(task_id, query_opts)
    return api_get(V2, "/task/" .. urlencode(task_id), query_opts)
  end

  function c.tasks:create(list_id, task)
    return api_post(V2, "/list/" .. urlencode(list_id) .. "/task", task)
  end

  function c.tasks:update(task_id, patch)
    return api_put(V2, "/task/" .. urlencode(task_id), patch)
  end

  function c.tasks:delete(task_id)
    return api_delete(V2, "/task/" .. urlencode(task_id))
  end

  function c.tasks:filtered(team_id, query_opts)
    return api_get(V2, "/team/" .. urlencode(team_id) .. "/task", query_opts)
      or { tasks = {}, last_page = true }
  end

  function c.tasks:members(task_id)
    return items(api_get(V2, "/task/" .. urlencode(task_id) .. "/member"), "members")
  end

  -- ===== Comments =====

  c.comments = {}

  function c.comments:list(task_id, query_opts)
    return items(api_get(V2, "/task/" .. urlencode(task_id) .. "/comment", query_opts), "comments")
  end

  function c.comments:create(task_id, body)
    local payload = type(body) == "table" and body or { comment_text = tostring(body) }
    return api_post(V2, "/task/" .. urlencode(task_id) .. "/comment", payload)
  end

  function c.comments:update(comment_id, patch)
    return api_put(V2, "/comment/" .. urlencode(comment_id), patch)
  end

  function c.comments:delete(comment_id)
    return api_delete(V2, "/comment/" .. urlencode(comment_id))
  end

  -- ===== Goals =====

  c.goals = {}

  function c.goals:list(team_id, query_opts)
    return items(api_get(V2, "/team/" .. urlencode(team_id) .. "/goal", query_opts), "goals")
  end

  function c.goals:get(goal_id)
    return one(api_get(V2, "/goal/" .. urlencode(goal_id)), "goal")
  end

  function c.goals:create(team_id, goal)
    return one(api_post(V2, "/team/" .. urlencode(team_id) .. "/goal", goal), "goal")
  end

  function c.goals:update(goal_id, patch)
    return one(api_put(V2, "/goal/" .. urlencode(goal_id), patch), "goal")
  end

  function c.goals:delete(goal_id)
    return api_delete(V2, "/goal/" .. urlencode(goal_id))
  end

  -- ===== Custom fields =====

  c.fields = {}

  function c.fields:list(list_id)
    return items(api_get(V2, "/list/" .. urlencode(list_id) .. "/field"), "fields")
  end

  function c.fields:space(space_id)
    return items(api_get(V2, "/space/" .. urlencode(space_id) .. "/field"), "fields")
  end

  function c.fields:team(team_id)
    return items(api_get(V2, "/team/" .. urlencode(team_id) .. "/field"), "fields")
  end

  function c.fields:set(task_id, field_id, value, query_opts)
    api_put(V2, "/task/" .. urlencode(task_id) .. "/field/" .. urlencode(field_id),
      { value = value }, query_opts)
    return true
  end

  function c.fields:remove(task_id, field_id)
    return api_delete(V2, "/task/" .. urlencode(task_id) .. "/field/" .. urlencode(field_id))
  end

  -- ===== Time tracking =====

  c.time = {}

  function c.time:entries(team_id, query_opts)
    return items(api_get(V2, "/team/" .. urlencode(team_id) .. "/time_entries", query_opts), "data")
  end

  function c.time:running(team_id, query_opts)
    local envelope = api_get(V2, "/team/" .. urlencode(team_id) .. "/time_entries/running",
      query_opts)
    return envelope and envelope.data or nil
  end

  function c.time:create(team_id, entry)
    return api_post(V2, "/team/" .. urlencode(team_id) .. "/time_entries", entry)
  end

  function c.time:stop(team_id)
    return api_post(V2, "/team/" .. urlencode(team_id) .. "/time_entries/stop", {})
  end

  -- ===== Docs (API v3) =====

  -- Docs never shipped on v2; they are the one resource rooted at /v3, and v3
  -- names the workspace explicitly instead of calling it a team.
  c.docs = {}

  function c.docs:search(workspace_id, query_opts)
    return api_get(V3, "/workspaces/" .. urlencode(workspace_id) .. "/docs", query_opts)
      or { docs = {} }
  end

  function c.docs:get(workspace_id, doc_id)
    return api_get(V3, "/workspaces/" .. urlencode(workspace_id) .. "/docs/" .. urlencode(doc_id))
  end

  function c.docs:create(workspace_id, doc)
    return api_post(V3, "/workspaces/" .. urlencode(workspace_id) .. "/docs", doc)
  end

  function c.docs:pages(workspace_id, doc_id, query_opts)
    return api_get(V3, "/workspaces/" .. urlencode(workspace_id) .. "/docs/" ..
      urlencode(doc_id) .. "/pages", query_opts) or {}
  end

  function c.docs:create_page(workspace_id, doc_id, page)
    return api_post(V3, "/workspaces/" .. urlencode(workspace_id) .. "/docs/" ..
      urlencode(doc_id) .. "/pages", page)
  end

  function c.docs:edit_page(workspace_id, doc_id, page_id, patch)
    local path_str = "/workspaces/" .. urlencode(workspace_id) .. "/docs/" ..
      urlencode(doc_id) .. "/pages/" .. urlencode(page_id)
    return api_send("PATCH", http.patch, V3, path_str, patch)
  end

  function c.discover()
    return {
      base_url = base_url,
      authenticated = token ~= nil,
      sections = {
        "teams", "spaces", "folders", "lists", "tasks",
        "comments", "goals", "fields", "time", "docs",
      },
    }
  end

  return c
end

--- Walk every page of a task collection until the API reports `last_page`.
---
--- ClickUp paginates tasks by zero-based `page` and flags the final page with
--- `last_page = true`. Anything other than an explicit `false` ends the walk, so
--- a malformed envelope cannot spin forever; `max_pages` bounds it regardless.
function M.all_tasks(c, list_id, opts)
  opts = opts or {}
  local max_pages = opts.max_pages or 100
  local query = {}
  for k, v in pairs(opts) do
    if k ~= "max_pages" then query[k] = v end
  end

  local out = {}
  for page = 0, max_pages - 1 do
    query.page = page
    local envelope = c.tasks:page(list_id, query)
    for _, task in ipairs(envelope and envelope.tasks or {}) do
      out[#out + 1] = task
    end
    if not envelope or envelope.last_page ~= false then break end
  end
  return out
end

--- Exact-name task lookup within a list.
function M.find_task_by_name(c, list_id, name, opts)
  for _, task in ipairs(M.all_tasks(c, list_id, opts)) do
    if task.name == name then return task end
  end
  return nil
end

--- Create a task unless one in the list already carries that name.
---
--- Returns the existing task untouched when it is found, so repeated runs of the
--- same automation do not fan out duplicates.
function M.ensure_task(c, list_id, spec, opts)
  if type(spec) ~= "table" or not spec.name then
    error("clickup: ensure_task requires spec.name")
  end
  local existing = M.find_task_by_name(c, list_id, spec.name, opts)
  if existing then return existing end
  return c.tasks:create(list_id, spec)
end

--- Resolve the workspace to act on: the only one, or the one matching `name`.
function M.resolve_team(c, name)
  local teams = c.teams:list()
  if name then
    for _, team in ipairs(teams) do
      if team.name == name then return team end
    end
    error("clickup: no workspace named " .. tostring(name))
  end
  if #teams == 0 then error("clickup: token can see no workspaces") end
  if #teams > 1 then
    error("clickup: token sees " .. #teams .. " workspaces; pass a name to disambiguate")
  end
  return teams[1]
end

return M
