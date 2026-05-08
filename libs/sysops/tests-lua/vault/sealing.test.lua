--! sysops.vault.sealing tests
--!
--! Run via:
--!   LUA_PATH='libs/?.lua;libs/?/init.lua;libs/sysops/tests-lua/?.lua;;' \
--!     assay libs/sysops/tests-lua/vault/sealing.test.lua

local sealing_mod = require("sysops.vault.sealing")

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

print("[sysops.vault.sealing]")

do
  local eng = make_engine({
    ["GET /api/v1/vault/sys/seal-status"] = {
      status = 200,
      body = json.encode({ sealed = false, threshold = 3, shares = 5, progress = 0 }),
    },
  })
  local sealing = sealing_mod.new(eng)
  local data, err = sealing.status()
  assert.eq(err, nil, "status: no error")
  assert.eq(eng.calls[1].method, "GET", "status method")
  assert.eq(eng.calls[1].path, "/api/v1/vault/sys/seal-status", "status path")
  assert.eq(data.sealed, false, "status: sealed=false")
  print("  ok status 200 unsealed")
end

do
  local eng = make_engine({
    ["GET /api/v1/vault/sys/seal-status"] = {
      status = 200,
      body = json.encode({ sealed = true, threshold = 3, shares = 5, progress = 1 }),
    },
  })
  local sealing = sealing_mod.new(eng)
  local data, err = sealing.status()
  assert.eq(err, nil, "status sealed: no error")
  assert.eq(data.sealed, true, "status: sealed=true")
  print("  ok status 200 sealed")
end

do
  local eng = make_engine({
    ["GET /api/v1/vault/sys/seal-status"] = { status = 0, body = nil },
  })
  local sealing = sealing_mod.new(eng)
  local data, err = sealing.status()
  assert.eq(data, nil, "status network fail: no data")
  assert.eq(err.status, 0, "status network fail: status 0")
  print("  ok status network failure (status=0)")
end

do
  local eng = make_engine({
    ["POST /api/v1/vault/sys/seal"] = { status = 204, body = "" },
  })
  local sealing = sealing_mod.new(eng)
  local data, err = sealing.seal()
  assert.eq(err, nil, "seal: no error")
  assert.eq(eng.calls[1].method, "POST", "seal method")
  assert.eq(eng.calls[1].path, "/api/v1/vault/sys/seal", "seal path")
  print("  ok seal 204")
end

do
  local eng = make_engine({
    ["POST /api/v1/vault/sys/unseal"] = {
      status = 200,
      body = json.encode({ sealed = false, progress = 0 }),
    },
  })
  local sealing = sealing_mod.new(eng)
  local data, err = sealing.unseal("AAABBBCCC==")
  assert.eq(err, nil, "unseal: no error")
  assert.eq(eng.calls[1].path, "/api/v1/vault/sys/unseal", "unseal path")
  assert.eq(eng.calls[1].body.key, "AAABBBCCC==", "unseal body.key")
  assert.eq(data.sealed, false, "unseal: sealed=false")
  print("  ok unseal 200")
end

do
  local eng = make_engine({
    ["POST /api/v1/vault/sys/unseal"] = { status = 503, body = "uninitialized" },
  })
  local sealing = sealing_mod.new(eng)
  local data, err = sealing.unseal("badkey")
  assert.eq(data, nil, "unseal 503: no data")
  assert.eq(err.status, 503, "unseal 503: error status")
  print("  ok unseal 503")
end

do
  local eng = make_engine({
    ["POST /api/v1/vault/sys/init"] = {
      status = 200,
      body = json.encode({ keys = { "key1", "key2", "key3" }, root_token = "root-tok" }),
    },
  })
  local sealing = sealing_mod.new(eng)
  local data, err = sealing.init(5, 3)
  assert.eq(err, nil, "init: no error")
  assert.eq(eng.calls[1].path, "/api/v1/vault/sys/init", "init path")
  assert.eq(eng.calls[1].body.secret_shares, 5, "init body.secret_shares")
  assert.eq(eng.calls[1].body.secret_threshold, 3, "init body.secret_threshold")
  assert.eq(data.root_token, "root-tok", "init root_token returned")
  print("  ok init 200")
end

do
  local eng = make_engine({
    ["POST /api/v1/vault/sys/init"] = { status = 400, body = "already initialized" },
  })
  local sealing = sealing_mod.new(eng)
  local data, err = sealing.init(5, 3)
  assert.eq(data, nil, "init 400: no data")
  assert.eq(err.status, 400, "init 400: error status")
  print("  ok init 400 already initialized")
end

print("[sysops.vault.sealing] all passed")
