--- @module assay.dex
--- @description Dex OIDC identity provider. Discovery, JWKS, health, and configuration validation.
--- @keywords dex, oidc, identity, discovery, jwks, authentication, openid-configuration, key-set, scope, grant-type, response-type, validation
--- @quickref c.discovery:config() -> {issuer, endpoints...} | Get OIDC discovery configuration
--- @quickref c.discovery:jwks() -> {keys} | Get JSON Web Key Set
--- @quickref c.discovery:issuer() -> string | Get issuer URL from discovery
--- @quickref c.discovery:has_endpoint(endpoint_name) -> bool | Check if endpoint exists in discovery
--- @quickref c.health:check() -> bool | Check Dex health
--- @quickref c.health:ready() -> bool | Check Dex readiness
--- @quickref c.scopes:list() -> [string] | List supported OIDC scopes
--- @quickref c.scopes:supports(scope) -> bool | Check if scope is supported
--- @quickref c.grants:list() -> [string] | List supported grant types
--- @quickref c.grants:supports(grant_type) -> bool | Check if grant type is supported
--- @quickref c.grants:response_types() -> [string] | List supported response types
--- @quickref c:validate_config() -> {ok, errors} | Validate OIDC configuration
--- @quickref c:admin_version() -> version|nil | Get Dex admin API version

local M = {}

-- Legacy module-level functions for backward compatibility
M.discovery = nil
M.jwks = nil
M.issuer = nil
M.health = nil
M.ready = nil
M.has_endpoint = nil
M.supported_scopes = nil
M.supported_response_types = nil
M.supported_grant_types = nil
M.supports_scope = nil
M.supports_grant_type = nil
M.validate_config = nil
M.admin_version = nil

function M.client(url)
  local base_url = url:gsub("/+$", "")

  -- Shared helpers (plain closures capturing base_url as upvalue)

  local function fetch_discovery()
    local resp = http.get(base_url .. "/.well-known/openid-configuration", { headers = {} })
    if resp.status ~= 200 then
      error("dex.discovery: HTTP " .. resp.status .. ": " .. resp.body)
    end
    return json.parse(resp.body)
  end

  -- ===== Client =====

  local c = {}

  -- ===== Discovery =====

  c.discovery = {}

  function c.discovery:config()
    return fetch_discovery()
  end

  function c.discovery:jwks()
    local config = fetch_discovery()
    if not config.jwks_uri then
      error("dex.jwks: discovery response missing jwks_uri")
    end
    local resp = http.get(config.jwks_uri, { headers = {} })
    if resp.status ~= 200 then
      error("dex.jwks: HTTP " .. resp.status .. ": " .. resp.body)
    end
    return json.parse(resp.body)
  end

  function c.discovery:issuer()
    local config = fetch_discovery()
    return config.issuer
  end

  function c.discovery:has_endpoint(endpoint_name)
    local config = fetch_discovery()
    return config[endpoint_name] ~= nil
  end

  -- ===== Health =====

  c.health = {}

  function c.health:check()
    local resp = http.get(base_url .. "/healthz", { headers = {} })
    return resp.status == 200
  end

  function c.health:ready()
    return c.health:check()
  end

  -- ===== Scopes =====

  c.scopes = {}

  function c.scopes:list()
    local config = fetch_discovery()
    return config.scopes_supported or {}
  end

  function c.scopes:supports(scope)
    local scopes = c.scopes:list()
    for _, s in ipairs(scopes) do
      if s == scope then
        return true
      end
    end
    return false
  end

  -- ===== Grants =====

  c.grants = {}

  function c.grants:list()
    local config = fetch_discovery()
    return config.grant_types_supported or {}
  end

  function c.grants:supports(grant_type)
    local types = c.grants:list()
    for _, gt in ipairs(types) do
      if gt == grant_type then
        return true
      end
    end
    return false
  end

  function c.grants:response_types()
    local config = fetch_discovery()
    return config.response_types_supported or {}
  end

  -- ===== Top-level methods =====

  function c:validate_config()
    local errors = {}

    local ok, config = pcall(fetch_discovery)
    if not ok then
      return { ok = false, errors = { "discovery failed: " .. tostring(config) } }
    end

    if not config.issuer then
      errors[#errors + 1] = "missing issuer"
    else
      if config.issuer ~= base_url then
        errors[#errors + 1] = "issuer mismatch: expected " .. base_url .. ", got " .. config.issuer
      end
    end

    if not config.authorization_endpoint then
      errors[#errors + 1] = "missing authorization_endpoint"
    end

    if not config.token_endpoint then
      errors[#errors + 1] = "missing token_endpoint"
    end

    if not config.jwks_uri then
      errors[#errors + 1] = "missing jwks_uri"
    end

    return { ok = #errors == 0, errors = errors }
  end

  function c:admin_version()
    local ok, result = pcall(function()
      local resp = http.get(base_url .. "/api/v1/version", { headers = {} })
      if resp.status ~= 200 then
        error("HTTP " .. resp.status)
      end
      return json.parse(resp.body)
    end)

    if ok then
      return result
    end
    return nil
  end

  return c
end

-- Legacy module-level functions (delegate to a temporary client)
-- These preserve backward compatibility: M.discovery(url) still works.

M.discovery = function(url)
  return M.client(url).discovery:config()
end

M.jwks = function(url)
  return M.client(url).discovery:jwks()
end

M.issuer = function(url)
  return M.client(url).discovery:issuer()
end

M.health = function(url)
  return M.client(url).health:check()
end

M.ready = function(url)
  return M.client(url).health:ready()
end

M.has_endpoint = function(url, endpoint_name)
  return M.client(url).discovery:has_endpoint(endpoint_name)
end

M.supported_scopes = function(url)
  return M.client(url).scopes:list()
end

M.supported_response_types = function(url)
  return M.client(url).grants:response_types()
end

M.supported_grant_types = function(url)
  return M.client(url).grants:list()
end

M.supports_scope = function(url, scope)
  return M.client(url).scopes:supports(scope)
end

M.supports_grant_type = function(url, grant_type)
  return M.client(url).grants:supports(grant_type)
end

M.validate_config = function(url)
  return M.client(url):validate_config()
end

M.admin_version = function(url)
  return M.client(url):admin_version()
end

return M
