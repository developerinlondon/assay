--- @module assay.grafana
--- @description Grafana monitoring and dashboards. Health, datasources, annotations, alerts, folders.
--- @keywords grafana, monitoring, dashboards, datasources, annotations, alerts, health
--- @quickref c:health() -> {database, version, commit} | Check Grafana health
--- @quickref c:datasources() -> [{id, name, type, url}] | List all datasources
--- @quickref c:datasource(id_or_uid) -> {id, name, type} | Get datasource by ID or UID
--- @quickref c:search(opts?) -> [{id, title, type}] | Search dashboards/folders
--- @quickref c:dashboard(uid) -> {dashboard, meta} | Get dashboard by UID
--- @quickref c:annotations(opts?) -> [{id, text, time}] | List annotations
--- @quickref c:create_annotation(annotation) -> {id} | Create annotation
--- @quickref c:org() -> {id, name} | Get current organization
--- @quickref c:alert_rules() -> [{uid, title}] | List alert rules
--- @quickref c:folders() -> [{id, uid, title}] | List folders

local M = {}

function M.client(url, opts)
  opts = opts or {}
  local c = {
    url = url:gsub("/+$", ""),
    api_key = opts.api_key,
    username = opts.username,
    password = opts.password,
  }

  local function headers(self)
    local h = { ["Content-Type"] = "application/json" }
    if self.api_key then
      h["Authorization"] = "Bearer " .. self.api_key
    elseif self.username and self.password then
      h["Authorization"] = "Basic " .. base64.encode(self.username .. ":" .. self.password)
    end
    return h
  end

  local function api_get(self, path_str)
    local resp = http.get(self.url .. path_str, { headers = headers(self) })
    if resp.status ~= 200 then
      error("grafana: GET " .. path_str .. " HTTP " .. resp.status .. ": " .. resp.body)
    end
    return json.parse(resp.body)
  end

  local function api_post(self, path_str, payload)
    local resp = http.post(self.url .. path_str, payload, { headers = headers(self) })
    if resp.status ~= 200 then
      error("grafana: POST " .. path_str .. " HTTP " .. resp.status .. ": " .. resp.body)
    end
    return json.parse(resp.body)
  end

  function c:health()
    local resp = http.get(self.url .. "/api/health")
    if resp.status ~= 200 then
      error("grafana: GET /api/health HTTP " .. resp.status .. ": " .. resp.body)
    end
    return json.parse(resp.body)
  end

  function c:datasources()
    return api_get(self, "/api/datasources")
  end

  function c:datasource(id_or_uid)
    if type(id_or_uid) == "number" then
      return api_get(self, "/api/datasources/" .. id_or_uid)
    end
    return api_get(self, "/api/datasources/uid/" .. id_or_uid)
  end

  function c:search(opts)
    opts = opts or {}
    local params = {}
    if opts.query then params[#params + 1] = "query=" .. opts.query end
    if opts.type then params[#params + 1] = "type=" .. opts.type end
    if opts.tag then params[#params + 1] = "tag=" .. opts.tag end
    if opts.limit then params[#params + 1] = "limit=" .. opts.limit end
    local qs = ""
    if #params > 0 then qs = "?" .. table.concat(params, "&") end
    return api_get(self, "/api/search" .. qs)
  end

  function c:dashboard(uid)
    return api_get(self, "/api/dashboards/uid/" .. uid)
  end

  function c:annotations(opts)
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
    return api_get(self, "/api/annotations" .. qs)
  end

  function c:create_annotation(annotation)
    return api_post(self, "/api/annotations", annotation)
  end

  function c:org()
    return api_get(self, "/api/org")
  end

  function c:alert_rules()
    return api_get(self, "/api/v1/provisioning/alert-rules")
  end

  function c:folders()
    return api_get(self, "/api/folders")
  end

  return c
end

return M
