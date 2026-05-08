--! sysops.vault.transit tests
--!
--! Run via:
--!   LUA_PATH='libs/?.lua;libs/?/init.lua;libs/sysops/tests-lua/?.lua;;' \
--!     assay libs/sysops/tests-lua/vault/transit.test.lua

local transit_mod = require("sysops.vault.transit")

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

print("[sysops.vault.transit]")

do
  local eng = make_engine({
    ["GET /api/v1/vault/transit/keys"] = {
      status = 200,
      body = json.encode({ keys = { "master", "backup" } }),
    },
  })
  local transit = transit_mod.new(eng)
  local data, err = transit.keys()
  assert.eq(err, nil, "keys: no error")
  assert.eq(eng.calls[1].path, "/api/v1/vault/transit/keys", "keys path")
  assert.eq(data.keys[1], "master", "keys first entry")
  print("  ok keys 200")
end

do
  local eng = make_engine({
    ["GET /api/v1/vault/transit/keys"] = { status = 503, body = "module disabled" },
  })
  local transit = transit_mod.new(eng)
  local data, err = transit.keys()
  assert.eq(data, nil, "keys 503: no data")
  assert.eq(err.status, 503, "keys 503: error status")
  print("  ok keys 503")
end

do
  local eng = make_engine({
    ["POST /api/v1/vault/transit/keys/master"] = {
      status = 201,
      body = json.encode({ name = "master", type = "aes256-gcm96" }),
    },
  })
  local transit = transit_mod.new(eng)
  local data, err = transit.create("master", "aes256-gcm96")
  assert.eq(err, nil, "create: no error")
  assert.eq(eng.calls[1].method, "POST", "create method")
  assert.eq(eng.calls[1].path, "/api/v1/vault/transit/keys/master", "create path")
  assert.eq(eng.calls[1].body.type, "aes256-gcm96", "create body.type")
  print("  ok create 201")
end

do
  local eng = make_engine({
    ["POST /api/v1/vault/transit/keys/master/rotate"] = {
      status = 200,
      body = json.encode({ version = 2 }),
    },
  })
  local transit = transit_mod.new(eng)
  local data, err = transit.rotate("master")
  assert.eq(err, nil, "rotate: no error")
  assert.eq(eng.calls[1].path, "/api/v1/vault/transit/keys/master/rotate", "rotate path")
  print("  ok rotate 200")
end

do
  local eng = make_engine({
    ["POST /api/v1/vault/transit/encrypt/master"] = {
      status = 200,
      body = json.encode({ ciphertext = "vault:v1:abc123" }),
    },
  })
  local transit = transit_mod.new(eng)
  local data, err = transit.encrypt("master", "hello world")
  assert.eq(err, nil, "encrypt: no error")
  assert.eq(eng.calls[1].path, "/api/v1/vault/transit/encrypt/master", "encrypt path")
  assert.eq(eng.calls[1].body.plaintext, "hello world", "encrypt body.plaintext")
  assert.eq(data.ciphertext, "vault:v1:abc123", "encrypt ciphertext returned")
  print("  ok encrypt 200")
end

do
  local eng = make_engine({
    ["POST /api/v1/vault/transit/decrypt/master"] = {
      status = 200,
      body = json.encode({ plaintext = "hello world" }),
    },
  })
  local transit = transit_mod.new(eng)
  local data, err = transit.decrypt("master", "vault:v1:abc123")
  assert.eq(err, nil, "decrypt: no error")
  assert.eq(eng.calls[1].path, "/api/v1/vault/transit/decrypt/master", "decrypt path")
  assert.eq(eng.calls[1].body.ciphertext, "vault:v1:abc123", "decrypt body.ciphertext")
  assert.eq(data.plaintext, "hello world", "decrypt plaintext returned")
  print("  ok decrypt 200")
end

do
  local eng = make_engine({
    ["POST /api/v1/vault/transit/encrypt/master"] = { status = 0, body = nil },
  })
  local transit = transit_mod.new(eng)
  local data, err = transit.encrypt("master", "hello")
  assert.eq(data, nil, "encrypt network fail: no data")
  assert.eq(err.status, 0, "encrypt network fail: status 0")
  print("  ok encrypt network failure (status=0)")
end

do
  local eng = make_engine({
    ["POST /api/v1/vault/transit/keys/my%20key"] = {
      status = 201,
      body = json.encode({ name = "my key" }),
    },
  })
  local transit = transit_mod.new(eng)
  local data, err = transit.create("my key", "aes256-gcm96")
  assert.eq(err, nil, "create encoded name: no error")
  assert.eq(eng.calls[1].path, "/api/v1/vault/transit/keys/my%20key", "create encoded path")
  print("  ok key name is percent-encoded")
end

print("[sysops.vault.transit] all passed")
