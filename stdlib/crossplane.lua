--- @module assay.crossplane
--- @description Crossplane infrastructure management. Providers, XRDs, compositions, managed resources.
--- @keywords crossplane, providers, xrds, compositions, managed, kubernetes, infrastructure, configuration, function, composition, managed-resource, health, readiness, established, terraform
--- @quickref c:providers() -> {items} | List providers
--- @quickref c:provider(name) -> provider|nil | Get provider by name
--- @quickref c:is_provider_healthy(name) -> bool | Check if provider is healthy
--- @quickref c:is_provider_installed(name) -> bool | Check if provider is installed
--- @quickref c:provider_status(name) -> {installed, healthy, current_revision} | Get provider status
--- @quickref c:provider_revisions() -> {items} | List provider revisions
--- @quickref c:provider_revision(name) -> revision|nil | Get provider revision
--- @quickref c:configurations() -> {items} | List configurations
--- @quickref c:configuration(name) -> config|nil | Get configuration by name
--- @quickref c:is_configuration_healthy(name) -> bool | Check if configuration is healthy
--- @quickref c:is_configuration_installed(name) -> bool | Check if configuration is installed
--- @quickref c:functions() -> {items} | List functions
--- @quickref c:xfunction(name) -> function|nil | Get function by name
--- @quickref c:is_function_healthy(name) -> bool | Check if function is healthy
--- @quickref c:xrds() -> {items} | List composite resource definitions
--- @quickref c:xrd(name) -> xrd|nil | Get XRD by name
--- @quickref c:is_xrd_established(name) -> bool | Check if XRD is established
--- @quickref c:compositions() -> {items} | List compositions
--- @quickref c:composition(name) -> composition|nil | Get composition by name
--- @quickref c:managed_resource(api_group, version, kind, name) -> resource|nil | Get managed resource
--- @quickref c:is_managed_ready(api_group, version, kind, name) -> bool | Check if managed resource is ready
--- @quickref c:managed_resources(api_group, version, kind) -> {items} | List managed resources
--- @quickref c:all_providers_healthy() -> {healthy, unhealthy, total} | Check all providers health
--- @quickref c:all_xrds_established() -> {established, not_established, total} | Check all XRDs status

local M = {}

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
      error("crossplane: GET " .. path .. " HTTP " .. resp.status .. ": " .. resp.body)
    end
    return json.parse(resp.body)
  end

  local function api_list(self, path)
    local resp = http.get(self.url .. path, { headers = headers(self) })
    if resp.status == 404 then return { items = {} } end
    if resp.status ~= 200 then
      error("crossplane: LIST " .. path .. " HTTP " .. resp.status .. ": " .. resp.body)
    end
    return json.parse(resp.body)
  end

  local function check_condition(resource, condition_type)
    if not resource or not resource.status or not resource.status.conditions then
      return false
    end
    for _, cond in ipairs(resource.status.conditions) do
      if cond.type == condition_type then
        return cond.status == "True"
      end
    end
    return false
  end

  function c:providers()
    return api_list(self, "/apis/pkg.crossplane.io/v1/providers")
  end

  function c:provider(name)
    return api_get(self, "/apis/pkg.crossplane.io/v1/providers/" .. name)
  end

  function c:is_provider_healthy(name)
    local p = self:provider(name)
    return check_condition(p, "Healthy")
  end

  function c:is_provider_installed(name)
    local p = self:provider(name)
    return check_condition(p, "Installed")
  end

  function c:provider_status(name)
    local p = self:provider(name)
    if not p then
      error("crossplane: provider not found: " .. name)
    end
    local installed = check_condition(p, "Installed")
    local healthy = check_condition(p, "Healthy")
    local current_revision = nil
    if p.status and p.status.currentRevision then
      current_revision = p.status.currentRevision
    end
    local conditions = {}
    if p.status and p.status.conditions then
      conditions = p.status.conditions
    end
    return {
      installed = installed,
      healthy = healthy,
      current_revision = current_revision,
      conditions = conditions,
    }
  end

  function c:provider_revisions()
    return api_list(self, "/apis/pkg.crossplane.io/v1/providerrevisions")
  end

  function c:provider_revision(name)
    return api_get(self, "/apis/pkg.crossplane.io/v1/providerrevisions/" .. name)
  end

  function c:configurations()
    return api_list(self, "/apis/pkg.crossplane.io/v1/configurations")
  end

  function c:configuration(name)
    return api_get(self, "/apis/pkg.crossplane.io/v1/configurations/" .. name)
  end

  function c:is_configuration_healthy(name)
    local cfg = self:configuration(name)
    return check_condition(cfg, "Healthy")
  end

  function c:is_configuration_installed(name)
    local cfg = self:configuration(name)
    return check_condition(cfg, "Installed")
  end

  function c:functions()
    return api_list(self, "/apis/pkg.crossplane.io/v1beta1/functions")
  end

  function c:xfunction(name)
    return api_get(self, "/apis/pkg.crossplane.io/v1beta1/functions/" .. name)
  end

  function c:is_function_healthy(name)
    local fn_resource = self:xfunction(name)
    return check_condition(fn_resource, "Healthy")
  end

  function c:xrds()
    return api_list(self, "/apis/apiextensions.crossplane.io/v1/compositeresourcedefinitions")
  end

  function c:xrd(name)
    return api_get(self, "/apis/apiextensions.crossplane.io/v1/compositeresourcedefinitions/" .. name)
  end

  function c:is_xrd_established(name)
    local x = self:xrd(name)
    return check_condition(x, "Established")
  end

  function c:compositions()
    return api_list(self, "/apis/apiextensions.crossplane.io/v1/compositions")
  end

  function c:composition(name)
    return api_get(self, "/apis/apiextensions.crossplane.io/v1/compositions/" .. name)
  end

  function c:managed_resource(api_group, version, kind, name)
    local path = "/apis/" .. api_group .. "/" .. version .. "/" .. kind .. "/" .. name
    return api_get(self, path)
  end

  function c:is_managed_ready(api_group, version, kind, name)
    local r = self:managed_resource(api_group, version, kind, name)
    return check_condition(r, "Ready")
  end

  function c:managed_resources(api_group, version, kind)
    local path = "/apis/" .. api_group .. "/" .. version .. "/" .. kind
    return api_list(self, path)
  end

  function c:all_providers_healthy()
    local list = self:providers()
    local items = list.items or {}
    local healthy = 0
    local unhealthy = 0
    local unhealthy_names = {}
    for _, p in ipairs(items) do
      if check_condition(p, "Healthy") then
        healthy = healthy + 1
      else
        unhealthy = unhealthy + 1
        local name = p.metadata and p.metadata.name or "unknown"
        unhealthy_names[#unhealthy_names + 1] = name
      end
    end
    return {
      healthy = healthy,
      unhealthy = unhealthy,
      total = #items,
      unhealthy_names = unhealthy_names,
    }
  end

  function c:all_xrds_established()
    local list = self:xrds()
    local items = list.items or {}
    local established = 0
    local not_established = 0
    for _, x in ipairs(items) do
      if check_condition(x, "Established") then
        established = established + 1
      else
        not_established = not_established + 1
      end
    end
    return {
      established = established,
      not_established = not_established,
      total = #items,
    }
  end

  return c
end

return M
