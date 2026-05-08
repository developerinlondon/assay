--! sysops.vault.dynamic tests
--!
--! Run via:
--!   LUA_PATH='libs/?.lua;libs/?/init.lua;libs/sysops/tests-lua/?.lua;;' \
--!     assay libs/sysops/tests-lua/vault/dynamic.test.lua

local dynamic_mod = require("sysops.vault.dynamic")

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

print("[sysops.vault.dynamic]")

do
  local eng = make_engine({
    ["POST /api/v1/vault/dynamic/postgres/readonly/lease"] = {
      status = 200,
      body = json.encode({ id = "lease-abc", username = "dyn_abc", password = "pw123" }),
    },
  })
  local dynamic = dynamic_mod.new(eng)
  local data, err = dynamic.lease("postgres", "readonly")
  assert.eq(err, nil, "lease: no error")
  assert.eq(eng.calls[1].method, "POST", "lease method")
  assert.eq(eng.calls[1].path, "/api/v1/vault/dynamic/postgres/readonly/lease", "lease path")
  assert.eq(data.id, "lease-abc", "lease id returned")
  print("  ok lease 200")
end

do
  local eng = make_engine({
    ["POST /api/v1/vault/dynamic/postgres/readonly/lease"] = { status = 503, body = "disabled" },
  })
  local dynamic = dynamic_mod.new(eng)
  local data, err = dynamic.lease("postgres", "readonly")
  assert.eq(data, nil, "lease 503: no data")
  assert.eq(err.status, 503, "lease 503: error status")
  print("  ok lease 503")
end

do
  local eng = make_engine({
    ["POST /api/v1/vault/dynamic/postgres/readonly/lease"] = { status = 0, body = nil },
  })
  local dynamic = dynamic_mod.new(eng)
  local data, err = dynamic.lease("postgres", "readonly")
  assert.eq(data, nil, "lease network fail: no data")
  assert.eq(err.status, 0, "lease network fail: status 0")
  print("  ok lease network failure (status=0)")
end

do
  local eng = make_engine({
    ["GET /api/v1/vault/dynamic/leases"] = {
      status = 200,
      body = json.encode({ leases = { { id = "lease-1" }, { id = "lease-2" } } }),
    },
  })
  local dynamic = dynamic_mod.new(eng)
  local data, err = dynamic.list()
  assert.eq(err, nil, "list no-provider: no error")
  assert.eq(eng.calls[1].path, "/api/v1/vault/dynamic/leases", "list no-provider path")
  print("  ok list no provider")
end

do
  local eng = make_engine({
    ["GET /api/v1/vault/dynamic/leases?provider=postgres"] = {
      status = 200,
      body = json.encode({ leases = { { id = "lease-1" } } }),
    },
  })
  local dynamic = dynamic_mod.new(eng)
  local data, err = dynamic.list("postgres")
  assert.eq(err, nil, "list provider: no error")
  assert.eq(eng.calls[1].path, "/api/v1/vault/dynamic/leases?provider=postgres", "list provider path")
  print("  ok list with provider filter")
end

do
  local eng = make_engine({
    ["DELETE /api/v1/vault/dynamic/leases/lease-abc"] = { status = 204, body = "" },
  })
  local dynamic = dynamic_mod.new(eng)
  local data, err = dynamic.revoke("lease-abc")
  assert.eq(err, nil, "revoke: no error")
  assert.eq(eng.calls[1].method, "DELETE", "revoke method")
  assert.eq(eng.calls[1].path, "/api/v1/vault/dynamic/leases/lease-abc", "revoke path")
  print("  ok revoke 204")
end

do
  local eng = make_engine({
    ["DELETE /api/v1/vault/dynamic/leases/lease-abc"] = { status = 404, body = "not found" },
  })
  local dynamic = dynamic_mod.new(eng)
  local data, err = dynamic.revoke("lease-abc")
  assert.eq(data, nil, "revoke 404: no data")
  assert.eq(err.status, 404, "revoke 404: error status")
  print("  ok revoke 404")
end

do
  local eng = make_engine({
    ["POST /api/v1/vault/dynamic/my%20db/read%2Fonly/lease"] = {
      status = 200,
      body = json.encode({ id = "lease-enc" }),
    },
  })
  local dynamic = dynamic_mod.new(eng)
  local data, err = dynamic.lease("my db", "read/only")
  assert.eq(err, nil, "lease encoded: no error")
  assert.eq(
    eng.calls[1].path,
    "/api/v1/vault/dynamic/my%20db/read%2Fonly/lease",
    "lease encoded path"
  )
  print("  ok provider and role are percent-encoded")
end

print("[sysops.vault.dynamic] all passed")
