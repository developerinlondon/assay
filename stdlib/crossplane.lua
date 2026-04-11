--- @module assay.crossplane
--- @description Crossplane infrastructure management. Providers, XRDs, compositions, managed resources.
--- @keywords crossplane, providers, xrds, compositions, managed, kubernetes, infrastructure, configuration, function, composition, managed-resource, health, readiness, established, terraform
--- @quickref c.providers:list() -> {items} | List providers
--- @quickref c.providers:get(name) -> provider|nil | Get provider by name
--- @quickref c.providers:is_healthy(name) -> bool | Check if provider is healthy
--- @quickref c.providers:is_installed(name) -> bool | Check if provider is installed
--- @quickref c.providers:status(name) -> {installed, healthy, current_revision} | Get provider status
--- @quickref c.providers:all_healthy() -> {healthy, unhealthy, total} | Check all providers health
--- @quickref c.provider_revisions:list() -> {items} | List provider revisions
--- @quickref c.provider_revisions:get(name) -> revision|nil | Get provider revision
--- @quickref c.configurations:list() -> {items} | List configurations
--- @quickref c.configurations:get(name) -> config|nil | Get configuration by name
--- @quickref c.configurations:is_healthy(name) -> bool | Check if configuration is healthy
--- @quickref c.configurations:is_installed(name) -> bool | Check if configuration is installed
--- @quickref c.functions:list() -> {items} | List functions
--- @quickref c.functions:get(name) -> function|nil | Get function by name
--- @quickref c.functions:is_healthy(name) -> bool | Check if function is healthy
--- @quickref c.xrds:list() -> {items} | List composite resource definitions
--- @quickref c.xrds:get(name) -> xrd|nil | Get XRD by name
--- @quickref c.xrds:is_established(name) -> bool | Check if XRD is established
--- @quickref c.xrds:all_established() -> {established, not_established, total} | Check all XRDs status
--- @quickref c.compositions:list() -> {items} | List compositions
--- @quickref c.compositions:get(name) -> composition|nil | Get composition by name
--- @quickref c.managed_resources:get(api_group, version, kind, name) -> resource|nil | Get managed resource
--- @quickref c.managed_resources:is_ready(api_group, version, kind, name) -> bool | Check if managed resource is ready
--- @quickref c.managed_resources:list(api_group, version, kind) -> {items} | List managed resources

local M = {}

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
      error("crossplane: GET " .. path .. " HTTP " .. resp.status .. ": " .. resp.body)
    end
    return json.parse(resp.body)
  end

  local function api_list(path)
    local resp = http.get(base_url .. path, { headers = headers() })
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

  -- ===== Client =====

  local c = {}

  -- ===== Providers =====

  c.providers = {}

  function c.providers:list()
    return api_list("/apis/pkg.crossplane.io/v1/providers")
  end

  function c.providers:get(name)
    return api_get("/apis/pkg.crossplane.io/v1/providers/" .. name)
  end

  function c.providers:is_healthy(name)
    local p = c.providers:get(name)
    return check_condition(p, "Healthy")
  end

  function c.providers:is_installed(name)
    local p = c.providers:get(name)
    return check_condition(p, "Installed")
  end

  function c.providers:status(name)
    local p = c.providers:get(name)
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

  function c.providers:all_healthy()
    local list = c.providers:list()
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

  -- ===== Provider Revisions =====

  c.provider_revisions = {}

  function c.provider_revisions:list()
    return api_list("/apis/pkg.crossplane.io/v1/providerrevisions")
  end

  function c.provider_revisions:get(name)
    return api_get("/apis/pkg.crossplane.io/v1/providerrevisions/" .. name)
  end

  -- ===== Configurations =====

  c.configurations = {}

  function c.configurations:list()
    return api_list("/apis/pkg.crossplane.io/v1/configurations")
  end

  function c.configurations:get(name)
    return api_get("/apis/pkg.crossplane.io/v1/configurations/" .. name)
  end

  function c.configurations:is_healthy(name)
    local cfg = c.configurations:get(name)
    return check_condition(cfg, "Healthy")
  end

  function c.configurations:is_installed(name)
    local cfg = c.configurations:get(name)
    return check_condition(cfg, "Installed")
  end

  -- ===== Functions =====

  c.functions = {}

  function c.functions:list()
    return api_list("/apis/pkg.crossplane.io/v1beta1/functions")
  end

  function c.functions:get(name)
    return api_get("/apis/pkg.crossplane.io/v1beta1/functions/" .. name)
  end

  function c.functions:is_healthy(name)
    local fn_resource = c.functions:get(name)
    return check_condition(fn_resource, "Healthy")
  end

  -- ===== XRDs =====

  c.xrds = {}

  function c.xrds:list()
    return api_list("/apis/apiextensions.crossplane.io/v1/compositeresourcedefinitions")
  end

  function c.xrds:get(name)
    return api_get("/apis/apiextensions.crossplane.io/v1/compositeresourcedefinitions/" .. name)
  end

  function c.xrds:is_established(name)
    local x = c.xrds:get(name)
    return check_condition(x, "Established")
  end

  function c.xrds:all_established()
    local list = c.xrds:list()
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

  -- ===== Compositions =====

  c.compositions = {}

  function c.compositions:list()
    return api_list("/apis/apiextensions.crossplane.io/v1/compositions")
  end

  function c.compositions:get(name)
    return api_get("/apis/apiextensions.crossplane.io/v1/compositions/" .. name)
  end

  -- ===== Managed Resources =====

  c.managed_resources = {}

  function c.managed_resources:get(api_group, version, kind, name)
    local path = "/apis/" .. api_group .. "/" .. version .. "/" .. kind .. "/" .. name
    return api_get(path)
  end

  function c.managed_resources:is_ready(api_group, version, kind, name)
    local r = c.managed_resources:get(api_group, version, kind, name)
    return check_condition(r, "Ready")
  end

  function c.managed_resources:list(api_group, version, kind)
    local path = "/apis/" .. api_group .. "/" .. version .. "/" .. kind
    return api_list(path)
  end

  return c
end

return M
