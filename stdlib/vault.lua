local M = {}

function M.client(url, token)
  local c = {
    url = url:gsub("/+$", ""),
    token = token,
  }

  local function headers(self)
    return { ["X-Vault-Token"] = self.token }
  end

  local function api_get(self, path)
    local resp = http.get(self.url .. path, { headers = headers(self) })
    if resp.status == 404 then return nil end
    if resp.status ~= 200 then
      error("vault: GET " .. path .. " HTTP " .. resp.status .. ": " .. resp.body)
    end
    return json.parse(resp.body)
  end

  local function api_post(self, path, payload)
    local resp = http.post(self.url .. path, payload, { headers = headers(self) })
    if resp.status ~= 200 and resp.status ~= 204 then
      error("vault: POST " .. path .. " HTTP " .. resp.status .. ": " .. resp.body)
    end
    if resp.status == 204 then return nil end
    return json.parse(resp.body)
  end

  local function api_put(self, path, payload)
    local resp = http.put(self.url .. path, payload, { headers = headers(self) })
    if resp.status ~= 200 and resp.status ~= 204 then
      error("vault: PUT " .. path .. " HTTP " .. resp.status .. ": " .. resp.body)
    end
    if resp.status == 204 then return nil end
    return json.parse(resp.body)
  end

  local function api_delete(self, path)
    local resp = http.delete(self.url .. path, { headers = headers(self) })
    if resp.status ~= 200 and resp.status ~= 204 then
      error("vault: DELETE " .. path .. " HTTP " .. resp.status .. ": " .. resp.body)
    end
  end

  local function api_list(self, path)
    local resp = http.get(self.url .. path .. "?list=true", { headers = headers(self) })
    if resp.status == 404 then return {} end
    if resp.status ~= 200 then
      error("vault: LIST " .. path .. " HTTP " .. resp.status .. ": " .. resp.body)
    end
    local data = json.parse(resp.body)
    return (data.data or {}).keys or {}
  end

  function c:read(path)
    local data = api_get(self, "/v1/" .. path)
    if not data then return nil end
    return data.data
  end

  function c:write(path, payload)
    return api_post(self, "/v1/" .. path, payload)
  end

  function c:delete(path)
    return api_delete(self, "/v1/" .. path)
  end

  function c:list(path)
    return api_list(self, "/v1/" .. path)
  end

  function c:kv_get(mount, key)
    return self:read(mount .. "/data/" .. key)
  end

  function c:kv_put(mount, key, data)
    return self:write(mount .. "/data/" .. key, { data = data })
  end

  function c:kv_delete(mount, key)
    return self:delete(mount .. "/data/" .. key)
  end

  function c:kv_list(mount, prefix)
    prefix = prefix or ""
    return api_list(self, "/v1/" .. mount .. "/metadata/" .. prefix)
  end

  function c:kv_metadata(mount, key)
    return api_get(self, "/v1/" .. mount .. "/metadata/" .. key)
  end

  function c:health()
    local resp = http.get(self.url .. "/v1/sys/health")
    return json.parse(resp.body)
  end

  function c:seal_status()
    local resp = http.get(self.url .. "/v1/sys/seal-status")
    if resp.status ~= 200 then
      error("vault: seal-status HTTP " .. resp.status .. ": " .. resp.body)
    end
    return json.parse(resp.body)
  end

  function c:is_sealed()
    return self:seal_status().sealed
  end

  function c:is_initialized()
    return self:seal_status().initialized
  end

  function c:policy_get(name)
    local data = api_get(self, "/v1/sys/policies/acl/" .. name)
    if not data then return nil end
    return data.data
  end

  function c:policy_put(name, rules)
    return api_put(self, "/v1/sys/policies/acl/" .. name, { policy = rules })
  end

  function c:policy_delete(name)
    return api_delete(self, "/v1/sys/policies/acl/" .. name)
  end

  function c:policy_list()
    return api_list(self, "/v1/sys/policies/acl")
  end

  function c:auth_enable(path, auth_type, opts)
    opts = opts or {}
    local payload = { type = auth_type }
    if opts.description then payload.description = opts.description end
    if opts.config then payload.config = opts.config end
    return api_post(self, "/v1/sys/auth/" .. path, payload)
  end

  function c:auth_disable(path)
    return api_delete(self, "/v1/sys/auth/" .. path)
  end

  function c:auth_list()
    local data = api_get(self, "/v1/sys/auth")
    return data and data.data or data
  end

  function c:auth_config(path, config)
    return api_post(self, "/v1/auth/" .. path .. "/config", config)
  end

  function c:auth_create_role(path, role_name, role_config)
    return api_post(self, "/v1/auth/" .. path .. "/role/" .. role_name, role_config)
  end

  function c:auth_read_role(path, role_name)
    local data = api_get(self, "/v1/auth/" .. path .. "/role/" .. role_name)
    if not data then return nil end
    return data.data
  end

  function c:auth_list_roles(path)
    return api_list(self, "/v1/auth/" .. path .. "/role")
  end

  function c:engine_enable(path, engine_type, opts)
    opts = opts or {}
    local payload = { type = engine_type }
    if opts.description then payload.description = opts.description end
    if opts.config then payload.config = opts.config end
    if opts.options then payload.options = opts.options end
    return api_post(self, "/v1/sys/mounts/" .. path, payload)
  end

  function c:engine_disable(path)
    return api_delete(self, "/v1/sys/mounts/" .. path)
  end

  function c:engine_list()
    local data = api_get(self, "/v1/sys/mounts")
    return data and data.data or data
  end

  function c:engine_tune(path, config)
    return api_post(self, "/v1/sys/mounts/" .. path .. "/tune", config)
  end

  function c:token_create(opts)
    opts = opts or {}
    local data = api_post(self, "/v1/auth/token/create", opts)
    return data and data.auth or nil
  end

  function c:token_lookup(token_value)
    local data = api_post(self, "/v1/auth/token/lookup", { token = token_value })
    return data and data.data or nil
  end

  function c:token_lookup_self()
    local data = api_get(self, "/v1/auth/token/lookup-self")
    return data and data.data or nil
  end

  function c:token_revoke(token_value)
    return api_post(self, "/v1/auth/token/revoke", { token = token_value })
  end

  function c:token_revoke_self()
    return api_post(self, "/v1/auth/token/revoke-self", {})
  end

  function c:transit_encrypt(key_name, plaintext)
    local encoded = base64.encode(plaintext)
    local data = api_post(self, "/v1/transit/encrypt/" .. key_name, { plaintext = encoded })
    return data and data.data and data.data.ciphertext or nil
  end

  function c:transit_decrypt(key_name, ciphertext)
    local data = api_post(self, "/v1/transit/decrypt/" .. key_name, { ciphertext = ciphertext })
    if data and data.data and data.data.plaintext then
      return base64.decode(data.data.plaintext)
    end
    return nil
  end

  function c:transit_create_key(key_name, opts)
    return api_post(self, "/v1/transit/keys/" .. key_name, opts or {})
  end

  function c:transit_list_keys()
    return api_list(self, "/v1/transit/keys")
  end

  function c:pki_issue(mount, role_name, opts)
    local data = api_post(self, "/v1/" .. mount .. "/issue/" .. role_name, opts or {})
    return data and data.data or nil
  end

  function c:pki_ca_cert(mount)
    mount = mount or "pki"
    local resp = http.get(self.url .. "/v1/" .. mount .. "/ca/pem")
    if resp.status ~= 200 then
      error("vault: pki ca cert HTTP " .. resp.status)
    end
    return resp.body
  end

  function c:pki_create_role(mount, role_name, opts)
    return api_post(self, "/v1/" .. mount .. "/roles/" .. role_name, opts or {})
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

  local secret_data = k8s.get_secret(secret_ns, secret_name)
  local token = secret_data[secret_key]
  assert.not_nil(token, "vault.authenticated_client: key '" .. secret_key .. "' not found in secret " .. secret_ns .. "/" .. secret_name)

  -- Trim whitespace
  token = token:match("^%s*(.-)%s*$")

  return M.client(url, token)
end

function M.ensure_credentials(client, path, check_key, generator)
  -- Check if credentials already exist
  local existing = client:kv_get("secrets", path)
  if existing and existing.data and existing.data[check_key] then
    log.info("Credentials already exist at secrets/" .. path)
    return existing.data
  end

  -- Generate new credentials
  local creds = generator()
  client:kv_put("secrets", path, creds)
  log.info("Generated and stored credentials at secrets/" .. path)
  return creds
end

function M.assert_secret(client, path, expected_keys)
  local data = client:kv_get("secrets", path)
  assert.not_nil(data, "vault.assert_secret: secret not found at secrets/" .. path)
  assert.not_nil(data.data, "vault.assert_secret: no data at secrets/" .. path)

  for _, key in ipairs(expected_keys) do
    assert.not_nil(data.data[key], "vault.assert_secret: key '" .. key .. "' missing at secrets/" .. path)
  end

  log.info("Secret verified at secrets/" .. path .. " (" .. tostring(#expected_keys) .. " keys)")
  return data.data
end

return M
