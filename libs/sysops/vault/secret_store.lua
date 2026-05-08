--! sysops.vault.secret_store - engine-vault backed host secret store.
--!
--! Extracted verbatim from sysops.vault 0.1.4. The aggregator at
--! sysops/vault.lua re-exports `M.secret_store` for backwards compat.

local M = {}

local RUSTIC_KEYS = { password = true, access_key_id = true, secret_access_key = true }

local function env_get(key)
  if type(env) == "table" and type(env.get) == "function" then
    return env.get(key)
  end
  return nil
end

local function trim_slash(value)
  return (value or ""):gsub("/+$", "")
end

local function encode_segment(value)
  value = tostring(value or "")
  return (value:gsub("([^%w%-%._~])", function(ch)
    return string.format("%%%02X", string.byte(ch))
  end))
end

local function encode_path(value)
  local parts = {}
  for part in tostring(value or ""):gmatch("[^/]+") do
    table.insert(parts, encode_segment(part))
  end
  return table.concat(parts, "/")
end

local function first_env(names)
  for _, name in ipairs(names or {}) do
    local value = env_get(name)
    if value and value ~= "" then return value end
  end
  return nil
end

local function decode_json_body(resp)
  if not resp.body or resp.body == "" then return {} end
  if type(resp.body) == "table" then return resp.body end
  local ok, decoded = pcall(json.parse, resp.body)
  if ok and type(decoded) == "table" then return decoded end
  return nil, "engine vault response was not valid JSON"
end

local function load_file(path)
  if not path or path == "" then return {} end
  if not fs.exists(path) then return {} end
  local read_ok, body = pcall(fs.read, path)
  if not read_ok or not body or body == "" then return {} end
  local ok, decoded = pcall(json.parse, body)
  if ok and type(decoded) == "table" then return decoded end
  log.warn("sysops.vault: failed to parse " .. path)
  return {}
end

local function read_rustic_file(profile_dir, scope, key)
  if not RUSTIC_KEYS[key] then return nil end
  local path = profile_dir .. "/" .. scope .. "." .. key
  if not fs.exists(path) then return nil end
  local read_ok, body = pcall(fs.read, path)
  if not read_ok or not body or body == "" then return nil end
  return (body:gsub("[\r\n]+$", ""))
end

local function build_config(opts)
  opts = opts or {}
  local app = opts.app or env_get("SYSOPS_APP") or "sysops"
  local admin_key = opts.admin_key
    or first_env(opts.admin_key_envs)
    or env_get("ENGINE_ADMIN_KEY")
    or env_get("ASSAY_ADMIN_KEY")
  return {
    app = app,
    engine_url = trim_slash(
      opts.engine_url or env_get("ENGINE_URL") or env_get("ASSAY_ENGINE_URL") or ""
    ),
    admin_key = admin_key,
    kv_prefix = opts.kv_prefix or env_get("VAULT_KV_PREFIX") or app,
    secret_file = opts.secret_file or env_get("SECRET_FILE") or ("/etc/" .. app .. "/secrets.json"),
    rustic_profile_dir = opts.rustic_profile_dir or env_get("BACKUP_PROFILE_DIR") or "/etc/rustic",
  }
end

local function vault_path(cfg, scope, key)
  local prefix = encode_path(cfg.kv_prefix)
  local path = encode_segment(scope) .. "/" .. encode_segment(key)
  if prefix ~= "" then return prefix .. "/" .. path end
  return path
end

local function engine_headers(cfg)
  local headers = { ["Content-Type"] = "application/json" }
  if cfg.admin_key and cfg.admin_key ~= "" then
    headers["Authorization"] = "Bearer " .. cfg.admin_key
  end
  return headers
end

local function engine_request(cfg, method, path, body)
  if cfg.engine_url == "" then
    return nil, "engine URL not configured"
  end

  local url = cfg.engine_url .. path
  local headers = engine_headers(cfg)
  local ok
  local resp

  if type(http.request) == "function" then
    local request = { method = method, url = url, headers = headers }
    if body ~= nil then request.body = json.encode(body) end
    ok, resp = pcall(http.request, request)
  end

  if not ok then
    if method == "GET" then
      ok, resp = pcall(http.get, url, { headers = headers })
    elseif method == "PUT" then
      ok, resp = pcall(http.put, url, body or {}, { headers = headers })
    elseif method == "DELETE" then
      ok, resp = pcall(http.delete, url, { headers = headers })
    else
      return nil, "unsupported engine vault method: " .. tostring(method)
    end
  end

  if not ok or type(resp) ~= "table" then
    return nil, "engine vault request failed: " .. tostring(resp)
  end
  return resp
end

local function read_engine(cfg, scope, key)
  if cfg.engine_url == "" then return nil, nil, false end
  local path = "/api/v1/vault/kv/" .. vault_path(cfg, scope, key)
  local resp, err = engine_request(cfg, "GET", path)
  if not resp then return nil, err, true end
  if resp.status == 404 then return nil, nil, false end
  if resp.status ~= 200 then
    return nil, "engine vault read failed: HTTP " .. tostring(resp.status), true
  end
  local decoded, decode_err = decode_json_body(resp)
  if not decoded then return nil, decode_err, true end
  if decoded.value ~= nil then return decoded.value, nil, true end
  if decoded.data ~= nil then return decoded.data, nil, true end
  return decoded, nil, true
end

function M.secret_store(opts)
  local cfg = build_config(opts)
  local store = {}

  function store.read(scope, key)
    local value, err, engine_terminal = read_engine(cfg, scope, key)
    if value ~= nil then return value end
    if engine_terminal then return nil, err end

    value = read_rustic_file(cfg.rustic_profile_dir, scope, key)
    if value ~= nil then return value end

    local file_store = load_file(cfg.secret_file)
    return (file_store[scope] and file_store[scope][key]) or nil
  end

  function store.write(scope, key, value)
    local path = "/api/v1/vault/kv/" .. vault_path(cfg, scope, key)
    local resp, err = engine_request(cfg, "PUT", path, {
      data = tostring(value or ""),
      custom_md = {
        app = tostring(cfg.app or ""),
        scope = tostring(scope or ""),
        key = tostring(key or ""),
      },
    })
    if not resp then return false, err end
    if resp.status == 200 or resp.status == 201 then return true end
    return false, "engine vault write failed: HTTP " .. tostring(resp.status)
  end

  function store.delete(scope, key)
    local path = "/api/v1/vault/kv/" .. vault_path(cfg, scope, key)
    local resp, err = engine_request(cfg, "GET", path)
    if not resp then return false, err end
    if resp.status == 404 then return true end
    if resp.status ~= 200 then
      return false, "engine vault delete preflight failed: HTTP " .. tostring(resp.status)
    end

    local decoded, decode_err = decode_json_body(resp)
    if not decoded then return false, decode_err end
    if not decoded.version then return false, "engine vault delete preflight missing version" end

    resp, err = engine_request(cfg, "DELETE", path .. "?version=" .. tostring(decoded.version))
    if not resp then return false, err end
    if resp.status == 204 then return true end
    return false, "engine vault delete failed: HTTP " .. tostring(resp.status)
  end

  function store.available()
    local resp, err = engine_request(cfg, "GET", "/api/v1/vault/sys/seal-status")
    if not resp then return false, { error = err } end
    if resp.status ~= 200 then
      return false, { error = "engine vault status failed: HTTP " .. tostring(resp.status) }
    end
    local decoded, decode_err = decode_json_body(resp)
    if not decoded then return false, { error = decode_err } end
    decoded.loaded = true
    return true, decoded
  end

  return store
end

return M
