--! sysops.vault.share tests
--!
--! Run via:
--!   LUA_PATH='libs/?.lua;libs/?/init.lua;libs/sysops/tests-lua/?.lua;;' \
--!     assay libs/sysops/tests-lua/vault/share.test.lua

local share_mod = require("sysops.vault.share")

local function make_engine(responses)
  local calls = {}
  local eng = {}
  eng.calls = calls

  local function do_call(method, path, body)
    table.insert(calls, { method = method, path = path, body = body })
    local key = method .. " " .. path
    if responses[key] then return responses[key] end
    return { status = 200, body = json.encode({}) }
  end

  function eng.get(path)        return do_call("GET",    path) end
  function eng.post(path, body) return do_call("POST",   path, body) end
  function eng.put(path, body)  return do_call("PUT",    path, body) end
  function eng.delete(path)     return do_call("DELETE", path) end

  return eng
end

print("[sysops.vault.share]")

do
  local eng = make_engine({
    ["POST /api/v1/vault/share"] = {
      status = 201,
      body = json.encode({ token = "tok-abc123", expires_at = "2026-06-01T00:00:00Z" }),
    },
  })
  local share = share_mod.new(eng)
  local data, err = share.mint({ ttl = "24h", secret = "my-secret" })
  assert.eq(err, nil, "mint: no error")
  assert.eq(eng.calls[1].method, "POST", "mint method")
  assert.eq(eng.calls[1].path, "/api/v1/vault/share", "mint path")
  assert.eq(eng.calls[1].body.ttl, "24h", "mint body.ttl")
  assert.eq(data.token, "tok-abc123", "mint token returned")
  print("  ok mint 201")
end

do
  local eng = make_engine({
    ["POST /api/v1/vault/share"] = { status = 503, body = "module disabled" },
  })
  local share = share_mod.new(eng)
  local data, err = share.mint({})
  assert.eq(data, nil, "mint 503: no data")
  assert.eq(err.status, 503, "mint 503: error status")
  print("  ok mint 503")
end

do
  local eng = make_engine({
    ["POST /api/v1/vault/share"] = { status = 0, body = nil },
  })
  local share = share_mod.new(eng)
  local data, err = share.mint({})
  assert.eq(data, nil, "mint network fail: no data")
  assert.eq(err.status, 0, "mint network fail: status 0")
  print("  ok mint network failure (status=0)")
end

do
  local eng = make_engine({
    ["GET /api/v1/vault/share/tok-abc123"] = {
      status = 200,
      body = json.encode({ secret = "my-secret", redeemed = false }),
    },
  })
  local share = share_mod.new(eng)
  local data, err = share.redeem("tok-abc123")
  assert.eq(err, nil, "redeem: no error")
  assert.eq(eng.calls[1].method, "GET", "redeem method")
  assert.eq(eng.calls[1].path, "/api/v1/vault/share/tok-abc123", "redeem path")
  assert.eq(data.secret, "my-secret", "redeem secret returned")
  print("  ok redeem 200")
end

do
  local eng = make_engine({
    ["GET /api/v1/vault/share/expired-tok"] = { status = 404, body = "not found" },
  })
  local share = share_mod.new(eng)
  local data, err = share.redeem("expired-tok")
  assert.eq(data, nil, "redeem 404: no data")
  assert.eq(err.status, 404, "redeem 404: error status")
  print("  ok redeem 404 expired token")
end

do
  local eng = make_engine({
    ["POST /api/v1/vault/share/revoke"] = { status = 204, body = "" },
  })
  local share = share_mod.new(eng)
  local data, err = share.revoke("tok-abc123")
  assert.eq(err, nil, "revoke: no error")
  assert.eq(eng.calls[1].method, "POST", "revoke method")
  assert.eq(eng.calls[1].path, "/api/v1/vault/share/revoke", "revoke path")
  assert.eq(eng.calls[1].body.token, "tok-abc123", "revoke body.token")
  print("  ok revoke 204")
end

do
  local eng = make_engine({
    ["POST /api/v1/vault/share/revoke"] = { status = 404, body = "not found" },
  })
  local share = share_mod.new(eng)
  local data, err = share.revoke("gone-tok")
  assert.eq(data, nil, "revoke 404: no data")
  assert.eq(err.status, 404, "revoke 404: error status")
  print("  ok revoke 404")
end

print("[sysops.vault.share] all passed")
