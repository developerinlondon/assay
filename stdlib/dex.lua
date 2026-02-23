--- @module assay.dex
--- @description Dex OIDC identity provider. Discovery, JWKS, health, and configuration validation.
--- @keywords dex, oidc, identity, discovery, jwks, authentication
--- @quickref M.discovery(url) -> {issuer, endpoints...} | Get OIDC discovery configuration
--- @quickref M.jwks(url) -> {keys} | Get JSON Web Key Set
--- @quickref M.issuer(url) -> string | Get issuer URL from discovery
--- @quickref M.health(url) -> bool | Check Dex health
--- @quickref M.ready(url) -> bool | Check Dex readiness
--- @quickref M.has_endpoint(url, endpoint_name) -> bool | Check if endpoint exists in discovery
--- @quickref M.supported_scopes(url) -> [string] | List supported OIDC scopes
--- @quickref M.supported_response_types(url) -> [string] | List supported response types
--- @quickref M.supported_grant_types(url) -> [string] | List supported grant types
--- @quickref M.supports_scope(url, scope) -> bool | Check if scope is supported
--- @quickref M.supports_grant_type(url, grant_type) -> bool | Check if grant type is supported
--- @quickref M.validate_config(url) -> {ok, errors} | Validate OIDC configuration
--- @quickref M.admin_version(url) -> version|nil | Get Dex admin API version

local M = {}

function M.discovery(url)
  local base = url:gsub("/+$", "")
  local resp = http.get(base .. "/.well-known/openid-configuration", { headers = {} })

  if resp.status ~= 200 then
    error("dex.discovery: HTTP " .. resp.status .. ": " .. resp.body)
  end

  return json.parse(resp.body)
end

function M.jwks(url)
  local config = M.discovery(url)

  if not config.jwks_uri then
    error("dex.jwks: discovery response missing jwks_uri")
  end

  local resp = http.get(config.jwks_uri, { headers = {} })

  if resp.status ~= 200 then
    error("dex.jwks: HTTP " .. resp.status .. ": " .. resp.body)
  end

  return json.parse(resp.body)
end

function M.issuer(url)
  local config = M.discovery(url)
  return config.issuer
end

function M.health(url)
  local base = url:gsub("/+$", "")
  local resp = http.get(base .. "/healthz", { headers = {} })
  return resp.status == 200
end

function M.ready(url)
  return M.health(url)
end

function M.has_endpoint(url, endpoint_name)
  local config = M.discovery(url)
  return config[endpoint_name] ~= nil
end

function M.supported_scopes(url)
  local config = M.discovery(url)
  return config.scopes_supported or {}
end

function M.supported_response_types(url)
  local config = M.discovery(url)
  return config.response_types_supported or {}
end

function M.supported_grant_types(url)
  local config = M.discovery(url)
  return config.grant_types_supported or {}
end

function M.supports_scope(url, scope)
  local scopes = M.supported_scopes(url)
  for _, s in ipairs(scopes) do
    if s == scope then
      return true
    end
  end
  return false
end

function M.supports_grant_type(url, grant_type)
  local types = M.supported_grant_types(url)
  for _, gt in ipairs(types) do
    if gt == grant_type then
      return true
    end
  end
  return false
end

function M.validate_config(url)
  local errors = {}

  local ok, config = pcall(M.discovery, url)
  if not ok then
    return { ok = false, errors = { "discovery failed: " .. tostring(config) } }
  end

  if not config.issuer then
    errors[#errors + 1] = "missing issuer"
  else
    local base = url:gsub("/+$", "")
    if config.issuer ~= base then
      errors[#errors + 1] = "issuer mismatch: expected " .. base .. ", got " .. config.issuer
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

function M.admin_version(url)
  local base = url:gsub("/+$", "")
  local ok, result = pcall(function()
    local resp = http.get(base .. "/api/v1/version", { headers = {} })
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

return M
