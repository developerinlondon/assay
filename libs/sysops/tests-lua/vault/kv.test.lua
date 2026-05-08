--! sysops.vault.kv tests
--!
--! Run via:
--!   LUA_PATH='libs/?.lua;libs/?/init.lua;libs/sysops/tests-lua/?.lua;;' \
--!     assay libs/sysops/tests-lua/vault/kv.test.lua

local kv_mod = require("sysops.vault.kv")

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

print("[sysops.vault.kv]")

do
  local eng = make_engine({
    ["GET /api/v1/vault/kv/apps/foo/db_url"] = {
      status = 200,
      body = json.encode({ data = "postgres://localhost/foo", version = 3 }),
    },
  })
  local kv = kv_mod.new(eng)
  local data, err = kv.get("apps/foo/db_url")
  assert.eq(err, nil, "get success: no error")
  assert.eq(data.data, "postgres://localhost/foo", "get success: data value")
  assert.eq(eng.calls[1].method, "GET", "get method")
  assert.eq(eng.calls[1].path, "/api/v1/vault/kv/apps/foo/db_url", "get path")
  print("  ok get 200")
end

do
  local eng = make_engine({
    ["GET /api/v1/vault/kv/apps/foo/missing"] = { status = 404, body = "" },
  })
  local kv = kv_mod.new(eng)
  local data, err = kv.get("apps/foo/missing")
  assert.eq(data, nil, "get 404: no data")
  assert.eq(err.status, 404, "get 404: error status")
  print("  ok get 404")
end

do
  local eng = make_engine({
    ["GET /api/v1/vault/kv/apps/foo/db_url"] = { status = 503, body = "vault sealed" },
  })
  local kv = kv_mod.new(eng)
  local data, err = kv.get("apps/foo/db_url")
  assert.eq(data, nil, "get 503: no data")
  assert.eq(err.status, 503, "get 503: error status")
  print("  ok get 503")
end

do
  local eng = make_engine({
    ["GET /api/v1/vault/kv/apps/foo/db_url"] = { status = 0, body = nil },
  })
  local kv = kv_mod.new(eng)
  local data, err = kv.get("apps/foo/db_url")
  assert.eq(data, nil, "get network fail: no data")
  assert.eq(err.status, 0, "get network fail: status 0")
  print("  ok get network failure (status=0)")
end

do
  local eng = make_engine({
    ["PUT /api/v1/vault/kv/apps/foo/db_url"] = {
      status = 201,
      body = json.encode({ version = 1 }),
    },
  })
  local kv = kv_mod.new(eng)
  local data, err = kv.put("apps/foo/db_url", "postgres://new/foo")
  assert.eq(err, nil, "put success: no error")
  assert.eq(eng.calls[1].method, "PUT", "put method")
  assert.eq(eng.calls[1].path, "/api/v1/vault/kv/apps/foo/db_url", "put path")
  assert.eq(eng.calls[1].body.data, "postgres://new/foo", "put body.data")
  print("  ok put 201")
end

do
  local eng = make_engine({
    ["DELETE /api/v1/vault/kv/apps/foo/db_url?version=3"] = { status = 204, body = "" },
  })
  local kv = kv_mod.new(eng)
  local data, err = kv.delete("apps/foo/db_url", 3)
  assert.eq(err, nil, "delete success: no error")
  assert.eq(eng.calls[1].method, "DELETE", "delete method")
  assert.eq(eng.calls[1].path, "/api/v1/vault/kv/apps/foo/db_url?version=3", "delete path with version")
  print("  ok delete with version 204")
end

do
  local eng = make_engine({
    ["GET /api/v1/vault/kv-list"] = {
      status = 200,
      body = json.encode({ entries = { "apps/foo/", "apps/bar/" } }),
    },
  })
  local kv = kv_mod.new(eng)
  local data, err = kv.list()
  assert.eq(err, nil, "list no-prefix: no error")
  assert.eq(eng.calls[1].path, "/api/v1/vault/kv-list", "list no-prefix path")
  print("  ok list no prefix")
end

do
  local eng = make_engine({
    ["GET /api/v1/vault/kv-list/apps/foo"] = {
      status = 200,
      body = json.encode({ entries = { "apps/foo/db_url" } }),
    },
  })
  local kv = kv_mod.new(eng)
  local data, err = kv.list("apps/foo")
  assert.eq(err, nil, "list prefix: no error")
  assert.eq(eng.calls[1].path, "/api/v1/vault/kv-list/apps/foo", "list prefix path")
  print("  ok list with prefix")
end

do
  local eng = make_engine({
    ["GET /api/v1/vault/kv-meta/apps/foo/db_url"] = {
      status = 200,
      body = json.encode({ current_version = 3, versions = {} }),
    },
  })
  local kv = kv_mod.new(eng)
  local data, err = kv.meta("apps/foo/db_url")
  assert.eq(err, nil, "meta: no error")
  assert.eq(eng.calls[1].path, "/api/v1/vault/kv-meta/apps/foo/db_url", "meta path")
  print("  ok meta")
end

do
  local eng = make_engine({
    ["POST /api/v1/vault/kv-destroy/apps/foo/db_url"] = { status = 204, body = "" },
  })
  local kv = kv_mod.new(eng)
  local data, err = kv.destroy("apps/foo/db_url", { 1, 2 })
  assert.eq(err, nil, "destroy: no error")
  assert.eq(eng.calls[1].method, "POST", "destroy method")
  assert.eq(eng.calls[1].body.versions[1], 1, "destroy versions[1]")
  print("  ok destroy")
end

do
  local eng = make_engine({
    ["POST /api/v1/vault/kv-undelete/apps/foo/db_url"] = { status = 204, body = "" },
  })
  local kv = kv_mod.new(eng)
  local data, err = kv.undelete("apps/foo/db_url", { 2 })
  assert.eq(err, nil, "undelete: no error")
  assert.eq(eng.calls[1].method, "POST", "undelete method")
  print("  ok undelete")
end

do
  -- Spaces and special chars within a segment are encoded; slashes remain path separators.
  local eng = make_engine({
    ["GET /api/v1/vault/kv/scope%20name/key%3Dname"] = {
      status = 200,
      body = json.encode({ data = "encoded-val" }),
    },
  })
  local kv = kv_mod.new(eng)
  local data, err = kv.get("scope name/key=name")
  assert.eq(err, nil, "encoded path: no error")
  assert.eq(eng.calls[1].path, "/api/v1/vault/kv/scope%20name/key%3Dname", "encoded path segments")
  print("  ok path segments are percent-encoded")
end

print("[sysops.vault.kv] all passed")
