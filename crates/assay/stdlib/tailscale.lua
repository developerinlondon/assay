--- @module assay.tailscale
--- @description Tailscale REST API client. OAuth2 client_credentials, mint auth keys, list/find devices, manage device key expiry, tags, authorize, delete, ACL preview.
--- @keywords tailscale, ts, oauth2, authkey, mint, device, tailnet, key-expiry, tags, acl
--- @quickref tailscale.client(opts?) -> client | OAuth2-authed Tailscale REST client
--- @quickref c:mint_key(opts) -> key | POST /tailnet/{tailnet}/keys
--- @quickref c:list_devices() -> [device] | GET /tailnet/{tailnet}/devices
--- @quickref c:find_device({hostname=...}) -> device|nil | First device whose hostname or name matches
--- @quickref c:get_device(id) -> device | GET /device/{id}
--- @quickref c:set_key_expiry(id, {disabled=bool}) -> "changed"|"unchanged" | Idempotent POST /device/{id}/key
--- @quickref c:authorize_device(id) -> table | POST /device/{id}/authorized
--- @quickref c:set_device_tags(id, {"tag:..."}) -> table | POST /device/{id}/tags
--- @quickref c:delete_device(id) -> true | DELETE /device/{id}
--- @quickref c:acl_test(opts) -> result | POST /tailnet/{tailnet}/acl/preview

local url = require("assay.url")

local M = {}

local DEFAULT_BASE_URL = "https://api.tailscale.com"
local DEFAULT_TAILNET = "-"
local DEFAULT_SCOPE = "all:write"
local TOKEN_SKEW_SECONDS = 30

function M.client(opts)
  opts = opts or {}
  local client_id = opts.client_id or env.get("TS_CLIENT_ID")
  local client_secret = opts.client_secret or env.get("TS_CLIENT_SECRET")
  if not client_id or client_id == "" then
    error("tailscale.client: missing client_id (set TS_CLIENT_ID or pass client_id)")
  end
  if not client_secret or client_secret == "" then
    error("tailscale.client: missing client_secret (set TS_CLIENT_SECRET or pass client_secret)")
  end
  local base_url = (opts.base_url or DEFAULT_BASE_URL):gsub("/+$", "")
  local tailnet = opts.tailnet or DEFAULT_TAILNET
  local scope = opts.scope or DEFAULT_SCOPE

  local cached_token = nil
  local cached_expires_at = 0

  local function fetch_token()
    local body = url.encode_form({
      grant_type = "client_credentials",
      client_id = client_id,
      client_secret = client_secret,
      scope = scope,
    })
    local resp = http.post(base_url .. "/api/v2/oauth/token", body, {
      headers = { ["Content-Type"] = "application/x-www-form-urlencoded" },
    })
    if resp.status ~= 200 then
      error("tailscale.client: token exchange HTTP " .. resp.status .. ": " .. (resp.body or ""))
    end
    local parsed = json.parse(resp.body)
    if not parsed.access_token then
      error("tailscale.client: token response missing access_token")
    end
    cached_token = parsed.access_token
    local ttl = tonumber(parsed.expires_in) or 3600
    cached_expires_at = os.time() + ttl
  end

  local function token()
    if not cached_token or os.time() >= (cached_expires_at - TOKEN_SKEW_SECONDS) then
      fetch_token()
    end
    return cached_token
  end

  local function auth_headers()
    return {
      ["Authorization"] = "Bearer " .. token(),
      ["Content-Type"] = "application/json",
      ["Accept"] = "application/json",
    }
  end

  local function api_get(path_str)
    local resp = http.get(base_url .. path_str, { headers = auth_headers() })
    if resp.status ~= 200 then
      error("tailscale: GET " .. path_str .. " HTTP " .. resp.status .. ": " .. (resp.body or ""))
    end
    if resp.body == nil or resp.body == "" then return nil end
    return json.parse(resp.body)
  end

  local function api_post(path_str, payload)
    local resp = http.post(base_url .. path_str, payload or {}, { headers = auth_headers() })
    if resp.status ~= 200 and resp.status ~= 201 and resp.status ~= 204 then
      error("tailscale: POST " .. path_str .. " HTTP " .. resp.status .. ": " .. (resp.body or ""))
    end
    if resp.body == nil or resp.body == "" then return true end
    local ok, parsed = pcall(json.parse, resp.body)
    if ok then return parsed end
    return resp.body
  end

  local function api_delete(path_str)
    local resp = http.delete(base_url .. path_str, { headers = auth_headers() })
    if resp.status ~= 200 and resp.status ~= 204 then
      error("tailscale: DELETE " .. path_str .. " HTTP " .. resp.status .. ": " .. (resp.body or ""))
    end
    return true
  end

  local c = {}
  c.tailnet = tailnet
  c.base_url = base_url

  function c:mint_key(mint_opts)
    mint_opts = mint_opts or {}
    local capabilities = {
      devices = {
        create = {
          reusable = mint_opts.reusable and true or false,
          ephemeral = mint_opts.ephemeral and true or false,
          preauthorized = mint_opts.preauthorized and true or false,
          tags = json.array(mint_opts.tags or {}),
        },
      },
    }
    local payload = { capabilities = capabilities }
    if mint_opts.expiry_seconds then
      payload.expirySeconds = mint_opts.expiry_seconds
    end
    if mint_opts.description then
      payload.description = mint_opts.description
    end
    return api_post("/api/v2/tailnet/" .. tailnet .. "/keys", payload)
  end

  function c:list_devices()
    local result = api_get("/api/v2/tailnet/" .. tailnet .. "/devices")
    if type(result) == "table" and result.devices then
      return result.devices
    end
    return result or {}
  end

  function c:find_device(query)
    query = query or {}
    local devices = self:list_devices()
    if query.hostname then
      for _, d in ipairs(devices) do
        if d.hostname == query.hostname then return d end
      end
      for _, d in ipairs(devices) do
        if d.name and (d.name == query.hostname
            or d.name:sub(1, #query.hostname + 1) == query.hostname .. ".") then
          return d
        end
      end
    end
    if query.id then
      for _, d in ipairs(devices) do
        if d.id == query.id or d.nodeId == query.id then return d end
      end
    end
    return nil
  end

  function c:get_device(id)
    if not id or id == "" then
      error("tailscale.get_device: missing device id")
    end
    return api_get("/api/v2/device/" .. id)
  end

  function c:set_key_expiry(id, expiry_opts)
    if not id or id == "" then
      error("tailscale.set_key_expiry: missing device id")
    end
    expiry_opts = expiry_opts or {}
    local desired = expiry_opts.disabled and true or false
    local current = self:get_device(id)
    if current and current.keyExpiryDisabled == desired then
      return "unchanged"
    end
    api_post("/api/v2/device/" .. id .. "/key", { keyExpiryDisabled = desired })
    return "changed"
  end

  function c:authorize_device(id)
    if not id or id == "" then
      error("tailscale.authorize_device: missing device id")
    end
    return api_post("/api/v2/device/" .. id .. "/authorized", { authorized = true })
  end

  function c:set_device_tags(id, tags)
    if not id or id == "" then
      error("tailscale.set_device_tags: missing device id")
    end
    if type(tags) ~= "table" then
      error("tailscale.set_device_tags: tags must be an array of strings")
    end
    return api_post("/api/v2/device/" .. id .. "/tags", { tags = tags })
  end

  function c:delete_device(id)
    if not id or id == "" then
      error("tailscale.delete_device: missing device id")
    end
    return api_delete("/api/v2/device/" .. id)
  end

  function c:acl_test(acl_opts)
    return api_post("/api/v2/tailnet/" .. tailnet .. "/acl/preview", acl_opts or {})
  end

  return c
end

function M.acl_test(client, acl_opts)
  return client:acl_test(acl_opts)
end

return M
