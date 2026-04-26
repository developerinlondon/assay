--- @module assay.vault
--- @description assay-engine vault client (plan 17 / v0.3.0). KV v2, transit, plus collections, share, dynamic creds in later phases. For HashiCorp Vault / OpenBao use `assay.hashicorp_vault`.
--- @keywords vault, secrets, kv, transit, encrypt, decrypt, rotate, key, credential, assay-engine
--- @quickref c.kv:put(path, data, custom_md?) -> {path, version} | Store new KV version
--- @quickref c.kv:get(path, version?) -> {data, version, deleted_at, created_at} | Read latest or specific version
--- @quickref c.kv:list(prefix?) -> {entries} | List paths under a prefix
--- @quickref c.kv:meta(path) -> {latest_version, custom_md, ...} | Read path metadata
--- @quickref c.kv:delete(path, version) | Soft-delete a version
--- @quickref c.kv:destroy(path, version) | Hard-destroy a version (irreversible)
--- @quickref c.kv:undelete(path, version) | Reverse a soft-delete
--- @quickref c.transit:create(name, opts?) | Create a transit key
--- @quickref c.transit:encrypt(name, plaintext) -> ciphertext | Encrypt bytes; returns vault:vN:b64 envelope
--- @quickref c.transit:decrypt(name, ciphertext) -> plaintext | Decrypt envelope back to raw bytes
--- @quickref c.transit:rotate(name) -> {version} | Mint a new version; old versions still decrypt
--- @quickref c.transit:list() -> {keys} | List every transit key

local M = {}

--- Construct a vault client.
--- @param opts table {engine_url, admin_key?}
---   engine_url: required, e.g. "http://localhost:8080". Falls back to ASSAY_ENGINE_URL.
---   admin_key:  required for Phase 1 — sent as Authorization: Bearer.
---               Falls back to ASSAY_ADMIN_API_KEY.
function M.client(opts)
  opts = opts or {}
  local engine_url = opts.engine_url or env.get("ASSAY_ENGINE_URL")
  if not engine_url or engine_url == "" then
    error("vault: engine_url is required (or set ASSAY_ENGINE_URL)")
  end
  engine_url = engine_url:gsub("/+$", "")
  local admin_key = opts.admin_key or env.get("ASSAY_ADMIN_API_KEY")

  local function headers()
    local h = { ["Content-Type"] = "application/json" }
    if admin_key and admin_key ~= "" then
      h["Authorization"] = "Bearer " .. admin_key
    end
    return h
  end

  local function url(path_str)
    return engine_url .. "/api/v1/vault" .. path_str
  end

  local function check(resp, expected, ctx)
    if type(expected) == "number" then expected = { expected } end
    for _, code in ipairs(expected) do
      if resp.status == code then return end
    end
    local body = resp.body or ""
    error(string.format("vault: %s HTTP %d: %s", ctx, resp.status, body))
  end

  local function api_get(path_str)
    local resp = http.get(url(path_str), { headers = headers() })
    if resp.status == 404 then return nil end
    check(resp, 200, "GET " .. path_str)
    return json.parse(resp.body)
  end

  local function api_put(path_str, payload, expected)
    local resp = http.put(url(path_str), payload or {}, { headers = headers() })
    check(resp, expected or { 200, 201 }, "PUT " .. path_str)
    if resp.body and resp.body ~= "" then return json.parse(resp.body) end
    return true
  end

  local function api_post(path_str, payload, expected)
    local resp = http.post(url(path_str), payload or {}, { headers = headers() })
    check(resp, expected or { 200, 201 }, "POST " .. path_str)
    if resp.body and resp.body ~= "" then return json.parse(resp.body) end
    return true
  end

  local function api_delete(path_str)
    local resp = http.delete(url(path_str), { headers = headers() })
    check(resp, { 204 }, "DELETE " .. path_str)
    return true
  end

  -- ===== Client =====
  local c = {}

  -- ────────── KV v2 ──────────
  c.kv = {}

  function c.kv:put(path_str, data, custom_md)
    return api_put("/kv/" .. path_str, {
      data = data,
      custom_md = custom_md or {},
    }, { 201 })
  end

  function c.kv:get(path_str, version)
    local q = ""
    if version then q = "?version=" .. tostring(version) end
    return api_get("/kv/" .. path_str .. q)
  end

  function c.kv:list(prefix)
    if prefix and prefix ~= "" then
      return api_get("/kv-list/" .. prefix)
    end
    return api_get("/kv-list")
  end

  function c.kv:meta(path_str)
    return api_get("/kv-meta/" .. path_str)
  end

  function c.kv:delete(path_str, version)
    if not version then error("vault.kv:delete requires a version") end
    return api_delete("/kv/" .. path_str .. "?version=" .. tostring(version))
  end

  function c.kv:destroy(path_str, version)
    if not version then error("vault.kv:destroy requires a version") end
    return api_post("/kv-destroy/" .. path_str .. "?version=" .. tostring(version), {}, { 204 })
  end

  function c.kv:undelete(path_str, version)
    if not version then error("vault.kv:undelete requires a version") end
    return api_post("/kv-undelete/" .. path_str .. "?version=" .. tostring(version), {}, { 204 })
  end

  -- ────────── Transit ──────────
  c.transit = {}

  function c.transit:create(name, transit_opts)
    transit_opts = transit_opts or {}
    return api_post("/transit/keys/" .. name, {
      algo = transit_opts.algo,
    }, { 201 })
  end

  function c.transit:list()
    return api_get("/transit/keys")
  end

  function c.transit:rotate(name)
    return api_post("/transit/keys/" .. name .. "/rotate", {})
  end

  function c.transit:encrypt(name, plaintext)
    local body = {
      plaintext_b64 = base64.encode(plaintext),
    }
    local resp = api_post("/transit/encrypt/" .. name, body)
    return resp.ciphertext
  end

  function c.transit:decrypt(name, ciphertext)
    local body = { ciphertext = ciphertext }
    local resp = api_post("/transit/decrypt/" .. name, body)
    return base64.decode(resp.plaintext_b64)
  end

  return c
end

return M
