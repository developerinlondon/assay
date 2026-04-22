--- @module assay.argocd
--- @description ArgoCD GitOps application management. Apps, sync, health, projects, repositories, clusters.
--- @keywords argocd, gitops, applications, sync, health, projects, repositories, clusters, rollback, manifest, resource-tree, refresh, wait, cicd, continuous-delivery
--- @quickref c.apps:list(opts?) -> [app] | List applications with optional project/selector filter
--- @quickref c.apps:get(name) -> app | Get application by name
--- @quickref c.apps:health(name) -> {status, sync, message} | Get app health and sync status
--- @quickref c.apps:sync(name, opts?) -> result | Trigger application sync
--- @quickref c.apps:refresh(name, opts?) -> app | Refresh application state
--- @quickref c.apps:rollback(name, id) -> result | Rollback application to history ID
--- @quickref c.apps:resources(name) -> resource_tree | Get application resource tree
--- @quickref c.apps:manifests(name, opts?) -> manifests | Get application manifests
--- @quickref c.apps:delete(name, opts?) -> nil | Delete application
--- @quickref c.apps:is_healthy(name) -> bool | Check if app is healthy
--- @quickref c.apps:is_synced(name) -> bool | Check if app is synced
--- @quickref c.apps:wait_healthy(name, timeout_secs) -> true | Wait for app to become healthy
--- @quickref c.apps:wait_synced(name, timeout_secs) -> true | Wait for app to become synced
--- @quickref c.projects:list() -> [project] | List projects
--- @quickref c.projects:get(name) -> project | Get project by name
--- @quickref c.repositories:list() -> [repo] | List repositories
--- @quickref c.repositories:get(repo_url) -> repo | Get repository by URL
--- @quickref c.clusters:list() -> [cluster] | List clusters
--- @quickref c.clusters:get(server_url) -> cluster | Get cluster by server URL
--- @quickref c.settings:get() -> settings | Get ArgoCD settings
--- @quickref c:version() -> version | Get ArgoCD version

local M = {}

function M.client(url, opts)
  opts = opts or {}
  local base_url = url:gsub("/+$", "")
  local token = opts.token
  local username = opts.username
  local password = opts.password

  -- Shared HTTP helpers (captured by all sub-object methods as upvalues)

  local function headers()
    local h = { ["Content-Type"] = "application/json" }
    if token then
      h["Authorization"] = "Bearer " .. token
    elseif username and password then
      h["Authorization"] = "Basic " .. base64.encode(username .. ":" .. password)
    end
    return h
  end

  local function api_get(path_str)
    local resp = http.get(base_url .. path_str, { headers = headers() })
    if resp.status ~= 200 then
      error("argocd: GET " .. path_str .. " HTTP " .. resp.status .. ": " .. resp.body)
    end
    return json.parse(resp.body)
  end

  local function api_post(path_str, payload)
    local resp = http.post(base_url .. path_str, payload, { headers = headers() })
    if resp.status ~= 200 then
      error("argocd: POST " .. path_str .. " HTTP " .. resp.status .. ": " .. resp.body)
    end
    return json.parse(resp.body)
  end

  local function api_put(path_str, payload)
    local resp = http.put(base_url .. path_str, payload, { headers = headers() })
    if resp.status ~= 200 then
      error("argocd: PUT " .. path_str .. " HTTP " .. resp.status .. ": " .. resp.body)
    end
    return json.parse(resp.body)
  end

  local function api_delete(path_str)
    local resp = http.delete(base_url .. path_str, { headers = headers() })
    if resp.status ~= 200 then
      error("argocd: DELETE " .. path_str .. " HTTP " .. resp.status .. ": " .. resp.body)
    end
    if resp.body and #resp.body > 0 then
      return json.parse(resp.body)
    end
    return nil
  end

  local function url_encode(s)
    return s:gsub("([^%w%-_.~])", function(ch)
      return string.format("%%%02X", string.byte(ch))
    end)
  end

  -- ===== Client =====

  local c = {}

  -- ===== Applications =====

  c.apps = {}

  function c.apps:list(app_opts)
    app_opts = app_opts or {}
    local params = {}
    if app_opts.project then params[#params + 1] = "project=" .. app_opts.project end
    if app_opts.selector then params[#params + 1] = "selector=" .. app_opts.selector end
    local qs = ""
    if #params > 0 then qs = "?" .. table.concat(params, "&") end
    local data = api_get("/api/v1/applications" .. qs)
    return data.items or {}
  end

  function c.apps:get(name)
    return api_get("/api/v1/applications/" .. name)
  end

  function c.apps:health(name)
    local app = c.apps:get(name)
    local health = app.status and app.status.health or {}
    local sync = app.status and app.status.sync or {}
    return {
      status = health.status,
      sync = sync.status,
      message = health.message,
    }
  end

  function c.apps:sync(name, sync_opts)
    sync_opts = sync_opts or {}
    local body = {}
    if sync_opts.revision then body.revision = sync_opts.revision end
    if sync_opts.prune ~= nil then body.prune = sync_opts.prune end
    if sync_opts.dry_run ~= nil then body.dryRun = sync_opts.dry_run end
    if sync_opts.strategy then body.strategy = sync_opts.strategy end
    return api_post("/api/v1/applications/" .. name .. "/sync", body)
  end

  function c.apps:refresh(name, refresh_opts)
    refresh_opts = refresh_opts or {}
    local refresh_type = refresh_opts.type or "normal"
    return api_get("/api/v1/applications/" .. name .. "?refresh=" .. refresh_type)
  end

  function c.apps:rollback(name, id)
    return api_put("/api/v1/applications/" .. name .. "/rollback", { id = id })
  end

  function c.apps:resources(name)
    return api_get("/api/v1/applications/" .. name .. "/resource-tree")
  end

  function c.apps:manifests(name, manifest_opts)
    manifest_opts = manifest_opts or {}
    local qs = ""
    if manifest_opts.revision then qs = "?revision=" .. manifest_opts.revision end
    return api_get("/api/v1/applications/" .. name .. "/manifests" .. qs)
  end

  function c.apps:delete(name, delete_opts)
    delete_opts = delete_opts or {}
    local params = {}
    if delete_opts.cascade ~= nil then params[#params + 1] = "cascade=" .. tostring(delete_opts.cascade) end
    if delete_opts.propagation_policy then params[#params + 1] = "propagationPolicy=" .. delete_opts.propagation_policy end
    local qs = ""
    if #params > 0 then qs = "?" .. table.concat(params, "&") end
    return api_delete("/api/v1/applications/" .. name .. qs)
  end

  function c.apps:is_healthy(name)
    local health = c.apps:health(name)
    return health.status == "Healthy"
  end

  function c.apps:is_synced(name)
    local health = c.apps:health(name)
    return health.sync == "Synced"
  end

  function c.apps:wait_healthy(name, timeout_secs)
    local deadline = time() + timeout_secs
    while time() < deadline do
      if c.apps:is_healthy(name) then return true end
      sleep(2)
    end
    error("argocd: timeout waiting for " .. name .. " to become healthy")
  end

  function c.apps:wait_synced(name, timeout_secs)
    local deadline = time() + timeout_secs
    while time() < deadline do
      if c.apps:is_synced(name) then return true end
      sleep(2)
    end
    error("argocd: timeout waiting for " .. name .. " to become synced")
  end

  -- ===== Projects =====

  c.projects = {}

  function c.projects:list()
    local data = api_get("/api/v1/projects")
    return data.items or {}
  end

  function c.projects:get(name)
    return api_get("/api/v1/projects/" .. name)
  end

  -- ===== Repositories =====

  c.repositories = {}

  function c.repositories:list()
    local data = api_get("/api/v1/repositories")
    return data.items or {}
  end

  function c.repositories:get(repo_url)
    return api_get("/api/v1/repositories/" .. url_encode(repo_url))
  end

  -- ===== Clusters =====

  c.clusters = {}

  function c.clusters:list()
    local data = api_get("/api/v1/clusters")
    return data.items or {}
  end

  function c.clusters:get(server_url)
    return api_get("/api/v1/clusters/" .. url_encode(server_url))
  end

  -- ===== Settings =====

  c.settings = {}

  function c.settings:get()
    return api_get("/api/v1/settings")
  end

  -- ===== Version (top-level, not resource-scoped) =====

  function c:version()
    return api_get("/api/version")
  end

  return c
end

return M
