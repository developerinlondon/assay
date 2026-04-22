--- @module assay.vault
--- @description HashiCorp Vault secrets management. KV, policies, auth, transit, PKI, token management.
--- @keywords vault, secrets, kv, policies, auth, transit, pki, tokens, encryption, decryption, certificate, seal, initialization, authentication, secret-engine, password, rotation
--- @quickref c.kv:get(mount, key) -> {data}|nil | Read KV v2 secret
--- @quickref c.kv:put(mount, key, data) -> result | Write KV v2 secret
--- @quickref c.kv:delete(mount, key) -> nil | Delete KV v2 secret
--- @quickref c.kv:list(mount, prefix?) -> [string] | List KV v2 keys
--- @quickref c.kv:metadata(mount, key) -> metadata|nil | Get KV v2 metadata
--- @quickref c.sys:health() -> {initialized, sealed, version} | Get Vault health
--- @quickref c.sys:seal_status() -> {sealed, initialized} | Get seal status
--- @quickref c.sys:is_sealed() -> bool | Check if Vault is sealed
--- @quickref c.sys:is_initialized() -> bool | Check if Vault is initialized
--- @quickref c.policies:get(name) -> policy|nil | Get ACL policy
--- @quickref c.policies:create(name, rules) -> nil | Create or update ACL policy
--- @quickref c.policies:delete(name) -> nil | Delete ACL policy
--- @quickref c.policies:list() -> [string] | List ACL policies
--- @quickref c.auth:enable(path, type, opts?) -> nil | Enable auth method
--- @quickref c.auth:disable(path) -> nil | Disable auth method
--- @quickref c.auth:methods() -> {path: config} | List auth methods
--- @quickref c.auth:config(path, config) -> nil | Configure auth method
--- @quickref c.auth:create_role(path, role_name, config) -> nil | Create auth role
--- @quickref c.auth:get_role(path, role_name) -> role|nil | Read auth role
--- @quickref c.auth:list_roles(path) -> [string] | List auth roles
--- @quickref c.engines:enable(path, type, opts?) -> nil | Enable secrets engine
--- @quickref c.engines:disable(path) -> nil | Disable secrets engine
--- @quickref c.engines:list() -> {path: config} | List secrets engines
--- @quickref c.engines:tune(path, config) -> nil | Tune secrets engine
--- @quickref c.token:create(opts?) -> {client_token, ...} | Create token
--- @quickref c.token:lookup(token) -> token_info|nil | Lookup token
--- @quickref c.token:lookup_self() -> token_info|nil | Lookup current token
--- @quickref c.token:revoke(token) -> nil | Revoke token
--- @quickref c.token:revoke_self() -> nil | Revoke current token
--- @quickref c.transit:encrypt(key_name, plaintext) -> ciphertext|nil | Encrypt with transit
--- @quickref c.transit:decrypt(key_name, ciphertext) -> plaintext|nil | Decrypt with transit
--- @quickref c.transit:create_key(key_name, opts?) -> nil | Create transit key
--- @quickref c.transit:list_keys() -> [string] | List transit keys
--- @quickref c.pki:issue(mount, role_name, opts?) -> cert|nil | Issue PKI certificate
--- @quickref c.pki:ca_cert(mount?) -> string | Get CA certificate PEM
--- @quickref c.pki:create_role(mount, role_name, opts?) -> nil | Create PKI role
--- @quickref M.wait(url, opts?) -> true | Wait for Vault to become healthy
--- @quickref M.authenticated_client(url, opts?) -> client | Create client with K8s secret auth
--- @quickref M.ensure_credentials(client, path, check_key, generator) -> creds | Ensure credentials exist
--- @quickref M.assert_secret(client, path, expected_keys) -> data | Assert secret exists with keys

local M = {}

function M.client(url, token)
  url = url:gsub("/+$", "")

  -- Shared HTTP helpers (captured by all sub-object methods as upvalues)

  local function headers()
    return { ["X-Vault-Token"] = token }
  end

  local function api_get(path)
    local resp = http.get(url .. path, { headers = headers() })
    if resp.status == 404 then return nil end
    if resp.status ~= 200 then
      error("vault: GET " .. path .. " HTTP " .. resp.status .. ": " .. resp.body)
    end
    return json.parse(resp.body)
  end

  local function api_post(path, payload)
    local resp = http.post(url .. path, payload, { headers = headers() })
    if resp.status ~= 200 and resp.status ~= 204 then
      error("vault: POST " .. path .. " HTTP " .. resp.status .. ": " .. resp.body)
    end
    if resp.status == 204 then return nil end
    return json.parse(resp.body)
  end

  local function api_put(path, payload)
    local resp = http.put(url .. path, payload, { headers = headers() })
    if resp.status ~= 200 and resp.status ~= 204 then
      error("vault: PUT " .. path .. " HTTP " .. resp.status .. ": " .. resp.body)
    end
    if resp.status == 204 then return nil end
    return json.parse(resp.body)
  end

  local function api_delete(path)
    local resp = http.delete(url .. path, { headers = headers() })
    if resp.status ~= 200 and resp.status ~= 204 then
      error("vault: DELETE " .. path .. " HTTP " .. resp.status .. ": " .. resp.body)
    end
  end

  local function api_list(path)
    local resp = http.get(url .. path .. "?list=true", { headers = headers() })
    if resp.status == 404 then return {} end
    if resp.status ~= 200 then
      error("vault: LIST " .. path .. " HTTP " .. resp.status .. ": " .. resp.body)
    end
    local data = json.parse(resp.body)
    return (data.data or {}).keys or {}
  end

  -- ===== Client =====

  local c = {}

  -- ===== Raw API =====

  function c:read(path)
    local data = api_get("/v1/" .. path)
    if not data then return nil end
    return data.data
  end

  function c:write(path, payload)
    return api_post("/v1/" .. path, payload)
  end

  function c:delete(path)
    return api_delete("/v1/" .. path)
  end

  function c:list(path)
    return api_list("/v1/" .. path)
  end

  -- ===== KV v2 =====

  c.kv = {}

  function c.kv:get(mount, key)
    local data = api_get("/v1/" .. mount .. "/data/" .. key)
    if not data then return nil end
    return data.data
  end

  function c.kv:put(mount, key, data)
    return api_post("/v1/" .. mount .. "/data/" .. key, { data = data })
  end

  function c.kv:delete(mount, key)
    return api_delete("/v1/" .. mount .. "/data/" .. key)
  end

  function c.kv:list(mount, prefix)
    prefix = prefix or ""
    return api_list("/v1/" .. mount .. "/metadata/" .. prefix)
  end

  function c.kv:metadata(mount, key)
    return api_get("/v1/" .. mount .. "/metadata/" .. key)
  end

  -- ===== System / Health =====

  c.sys = {}

  function c.sys:health()
    local resp = http.get(url .. "/v1/sys/health")
    return json.parse(resp.body)
  end

  function c.sys:seal_status()
    local resp = http.get(url .. "/v1/sys/seal-status")
    if resp.status ~= 200 then
      error("vault: seal-status HTTP " .. resp.status .. ": " .. resp.body)
    end
    return json.parse(resp.body)
  end

  function c.sys:is_sealed()
    return c.sys:seal_status().sealed
  end

  function c.sys:is_initialized()
    return c.sys:seal_status().initialized
  end

  -- ===== ACL Policies =====

  c.policies = {}

  function c.policies:get(name)
    local data = api_get("/v1/sys/policies/acl/" .. name)
    if not data then return nil end
    return data.data
  end

  function c.policies:create(name, rules)
    return api_put("/v1/sys/policies/acl/" .. name, { policy = rules })
  end

  function c.policies:delete(name)
    return api_delete("/v1/sys/policies/acl/" .. name)
  end

  function c.policies:list()
    return api_list("/v1/sys/policies/acl")
  end

  -- ===== Auth Methods =====

  c.auth = {}

  function c.auth:enable(path, auth_type, opts)
    opts = opts or {}
    local payload = { type = auth_type }
    if opts.description then payload.description = opts.description end
    if opts.config then payload.config = opts.config end
    return api_post("/v1/sys/auth/" .. path, payload)
  end

  function c.auth:disable(path)
    return api_delete("/v1/sys/auth/" .. path)
  end

  function c.auth:methods()
    local data = api_get("/v1/sys/auth")
    return data and data.data or data
  end

  function c.auth:config(path, config)
    return api_post("/v1/auth/" .. path .. "/config", config)
  end

  function c.auth:create_role(path, role_name, role_config)
    return api_post("/v1/auth/" .. path .. "/role/" .. role_name, role_config)
  end

  function c.auth:get_role(path, role_name)
    local data = api_get("/v1/auth/" .. path .. "/role/" .. role_name)
    if not data then return nil end
    return data.data
  end

  function c.auth:list_roles(path)
    return api_list("/v1/auth/" .. path .. "/role")
  end

  -- ===== Secrets Engines =====

  c.engines = {}

  function c.engines:enable(path, engine_type, opts)
    opts = opts or {}
    local payload = { type = engine_type }
    if opts.description then payload.description = opts.description end
    if opts.config then payload.config = opts.config end
    if opts.options then payload.options = opts.options end
    return api_post("/v1/sys/mounts/" .. path, payload)
  end

  function c.engines:disable(path)
    return api_delete("/v1/sys/mounts/" .. path)
  end

  function c.engines:list()
    local data = api_get("/v1/sys/mounts")
    return data and data.data or data
  end

  function c.engines:tune(path, config)
    return api_post("/v1/sys/mounts/" .. path .. "/tune", config)
  end

  -- ===== Token Management =====

  c.token = {}

  function c.token:create(opts)
    opts = opts or {}
    local data = api_post("/v1/auth/token/create", opts)
    return data and data.auth or nil
  end

  function c.token:lookup(token_value)
    local data = api_post("/v1/auth/token/lookup", { token = token_value })
    return data and data.data or nil
  end

  function c.token:lookup_self()
    local data = api_get("/v1/auth/token/lookup-self")
    return data and data.data or nil
  end

  function c.token:revoke(token_value)
    return api_post("/v1/auth/token/revoke", { token = token_value })
  end

  function c.token:revoke_self()
    return api_post("/v1/auth/token/revoke-self", {})
  end

  -- ===== Transit Encryption =====

  c.transit = {}

  function c.transit:encrypt(key_name, plaintext)
    local encoded = base64.encode(plaintext)
    local data = api_post("/v1/transit/encrypt/" .. key_name, { plaintext = encoded })
    return data and data.data and data.data.ciphertext or nil
  end

  function c.transit:decrypt(key_name, ciphertext)
    local data = api_post("/v1/transit/decrypt/" .. key_name, { ciphertext = ciphertext })
    if data and data.data and data.data.plaintext then
      return base64.decode(data.data.plaintext)
    end
    return nil
  end

  function c.transit:create_key(key_name, opts)
    return api_post("/v1/transit/keys/" .. key_name, opts or {})
  end

  function c.transit:list_keys()
    return api_list("/v1/transit/keys")
  end

  -- ===== PKI Certificates =====

  c.pki = {}

  function c.pki:issue(mount, role_name, opts)
    local data = api_post("/v1/" .. mount .. "/issue/" .. role_name, opts or {})
    return data and data.data or nil
  end

  function c.pki:ca_cert(mount)
    mount = mount or "pki"
    local resp = http.get(url .. "/v1/" .. mount .. "/ca/pem")
    if resp.status ~= 200 then
      error("vault: pki ca cert HTTP " .. resp.status)
    end
    return resp.body
  end

  function c.pki:create_role(mount, role_name, opts)
    return api_post("/v1/" .. mount .. "/roles/" .. role_name, opts or {})
  end

  return c
end

function M.wait(url, opts)
  opts = opts or {}
  local timeout = opts.timeout or 60
  local interval = opts.interval or 2
  local health_path = opts.health_path or "/v1/sys/health?standbyok=true&sealedcode=200&uninitcode=200"
  local max_attempts = math.ceil(timeout / interval)

  for i = 1, max_attempts do
    local ok, resp = pcall(http.get, url .. health_path)
    if ok and resp.status == 200 then
      log.info("Vault healthy after " .. tostring(i * interval) .. "s")
      return true
    end
    if i == max_attempts then
      error("vault.wait: not reachable at " .. url .. " after " .. tostring(timeout) .. "s")
    end
    log.info("Waiting for Vault... (" .. tostring(i) .. "/" .. tostring(max_attempts) .. ")")
    sleep(interval)
  end
end

function M.authenticated_client(url, opts)
  opts = opts or {}
  local k8s = require("assay.k8s")

  -- Wait for vault to be healthy first
  M.wait(url, { timeout = opts.timeout or 60, interval = opts.interval or 2 })

  -- Get token from K8s secret
  local secret_ns = opts.secret_namespace or opts.secret_ns or "secrets"
  local secret_name = opts.secret_name or "openbao-root-token"
  local secret_key = opts.secret_key or "root-token"

  local secret_data = k8s.secrets:get(secret_ns, secret_name)
  local token = secret_data[secret_key]
  assert.not_nil(token, "vault.authenticated_client: key '" .. secret_key .. "' not found in secret " .. secret_ns .. "/" .. secret_name)

  -- Trim whitespace
  token = token:match("^%s*(.-)%s*$")

  return M.client(url, token)
end

function M.ensure_credentials(client, path, check_key, generator)
  -- Check if credentials already exist
  local existing = client.kv:get("secrets", path)
  if existing and existing.data and existing.data[check_key] then
    log.info("Credentials already exist at secrets/" .. path)
    return existing.data
  end

  -- Generate new credentials
  local creds = generator()
  client.kv:put("secrets", path, creds)
  log.info("Generated and stored credentials at secrets/" .. path)
  return creds
end

function M.assert_secret(client, path, expected_keys)
  local data = client.kv:get("secrets", path)
  assert.not_nil(data, "vault.assert_secret: secret not found at secrets/" .. path)
  assert.not_nil(data.data, "vault.assert_secret: no data at secrets/" .. path)

  for _, key in ipairs(expected_keys) do
    assert.not_nil(data.data[key], "vault.assert_secret: key '" .. key .. "' missing at secrets/" .. path)
  end

  log.info("Secret verified at secrets/" .. path .. " (" .. tostring(#expected_keys) .. " keys)")
  return data.data
end

return M
