--- @module assay.argocd
--- @description ArgoCD GitOps application management. Apps, sync, health, projects, repositories, clusters.
--- @keywords argocd, gitops, applications, sync, health, projects, repositories, clusters, rollback, manifest, resource-tree, refresh, wait, cicd, continuous-delivery
--- @quickref c:applications(opts?) -> [app] | List applications with optional project/selector filter
--- @quickref c:application(name) -> app | Get application by name
--- @quickref c:app_health(name) -> {status, sync, message} | Get app health and sync status
--- @quickref c:sync(name, opts?) -> result | Trigger application sync
--- @quickref c:refresh(name, opts?) -> app | Refresh application state
--- @quickref c:rollback(name, id) -> result | Rollback application to history ID
--- @quickref c:app_resources(name) -> resource_tree | Get application resource tree
--- @quickref c:app_manifests(name, opts?) -> manifests | Get application manifests
--- @quickref c:delete_app(name, opts?) -> nil | Delete application
--- @quickref c:projects() -> [project] | List projects
--- @quickref c:project(name) -> project | Get project by name
--- @quickref c:repositories() -> [repo] | List repositories
--- @quickref c:repository(repo_url) -> repo | Get repository by URL
--- @quickref c:clusters() -> [cluster] | List clusters
--- @quickref c:cluster(server_url) -> cluster | Get cluster by server URL
--- @quickref c:settings() -> settings | Get ArgoCD settings
--- @quickref c:version() -> version | Get ArgoCD version
--- @quickref c:is_healthy(name) -> bool | Check if app is healthy
--- @quickref c:is_synced(name) -> bool | Check if app is synced
--- @quickref c:wait_healthy(name, timeout_secs) -> true | Wait for app to become healthy
--- @quickref c:wait_synced(name, timeout_secs) -> true | Wait for app to become synced

local M = {}

function M.client(url, opts)
  opts = opts or {}
  local c = {
    url = url:gsub("/+$", ""),
    token = opts.token,
    username = opts.username,
    password = opts.password,
  }

  local function headers(self)
    local h = { ["Content-Type"] = "application/json" }
    if self.token then
      h["Authorization"] = "Bearer " .. self.token
    elseif self.username and self.password then
      h["Authorization"] = "Basic " .. base64.encode(self.username .. ":" .. self.password)
    end
    return h
  end

  local function api_get(self, path_str)
    local resp = http.get(self.url .. path_str, { headers = headers(self) })
    if resp.status ~= 200 then
      error("argocd: GET " .. path_str .. " HTTP " .. resp.status .. ": " .. resp.body)
    end
    return json.parse(resp.body)
  end

  local function api_post(self, path_str, payload)
    local resp = http.post(self.url .. path_str, payload, { headers = headers(self) })
    if resp.status ~= 200 then
      error("argocd: POST " .. path_str .. " HTTP " .. resp.status .. ": " .. resp.body)
    end
    return json.parse(resp.body)
  end

  local function api_put(self, path_str, payload)
    local resp = http.put(self.url .. path_str, payload, { headers = headers(self) })
    if resp.status ~= 200 then
      error("argocd: PUT " .. path_str .. " HTTP " .. resp.status .. ": " .. resp.body)
    end
    return json.parse(resp.body)
  end

  local function api_delete(self, path_str)
    local resp = http.delete(self.url .. path_str, { headers = headers(self) })
    if resp.status ~= 200 then
      error("argocd: DELETE " .. path_str .. " HTTP " .. resp.status .. ": " .. resp.body)
    end
    if resp.body and #resp.body > 0 then
      return json.parse(resp.body)
    end
    return nil
  end

  local function url_encode(s)
    return s:gsub("([^%w%-_.~])", function(c)
      return string.format("%%%02X", string.byte(c))
    end)
  end

  function c:applications(opts)
    opts = opts or {}
    local params = {}
    if opts.project then params[#params + 1] = "project=" .. opts.project end
    if opts.selector then params[#params + 1] = "selector=" .. opts.selector end
    local qs = ""
    if #params > 0 then qs = "?" .. table.concat(params, "&") end
    local data = api_get(self, "/api/v1/applications" .. qs)
    return data.items or {}
  end

  function c:application(name)
    return api_get(self, "/api/v1/applications/" .. name)
  end

  function c:app_health(name)
    local app = self:application(name)
    local health = app.status and app.status.health or {}
    local sync = app.status and app.status.sync or {}
    return {
      status = health.status,
      sync = sync.status,
      message = health.message,
    }
  end

  function c:sync(name, opts)
    opts = opts or {}
    local body = {}
    if opts.revision then body.revision = opts.revision end
    if opts.prune ~= nil then body.prune = opts.prune end
    if opts.dry_run ~= nil then body.dryRun = opts.dry_run end
    if opts.strategy then body.strategy = opts.strategy end
    return api_post(self, "/api/v1/applications/" .. name .. "/sync", body)
  end

  function c:refresh(name, opts)
    opts = opts or {}
    local refresh_type = opts.type or "normal"
    return api_get(self, "/api/v1/applications/" .. name .. "?refresh=" .. refresh_type)
  end

  function c:rollback(name, id)
    return api_put(self, "/api/v1/applications/" .. name .. "/rollback", { id = id })
  end

  function c:app_resources(name)
    return api_get(self, "/api/v1/applications/" .. name .. "/resource-tree")
  end

  function c:app_manifests(name, opts)
    opts = opts or {}
    local qs = ""
    if opts.revision then qs = "?revision=" .. opts.revision end
    return api_get(self, "/api/v1/applications/" .. name .. "/manifests" .. qs)
  end

  function c:delete_app(name, opts)
    opts = opts or {}
    local params = {}
    if opts.cascade ~= nil then params[#params + 1] = "cascade=" .. tostring(opts.cascade) end
    if opts.propagation_policy then params[#params + 1] = "propagationPolicy=" .. opts.propagation_policy end
    local qs = ""
    if #params > 0 then qs = "?" .. table.concat(params, "&") end
    return api_delete(self, "/api/v1/applications/" .. name .. qs)
  end

  function c:projects()
    local data = api_get(self, "/api/v1/projects")
    return data.items or {}
  end

  function c:project(name)
    return api_get(self, "/api/v1/projects/" .. name)
  end

  function c:repositories()
    local data = api_get(self, "/api/v1/repositories")
    return data.items or {}
  end

  function c:repository(repo_url)
    return api_get(self, "/api/v1/repositories/" .. url_encode(repo_url))
  end

  function c:clusters()
    local data = api_get(self, "/api/v1/clusters")
    return data.items or {}
  end

  function c:cluster(server_url)
    return api_get(self, "/api/v1/clusters/" .. url_encode(server_url))
  end

  function c:settings()
    return api_get(self, "/api/v1/settings")
  end

  function c:version()
    return api_get(self, "/api/version")
  end

  function c:is_healthy(name)
    local health = self:app_health(name)
    return health.status == "Healthy"
  end

  function c:is_synced(name)
    local health = self:app_health(name)
    return health.sync == "Synced"
  end

  function c:wait_healthy(name, timeout_secs)
    local deadline = time() + timeout_secs
    while time() < deadline do
      if self:is_healthy(name) then return true end
      sleep(2)
    end
    error("argocd: timeout waiting for " .. name .. " to become healthy")
  end

  function c:wait_synced(name, timeout_secs)
    local deadline = time() + timeout_secs
    while time() < deadline do
      if self:is_synced(name) then return true end
      sleep(2)
    end
    error("argocd: timeout waiting for " .. name .. " to become synced")
  end

  return c
end

return M
