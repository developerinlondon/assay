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
  local c = {
    url = url:gsub("/+$", ""),
    token = token,
  }

  local function headers(self)
    return { ["Authorization"] = "Bearer " .. self.token }
  end

  local function api_get(self, path)
    local resp = http.get(self.url .. path, { headers = headers(self) })
    if resp.status == 404 then return nil end
    if resp.status ~= 200 then
      error("flux: GET " .. path .. " HTTP " .. resp.status .. ": " .. resp.body)
    end
    return json.parse(resp.body)
  end

  local function api_list(self, path)
    local resp = http.get(self.url .. path, { headers = headers(self) })
    if resp.status ~= 200 then
      error("flux: LIST " .. path .. " HTTP " .. resp.status .. ": " .. resp.body)
    end
    return json.parse(resp.body)
  end

  -- Sources: GitRepositories

  function c:git_repositories(namespace)
    return api_list(self, SOURCE_GROUP .. "/namespaces/" .. namespace .. "/gitrepositories")
  end

  function c:git_repository(namespace, name)
    return api_get(self, SOURCE_GROUP .. "/namespaces/" .. namespace .. "/gitrepositories/" .. name)
  end

  function c:is_git_repo_ready(namespace, name)
    local resource = self:git_repository(namespace, name)
    return is_ready(resource)
  end

  -- Sources: HelmRepositories

  function c:helm_repositories(namespace)
    return api_list(self, SOURCE_GROUP .. "/namespaces/" .. namespace .. "/helmrepositories")
  end

  function c:helm_repository(namespace, name)
    return api_get(self, SOURCE_GROUP .. "/namespaces/" .. namespace .. "/helmrepositories/" .. name)
  end

  function c:is_helm_repo_ready(namespace, name)
    local resource = self:helm_repository(namespace, name)
    return is_ready(resource)
  end

  -- Sources: HelmCharts

  function c:helm_charts(namespace)
    return api_list(self, SOURCE_GROUP .. "/namespaces/" .. namespace .. "/helmcharts")
  end

  -- Sources: OCIRepositories

  function c:oci_repositories(namespace)
    return api_list(self, SOURCE_GROUP_V1BETA2 .. "/namespaces/" .. namespace .. "/ocirepositories")
  end

  -- Kustomizations

  function c:kustomizations(namespace)
    return api_list(self, KUSTOMIZE_GROUP .. "/namespaces/" .. namespace .. "/kustomizations")
  end

  function c:kustomization(namespace, name)
    return api_get(self, KUSTOMIZE_GROUP .. "/namespaces/" .. namespace .. "/kustomizations/" .. name)
  end

  function c:is_kustomization_ready(namespace, name)
    local resource = self:kustomization(namespace, name)
    return is_ready(resource)
  end

  function c:kustomization_status(namespace, name)
    local resource = self:kustomization(namespace, name)
    if not resource then return nil end
    local status = resource.status or {}
    return {
      ready = is_ready(resource),
      revision = (status.lastAttemptedRevision or ""),
      last_applied_revision = (status.lastAppliedRevision or ""),
      conditions = status.conditions or {},
    }
  end

  -- Helm Releases

  function c:helm_releases(namespace)
    return api_list(self, HELM_GROUP .. "/namespaces/" .. namespace .. "/helmreleases")
  end

  function c:helm_release(namespace, name)
    return api_get(self, HELM_GROUP .. "/namespaces/" .. namespace .. "/helmreleases/" .. name)
  end

  function c:is_helm_release_ready(namespace, name)
    local resource = self:helm_release(namespace, name)
    return is_ready(resource)
  end

  function c:helm_release_status(namespace, name)
    local resource = self:helm_release(namespace, name)
    if not resource then return nil end
    local status = resource.status or {}
    return {
      ready = is_ready(resource),
      revision = (status.lastAttemptedRevision or ""),
      last_applied_revision = (status.lastAppliedRevision or ""),
      conditions = status.conditions or {},
    }
  end

  -- Notifications

  function c:alerts(namespace)
    return api_list(self, NOTIFICATION_GROUP .. "/namespaces/" .. namespace .. "/alerts")
  end

  function c:providers_list(namespace)
    return api_list(self, NOTIFICATION_GROUP .. "/namespaces/" .. namespace .. "/providers")
  end

  function c:receivers(namespace)
    return api_list(self, NOTIFICATION_GROUP_V1 .. "/namespaces/" .. namespace .. "/receivers")
  end

  -- Image Automation

  function c:image_policies(namespace)
    return api_list(self, IMAGE_GROUP .. "/namespaces/" .. namespace .. "/imagepolicies")
  end

  -- Utilities

  function c:all_sources_ready(namespace)
    local result = { ready = 0, not_ready = 0, total = 0, not_ready_names = {} }

    local git_repos = self:git_repositories(namespace)
    for i = 1, #git_repos.items do
      result.total = result.total + 1
      if is_ready(git_repos.items[i]) then
        result.ready = result.ready + 1
      else
        result.not_ready = result.not_ready + 1
        result.not_ready_names[#result.not_ready_names + 1] = git_repos.items[i].metadata.name
      end
    end

    local helm_repos = self:helm_repositories(namespace)
    for i = 1, #helm_repos.items do
      result.total = result.total + 1
      if is_ready(helm_repos.items[i]) then
        result.ready = result.ready + 1
      else
        result.not_ready = result.not_ready + 1
        result.not_ready_names[#result.not_ready_names + 1] = helm_repos.items[i].metadata.name
      end
    end

    return result
  end

  function c:all_kustomizations_ready(namespace)
    local result = { ready = 0, not_ready = 0, total = 0, not_ready_names = {} }

    local ks_list = self:kustomizations(namespace)
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

  function c:all_helm_releases_ready(namespace)
    local result = { ready = 0, not_ready = 0, total = 0, not_ready_names = {} }

    local hr_list = self:helm_releases(namespace)
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

  return c
end

return M
