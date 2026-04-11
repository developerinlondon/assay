--- @module assay.flux
--- @description Flux CD GitOps toolkit. GitRepositories, Kustomizations, HelmReleases, notifications.
--- @keywords flux, gitops, kustomizations, helmreleases, gitrepositories, kubernetes, helm, oci, image-automation, notification, readiness, sources
--- @quickref c.git_repos:list(namespace) -> {items} | List GitRepositories
--- @quickref c.git_repos:get(namespace, name) -> repo|nil | Get GitRepository by name
--- @quickref c.git_repos:is_ready(namespace, name) -> bool | Check if GitRepository is ready
--- @quickref c.helm_repos:list(namespace) -> {items} | List HelmRepositories
--- @quickref c.helm_repos:get(namespace, name) -> repo|nil | Get HelmRepository by name
--- @quickref c.helm_repos:is_ready(namespace, name) -> bool | Check if HelmRepository is ready
--- @quickref c.helm_charts:list(namespace) -> {items} | List HelmCharts
--- @quickref c.oci_repos:list(namespace) -> {items} | List OCIRepositories
--- @quickref c.kustomizations:list(namespace) -> {items} | List Kustomizations
--- @quickref c.kustomizations:get(namespace, name) -> ks|nil | Get Kustomization by name
--- @quickref c.kustomizations:is_ready(namespace, name) -> bool | Check if Kustomization is ready
--- @quickref c.kustomizations:status(namespace, name) -> {ready, revision}|nil | Get Kustomization status
--- @quickref c.kustomizations:all_ready(namespace) -> {ready, not_ready, total} | Check all Kustomizations
--- @quickref c.helm_releases:list(namespace) -> {items} | List HelmReleases
--- @quickref c.helm_releases:get(namespace, name) -> hr|nil | Get HelmRelease by name
--- @quickref c.helm_releases:is_ready(namespace, name) -> bool | Check if HelmRelease is ready
--- @quickref c.helm_releases:status(namespace, name) -> {ready, revision}|nil | Get HelmRelease status
--- @quickref c.helm_releases:all_ready(namespace) -> {ready, not_ready, total} | Check all HelmReleases
--- @quickref c.notifications:alerts(namespace) -> {items} | List notification alerts
--- @quickref c.notifications:providers(namespace) -> {items} | List notification providers
--- @quickref c.notifications:receivers(namespace) -> {items} | List notification receivers
--- @quickref c.image_policies:list(namespace) -> {items} | List image policies
--- @quickref c.sources:all_ready(namespace) -> {ready, not_ready, total} | Check all sources readiness

local M = {}

-- Flux CD CRD API paths
local SOURCE_GROUP = "/apis/source.toolkit.fluxcd.io/v1"
local SOURCE_GROUP_V1BETA2 = "/apis/source.toolkit.fluxcd.io/v1beta2"
local KUSTOMIZE_GROUP = "/apis/kustomize.toolkit.fluxcd.io/v1"
local HELM_GROUP = "/apis/helm.toolkit.fluxcd.io/v2"
local NOTIFICATION_GROUP = "/apis/notification.toolkit.fluxcd.io/v1beta3"
local NOTIFICATION_GROUP_V1 = "/apis/notification.toolkit.fluxcd.io/v1"
local IMAGE_GROUP = "/apis/image.toolkit.fluxcd.io/v1beta2"

local function is_ready(resource)
  if not resource or not resource.status or not resource.status.conditions then
    return false
  end
  for i = 1, #resource.status.conditions do
    local cond = resource.status.conditions[i]
    if cond.type == "Ready" then
      return cond.status == "True"
    end
  end
  return false
end

function M.client(url, token)
  local base_url = url:gsub("/+$", "")

  -- Shared HTTP helpers (captured by all sub-object methods as upvalues)

  local function headers()
    return { ["Authorization"] = "Bearer " .. token }
  end

  local function api_get(path)
    local resp = http.get(base_url .. path, { headers = headers() })
    if resp.status == 404 then return nil end
    if resp.status ~= 200 then
      error("flux: GET " .. path .. " HTTP " .. resp.status .. ": " .. resp.body)
    end
    return json.parse(resp.body)
  end

  local function api_list(path)
    local resp = http.get(base_url .. path, { headers = headers() })
    if resp.status ~= 200 then
      error("flux: LIST " .. path .. " HTTP " .. resp.status .. ": " .. resp.body)
    end
    return json.parse(resp.body)
  end

  -- ===== Client =====

  local c = {}

  -- ===== Git Repositories =====

  c.git_repos = {}

  function c.git_repos:list(namespace)
    return api_list(SOURCE_GROUP .. "/namespaces/" .. namespace .. "/gitrepositories")
  end

  function c.git_repos:get(namespace, name)
    return api_get(SOURCE_GROUP .. "/namespaces/" .. namespace .. "/gitrepositories/" .. name)
  end

  function c.git_repos:is_ready(namespace, name)
    local resource = c.git_repos:get(namespace, name)
    return is_ready(resource)
  end

  -- ===== Helm Repositories =====

  c.helm_repos = {}

  function c.helm_repos:list(namespace)
    return api_list(SOURCE_GROUP .. "/namespaces/" .. namespace .. "/helmrepositories")
  end

  function c.helm_repos:get(namespace, name)
    return api_get(SOURCE_GROUP .. "/namespaces/" .. namespace .. "/helmrepositories/" .. name)
  end

  function c.helm_repos:is_ready(namespace, name)
    local resource = c.helm_repos:get(namespace, name)
    return is_ready(resource)
  end

  -- ===== Helm Charts =====

  c.helm_charts = {}

  function c.helm_charts:list(namespace)
    return api_list(SOURCE_GROUP .. "/namespaces/" .. namespace .. "/helmcharts")
  end

  -- ===== OCI Repositories =====

  c.oci_repos = {}

  function c.oci_repos:list(namespace)
    return api_list(SOURCE_GROUP_V1BETA2 .. "/namespaces/" .. namespace .. "/ocirepositories")
  end

  -- ===== Kustomizations =====

  c.kustomizations = {}

  function c.kustomizations:list(namespace)
    return api_list(KUSTOMIZE_GROUP .. "/namespaces/" .. namespace .. "/kustomizations")
  end

  function c.kustomizations:get(namespace, name)
    return api_get(KUSTOMIZE_GROUP .. "/namespaces/" .. namespace .. "/kustomizations/" .. name)
  end

  function c.kustomizations:is_ready(namespace, name)
    local resource = c.kustomizations:get(namespace, name)
    return is_ready(resource)
  end

  function c.kustomizations:status(namespace, name)
    local resource = c.kustomizations:get(namespace, name)
    if not resource then return nil end
    local st = resource.status or {}
    return {
      ready = is_ready(resource),
      revision = (st.lastAttemptedRevision or ""),
      last_applied_revision = (st.lastAppliedRevision or ""),
      conditions = st.conditions or {},
    }
  end

  function c.kustomizations:all_ready(namespace)
    local result = { ready = 0, not_ready = 0, total = 0, not_ready_names = {} }

    local ks_list = c.kustomizations:list(namespace)
    for i = 1, #ks_list.items do
      result.total = result.total + 1
      if is_ready(ks_list.items[i]) then
        result.ready = result.ready + 1
      else
        result.not_ready = result.not_ready + 1
        result.not_ready_names[#result.not_ready_names + 1] = ks_list.items[i].metadata.name
      end
    end

    return result
  end

  -- ===== Helm Releases =====

  c.helm_releases = {}

  function c.helm_releases:list(namespace)
    return api_list(HELM_GROUP .. "/namespaces/" .. namespace .. "/helmreleases")
  end

  function c.helm_releases:get(namespace, name)
    return api_get(HELM_GROUP .. "/namespaces/" .. namespace .. "/helmreleases/" .. name)
  end

  function c.helm_releases:is_ready(namespace, name)
    local resource = c.helm_releases:get(namespace, name)
    return is_ready(resource)
  end

  function c.helm_releases:status(namespace, name)
    local resource = c.helm_releases:get(namespace, name)
    if not resource then return nil end
    local st = resource.status or {}
    return {
      ready = is_ready(resource),
      revision = (st.lastAttemptedRevision or ""),
      last_applied_revision = (st.lastAppliedRevision or ""),
      conditions = st.conditions or {},
    }
  end

  function c.helm_releases:all_ready(namespace)
    local result = { ready = 0, not_ready = 0, total = 0, not_ready_names = {} }

    local hr_list = c.helm_releases:list(namespace)
    for i = 1, #hr_list.items do
      result.total = result.total + 1
      if is_ready(hr_list.items[i]) then
        result.ready = result.ready + 1
      else
        result.not_ready = result.not_ready + 1
        result.not_ready_names[#result.not_ready_names + 1] = hr_list.items[i].metadata.name
      end
    end

    return result
  end

  -- ===== Notifications =====

  c.notifications = {}

  function c.notifications:alerts(namespace)
    return api_list(NOTIFICATION_GROUP .. "/namespaces/" .. namespace .. "/alerts")
  end

  function c.notifications:providers(namespace)
    return api_list(NOTIFICATION_GROUP .. "/namespaces/" .. namespace .. "/providers")
  end

  function c.notifications:receivers(namespace)
    return api_list(NOTIFICATION_GROUP_V1 .. "/namespaces/" .. namespace .. "/receivers")
  end

  -- ===== Image Policies =====

  c.image_policies = {}

  function c.image_policies:list(namespace)
    return api_list(IMAGE_GROUP .. "/namespaces/" .. namespace .. "/imagepolicies")
  end

  -- ===== Sources (aggregate) =====

  c.sources = {}

  function c.sources:all_ready(namespace)
    local result = { ready = 0, not_ready = 0, total = 0, not_ready_names = {} }

    local git_repos = c.git_repos:list(namespace)
    for i = 1, #git_repos.items do
      result.total = result.total + 1
      if is_ready(git_repos.items[i]) then
        result.ready = result.ready + 1
      else
        result.not_ready = result.not_ready + 1
        result.not_ready_names[#result.not_ready_names + 1] = git_repos.items[i].metadata.name
      end
    end

    local helm_repos_list = c.helm_repos:list(namespace)
    for i = 1, #helm_repos_list.items do
      result.total = result.total + 1
      if is_ready(helm_repos_list.items[i]) then
        result.ready = result.ready + 1
      else
        result.not_ready = result.not_ready + 1
        result.not_ready_names[#result.not_ready_names + 1] = helm_repos_list.items[i].metadata.name
      end
    end

    return result
  end

  return c
end

return M
