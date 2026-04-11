--- @module assay.grafana
--- @description Grafana monitoring and dashboards. Health, datasources, dashboards, annotations, alerts, folders, organization.
--- @keywords grafana, monitoring, dashboards, datasources, annotations, alerts, health, organization, folders, search, annotation
--- @quickref c.health:check() -> {database, version, commit} | Check Grafana health
--- @quickref c.datasources:list() -> [{id, name, type, url}] | List all datasources
--- @quickref c.datasources:get(id_or_uid) -> {id, name, type} | Get datasource by ID or UID
--- @quickref c.dashboards:search(opts?) -> [{id, title, type}] | Search dashboards/folders
--- @quickref c.dashboards:get(uid) -> {dashboard, meta} | Get dashboard by UID
--- @quickref c.annotations:list(opts?) -> [{id, text, time}] | List annotations
--- @quickref c.annotations:create(annotation) -> {id} | Create annotation
--- @quickref c.org:get() -> {id, name} | Get current organization
--- @quickref c.alerts:rules() -> [{uid, title}] | List alert rules
--- @quickref c.folders:list() -> [{id, uid, title}] | List folders

local M = {}

function M.client(url, opts)
  opts = opts or {}
  local base_url = url:gsub("/+$", "")
  local api_key = opts.api_key
  local username = opts.username
  local password = opts.password

  -- Shared HTTP helpers (captured by all sub-object methods as upvalues)

  local function headers()
    local h = { ["Content-Type"] = "application/json" }
    if api_key then
      h["Authorization"] = "Bearer " .. api_key
    elseif username and password then
      h["Authorization"] = "Basic " .. base64.encode(username .. ":" .. password)
    end
    return h
  end

  local function api_get(path_str)
    local resp = http.get(base_url .. path_str, { headers = headers() })
    if resp.status ~= 200 then
      error("grafana: GET " .. path_str .. " HTTP " .. resp.status .. ": " .. resp.body)
    end
    return json.parse(resp.body)
  end

  local function api_post(path_str, payload)
    local resp = http.post(base_url .. path_str, payload, { headers = headers() })
    if resp.status ~= 200 then
      error("grafana: POST " .. path_str .. " HTTP " .. resp.status .. ": " .. resp.body)
    end
    return json.parse(resp.body)
  end

  -- ===== Client =====

  local c = {}

  -- ===== Health =====

  c.health = {}

  function c.health:check()
    local resp = http.get(base_url .. "/api/health")
    if resp.status ~= 200 then
      error("grafana: GET /api/health HTTP " .. resp.status .. ": " .. resp.body)
    end
    return json.parse(resp.body)
  end

  -- ===== Datasources =====

  c.datasources = {}

  function c.datasources:list()
    return api_get("/api/datasources")
  end

  function c.datasources:get(id_or_uid)
    if type(id_or_uid) == "number" then
      return api_get("/api/datasources/" .. id_or_uid)
    end
    return api_get("/api/datasources/uid/" .. id_or_uid)
  end

  -- ===== Dashboards =====

  c.dashboards = {}

  function c.dashboards:search(opts)
    opts = opts or {}
    local params = {}
    if opts.query then params[#params + 1] = "query=" .. opts.query end
    if opts.type then params[#params + 1] = "type=" .. opts.type end
    if opts.tag then params[#params + 1] = "tag=" .. opts.tag end
    if opts.limit then params[#params + 1] = "limit=" .. opts.limit end
    local qs = ""
    if #params > 0 then qs = "?" .. table.concat(params, "&") end
    return api_get("/api/search" .. qs)
  end

  function c.dashboards:get(uid)
    return api_get("/api/dashboards/uid/" .. uid)
  end

  -- ===== Annotations =====

  c.annotations = {}

  function c.annotations:list(opts)
    opts = opts or {}
    local params = {}
    if opts.from then params[#params + 1] = "from=" .. opts.from end
    if opts.to then params[#params + 1] = "to=" .. opts.to end
    if opts.dashboard_id then params[#params + 1] = "dashboardId=" .. opts.dashboard_id end
    if opts.limit then params[#params + 1] = "limit=" .. opts.limit end
    if opts.tags then
      for _, tag in ipairs(opts.tags) do
        params[#params + 1] = "tags=" .. tag
      end
    end
    local qs = ""
    if #params > 0 then qs = "?" .. table.concat(params, "&") end
    return api_get("/api/annotations" .. qs)
  end

  function c.annotations:create(annotation)
    return api_post("/api/annotations", annotation)
  end

  -- ===== Organization =====

  c.org = {}

  function c.org:get()
    return api_get("/api/org")
  end

  -- ===== Alerts =====

  c.alerts = {}

  function c.alerts:rules()
    return api_get("/api/v1/provisioning/alert-rules")
  end

  -- ===== Folders =====

  c.folders = {}

  function c.folders:list()
    return api_get("/api/folders")
  end

  return c
end

return M
