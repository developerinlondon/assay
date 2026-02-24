--- @module assay.eso
--- @description External Secrets Operator. ExternalSecrets, SecretStores, ClusterSecretStores sync status.
--- @keywords eso, external-secrets, secretstores, kubernetes, secrets, sync, store, readiness, wait, cluster, external-secret
--- @quickref c:external_secrets(namespace) -> {items} | List ExternalSecrets in namespace
--- @quickref c:external_secret(namespace, name) -> es|nil | Get ExternalSecret by name
--- @quickref c:external_secret_status(namespace, name) -> {ready, status, sync_hash} | Get sync status
--- @quickref c:is_secret_synced(namespace, name) -> bool | Check if ExternalSecret is synced
--- @quickref c:wait_secret_synced(namespace, name, timeout_secs?) -> true | Wait for sync
--- @quickref c:secret_stores(namespace) -> {items} | List SecretStores in namespace
--- @quickref c:secret_store(namespace, name) -> store|nil | Get SecretStore by name
--- @quickref c:secret_store_status(namespace, name) -> {ready, conditions} | Get store status
--- @quickref c:is_store_ready(namespace, name) -> bool | Check if SecretStore is ready
--- @quickref c:cluster_secret_stores() -> {items} | List ClusterSecretStores
--- @quickref c:cluster_secret_store(name) -> store|nil | Get ClusterSecretStore by name
--- @quickref c:is_cluster_store_ready(name) -> bool | Check if ClusterSecretStore is ready
--- @quickref c:cluster_external_secrets() -> {items} | List ClusterExternalSecrets
--- @quickref c:cluster_external_secret(name) -> es|nil | Get ClusterExternalSecret by name
--- @quickref c:all_secrets_synced(namespace) -> {synced, failed, total} | Check all secrets sync status
--- @quickref c:all_stores_ready(namespace) -> {ready, not_ready, total} | Check all stores readiness

local M = {}

local API_BASE = "/apis/external-secrets.io/v1beta1"

function M.client(url, token)
  local c = {
    url = url:gsub("/+$", ""),
    token = token,
  }

  local function headers(self)
    return { Authorization = "Bearer " .. self.token }
  end

  local function api_get(self, path)
    local resp = http.get(self.url .. path, { headers = headers(self) })
    if resp.status == 404 then return nil end
    if resp.status ~= 200 then
      error("eso: GET " .. path .. " HTTP " .. resp.status .. ": " .. resp.body)
    end
    return json.parse(resp.body)
  end

  local function api_list(self, path)
    local resp = http.get(self.url .. path, { headers = headers(self) })
    if resp.status ~= 200 then
      error("eso: LIST " .. path .. " HTTP " .. resp.status .. ": " .. resp.body)
    end
    return json.parse(resp.body)
  end

  local function find_condition(conditions, cond_type)
    for _, cond in ipairs(conditions or {}) do
      if cond.type == cond_type then
        return cond
      end
    end
    return nil
  end

  -- ExternalSecrets (namespaced)

  function c:external_secrets(namespace)
    return api_list(self, API_BASE .. "/namespaces/" .. namespace .. "/externalsecrets")
  end

  function c:external_secret(namespace, name)
    return api_get(self, API_BASE .. "/namespaces/" .. namespace .. "/externalsecrets/" .. name)
  end

  function c:external_secret_status(namespace, name)
    local es = api_get(self, API_BASE .. "/namespaces/" .. namespace .. "/externalsecrets/" .. name)
    if not es then
      error("eso: ExternalSecret " .. namespace .. "/" .. name .. " not found")
    end
    local status = es.status or {}
    local conditions = status.conditions or {}
    local ready_cond = find_condition(conditions, "Ready")
    return {
      ready = ready_cond ~= nil and ready_cond.status == "True",
      status = ready_cond and ready_cond.reason or "Unknown",
      sync_hash = status.syncedResourceVersion or "",
      conditions = conditions,
    }
  end

  function c:is_secret_synced(namespace, name)
    local es = api_get(self, API_BASE .. "/namespaces/" .. namespace .. "/externalsecrets/" .. name)
    if not es then return false end
    local conditions = (es.status or {}).conditions or {}
    local ready_cond = find_condition(conditions, "Ready")
    return ready_cond ~= nil and ready_cond.status == "True"
  end

  function c:wait_secret_synced(namespace, name, timeout_secs)
    timeout_secs = timeout_secs or 60
    local elapsed = 0
    while elapsed < timeout_secs do
      local synced = self:is_secret_synced(namespace, name)
      if synced then return true end
      sleep(5)
      elapsed = elapsed + 5
    end
    error("eso: ExternalSecret " .. namespace .. "/" .. name .. " not synced after " .. timeout_secs .. "s")
  end

  -- SecretStores (namespaced)

  function c:secret_stores(namespace)
    return api_list(self, API_BASE .. "/namespaces/" .. namespace .. "/secretstores")
  end

  function c:secret_store(namespace, name)
    return api_get(self, API_BASE .. "/namespaces/" .. namespace .. "/secretstores/" .. name)
  end

  function c:secret_store_status(namespace, name)
    local ss = api_get(self, API_BASE .. "/namespaces/" .. namespace .. "/secretstores/" .. name)
    if not ss then
      error("eso: SecretStore " .. namespace .. "/" .. name .. " not found")
    end
    local conditions = (ss.status or {}).conditions or {}
    local ready_cond = find_condition(conditions, "Ready")
    return {
      ready = ready_cond ~= nil and ready_cond.status == "True",
      conditions = conditions,
    }
  end

  function c:is_store_ready(namespace, name)
    local ss = api_get(self, API_BASE .. "/namespaces/" .. namespace .. "/secretstores/" .. name)
    if not ss then return false end
    local conditions = (ss.status or {}).conditions or {}
    local ready_cond = find_condition(conditions, "Ready")
    return ready_cond ~= nil and ready_cond.status == "True"
  end

  -- ClusterSecretStores (cluster-scoped)

  function c:cluster_secret_stores()
    return api_list(self, API_BASE .. "/clustersecretstores")
  end

  function c:cluster_secret_store(name)
    return api_get(self, API_BASE .. "/clustersecretstores/" .. name)
  end

  function c:is_cluster_store_ready(name)
    local css = api_get(self, API_BASE .. "/clustersecretstores/" .. name)
    if not css then return false end
    local conditions = (css.status or {}).conditions or {}
    local ready_cond = find_condition(conditions, "Ready")
    return ready_cond ~= nil and ready_cond.status == "True"
  end

  -- ClusterExternalSecrets (cluster-scoped)

  function c:cluster_external_secrets()
    return api_list(self, API_BASE .. "/clusterexternalsecrets")
  end

  function c:cluster_external_secret(name)
    return api_get(self, API_BASE .. "/clusterexternalsecrets/" .. name)
  end

  -- Utilities

  function c:all_secrets_synced(namespace)
    local list = self:external_secrets(namespace)
    local synced = 0
    local failed = 0
    local total = 0
    local failed_names = {}
    for _, es in ipairs(list.items or {}) do
      total = total + 1
      local conditions = (es.status or {}).conditions or {}
      local ready_cond = find_condition(conditions, "Ready")
      if ready_cond and ready_cond.status == "True" then
        synced = synced + 1
      else
        failed = failed + 1
        failed_names[#failed_names + 1] = es.metadata.name
      end
    end
    return { synced = synced, failed = failed, total = total, failed_names = failed_names }
  end

  function c:all_stores_ready(namespace)
    local list = self:secret_stores(namespace)
    local ready = 0
    local not_ready = 0
    local total = 0
    local not_ready_names = {}
    for _, ss in ipairs(list.items or {}) do
      total = total + 1
      local conditions = (ss.status or {}).conditions or {}
      local ready_cond = find_condition(conditions, "Ready")
      if ready_cond and ready_cond.status == "True" then
        ready = ready + 1
      else
        not_ready = not_ready + 1
        not_ready_names[#not_ready_names + 1] = ss.metadata.name
      end
    end
    return { ready = ready, not_ready = not_ready, total = total, not_ready_names = not_ready_names }
  end

  return c
end

return M
