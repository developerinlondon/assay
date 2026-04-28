--- @module assay.engine.vault
--- @description Lua client for assay-engine's vault module (plan 17 / v0.3.0) mounted at `/api/v1/vault/*`. KV v2, transit, plus collections / share / dynamic-creds / sealing surfaces. For HashiCorp Vault / OpenBao use `assay.hashicorp.vault`.
--- @keywords vault, secrets, kv, transit, encrypt, decrypt, encryption, decryption, rotate, rotation, password, key, share, sealing, lease, credential, biscuit, kdf, assay-engine
--- @quickref vault.client(opts) -> client | Build a vault client (engine_url + optional api_key)
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
--- @quickref c.share:mint(opts) -> {token, revocation_ids, expires_at} | Mint a biscuit share link
--- @quickref c.share:redeem(token) -> grant | Redeem a share token (public surface)
--- @quickref c.share:revoke(revocation_id, reason?) | Revoke a token by block id
--- @quickref c.dynamic:lease(provider, role, ttl_secs?) -> lease | Issue dynamic credentials
--- @quickref c.dynamic:list(provider?) -> {leases} | List leases (optional provider filter)
--- @quickref c.dynamic:revoke(lease_id) | Revoke a lease + ask the provider to clean up
--- @quickref c.sys:status() -> {sealed, method, kid, ...} | Read seal status
--- @quickref c.sys:seal() | Seal the vault — every KV/transit op then 503s
--- @quickref c.sys:unseal(share_b64) -> status | Submit one Shamir share

local M = {}

local function trim_slash(s) return (s or ""):gsub("/+$", "") end

--- Build a vault client.
---
--- opts:
---   engine_url (string, required)  base URL of the assay-engine.
---                                  Falls back to ASSAY_ENGINE_URL.
---   api_key    (string, optional)  admin bearer token. Falls back
---                                  to ASSAY_ADMIN_KEY. Same env var
---                                  the other assay.engine.* clients
---                                  read so an operator only sets it
---                                  once.
function M.client(opts)
  opts = opts or {}
  local engine_url = trim_slash(opts.engine_url or env.get("ASSAY_ENGINE_URL") or "")
  if engine_url == "" then
    error("assay.engine.vault: engine_url required (or set ASSAY_ENGINE_URL)")
  end
  local api_key = opts.api_key or env.get("ASSAY_ADMIN_KEY")

  local function headers()
    local h = { ["Content-Type"] = "application/json" }
    if api_key and api_key ~= "" then
      h["Authorization"] = "Bearer " .. api_key
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
    error(string.format("assay.engine.vault: %s HTTP %d: %s", ctx, resp.status, body))
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
    if not version then error("assay.engine.vault.kv:delete requires a version") end
    return api_delete("/kv/" .. path_str .. "?version=" .. tostring(version))
  end

  function c.kv:destroy(path_str, version)
    if not version then error("assay.engine.vault.kv:destroy requires a version") end
    return api_post("/kv-destroy/" .. path_str .. "?version=" .. tostring(version), {}, { 204 })
  end

  function c.kv:undelete(path_str, version)
    if not version then error("assay.engine.vault.kv:undelete requires a version") end
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
    local body = { plaintext_b64 = base64.encode(plaintext) }
    local resp = api_post("/transit/encrypt/" .. name, body)
    return resp.ciphertext
  end

  function c.transit:decrypt(name, ciphertext)
    local body = { ciphertext = ciphertext }
    local resp = api_post("/transit/decrypt/" .. name, body)
    return base64.decode(resp.plaintext_b64)
  end

  -- ────────── Biscuit-share (Phase 4) ──────────
  c.share = {}

  function c.share:mint(share_opts)
    return api_post("/share", {
      target_kind = share_opts.target_kind,
      target_id   = share_opts.target_id,
      ttl_secs    = share_opts.ttl_secs or 3600,
      max_ip_cidr = share_opts.max_ip_cidr,
      max_uses    = share_opts.max_uses,
    }, { 201 })
  end

  function c.share:redeem(token)
    -- Public surface; admin auth not required.
    return api_get("/share/" .. token)
  end

  function c.share:revoke(revocation_id, reason)
    return api_post("/share/revoke", {
      revocation_id = revocation_id,
      reason = reason or "",
    }, { 204 })
  end

  -- ────────── Dynamic credentials (Phase 5) ──────────
  c.dynamic = {}

  function c.dynamic:lease(provider, role, ttl_secs)
    return api_post(
      "/dynamic/" .. provider .. "/" .. role .. "/lease",
      { ttl_secs = ttl_secs or 3600 },
      { 201 }
    )
  end

  function c.dynamic:list(provider)
    if provider and provider ~= "" then
      return api_get("/dynamic/leases?provider=" .. provider)
    end
    return api_get("/dynamic/leases")
  end

  function c.dynamic:revoke(lease_id)
    return api_delete("/dynamic/leases/" .. lease_id)
  end

  -- ────────── Sealing (Phase 2) ──────────
  c.sys = {}

  function c.sys:status()
    return api_get("/sys/seal-status")
  end

  function c.sys:seal()
    local resp = http.post(url("/sys/seal"), {}, { headers = headers() })
    check(resp, { 204 }, "POST /sys/seal")
    return true
  end

  function c.sys:unseal(share_b64)
    return api_post("/sys/unseal", { share_b64 = share_b64 })
  end

  function c.sys:init(threshold, shares_count)
    return api_post("/sys/init", {
      threshold = threshold,
      shares_count = shares_count,
    }, { 201 })
  end

  return c
end

return M
