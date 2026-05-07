--! sysops vault tests - engine-vault backed host secret store.
--!
--! Run via:
--!   LUA_PATH='libs/?.lua;libs/?/init.lua;libs/sysops/tests-lua/?.lua;;' \
--!     assay libs/sysops/tests-lua/vault.test.lua

local vault = require("sysops.vault")

local function reset_runtime(vars, handler, file_map)
  local files = file_map or {}

  env = {
    get = function(key) return vars[key] end,
  }

  fs = {
    exists = function(path) return files[path] ~= nil end,
    read = function(path)
      if files[path] == nil then error("missing file: " .. path) end
      return files[path]
    end,
  }

  log = {
    warn = function(_) end,
  }

  http = {
    request = vars.NO_HTTP_REQUEST and nil or function(opts) return handler(opts) end,
    get = function(url, opts)
      return handler({
        method = "GET",
        url = url,
        headers = opts and opts.headers or {},
      })
    end,
    put = function(url, body, opts)
      return handler({
        method = "PUT",
        url = url,
        headers = opts and opts.headers or {},
        body = json.encode(body),
      })
    end,
    delete = function(url, opts)
      return handler({
        method = "DELETE",
        url = url,
        headers = opts and opts.headers or {},
      })
    end,
  }
end

print("[sysops.vault]")

do
  local seen = nil
  reset_runtime({
    ENGINE_URL = "http://engine.local/",
    APP_ADMIN_API_KEYS = "app-token",
  }, function(opts)
    seen = opts
    return {
      status = 200,
      body = json.encode({ path = "hostops/host/password", version = 7, data = "from-vault" }),
      headers = { ["content-type"] = "application/json" },
    }
  end)

  local store = vault.secret_store({
    app = "hostops",
    admin_key_envs = { "APP_ADMIN_API_KEYS" },
  })

  assert.eq(store.read("host", "password"), "from-vault", "read returns engine KV data")
  assert.eq(seen.method, "GET", "read method")
  assert.eq(seen.url, "http://engine.local/api/v1/vault/kv/hostops/host/password", "read URL")
  assert.eq(seen.headers.Authorization, "Bearer app-token", "read auth header")
  print("  ok read uses engine KV endpoint")
end

do
  local seen = nil
  reset_runtime({
    ENGINE_URL = "http://engine.local",
    ENGINE_ADMIN_KEY = "engine-token",
    NO_HTTP_REQUEST = true,
  }, function(opts)
    seen = opts
    return {
      status = 201,
      body = json.encode({ path = "shared/host/password", version = 1 }),
      headers = { ["content-type"] = "application/json" },
    }
  end)

  local store = vault.secret_store({
    app = "hostops",
    kv_prefix = "shared",
  })
  local ok, err = store.write("host", "password", "stored-secret")

  assert.eq(ok, true, "write ok")
  assert.eq(err, nil, "write err")
  assert.eq(seen.method, "PUT", "write method")
  assert.eq(seen.url, "http://engine.local/api/v1/vault/kv/shared/host/password", "write URL")
  assert.eq(seen.headers.Authorization, "Bearer engine-token", "write auth header")
  local body = json.parse(seen.body)
  assert.eq(body.data, "stored-secret", "write payload data")
  assert.eq(body.custom_md.app, "hostops", "write metadata app")
  assert.eq(body.custom_md.scope, "host", "write metadata scope")
  assert.eq(body.custom_md.key, "password", "write metadata key")
  print("  ok write persists through engine KV")
end

do
  local calls = {}
  reset_runtime({
    ENGINE_URL = "http://engine.local",
    ASSAY_ADMIN_KEY = "assay-token",
  }, function(opts)
    table.insert(calls, opts)
    if #calls == 1 then
      return {
        status = 200,
        body = json.encode({ path = "hostops/host/password", version = 3, data = "stored-secret" }),
        headers = { ["content-type"] = "application/json" },
      }
    end
    return { status = 204, body = "", headers = {} }
  end)

  local store = vault.secret_store({ app = "hostops" })
  local ok, err = store.delete("host", "password")

  assert.eq(ok, true, "delete ok")
  assert.eq(err, nil, "delete err")
  assert.eq(calls[1].method, "GET", "delete reads current version first")
  assert.eq(calls[2].method, "DELETE", "delete method")
  assert.eq(
    calls[2].url,
    "http://engine.local/api/v1/vault/kv/hostops/host/password?version=3",
    "delete URL"
  )
  assert.eq(calls[2].headers.Authorization, "Bearer assay-token", "delete auth header")
  print("  ok delete soft-deletes latest engine KV version")
end

do
  reset_runtime({}, function()
    error("engine should not be called without ENGINE_URL", 2)
  end, {
    ["/etc/rustic/host.password"] = "fallback-password\n",
  })

  local store = vault.secret_store({ app = "hostops" })
  assert.eq(store.read("host", "password"), "fallback-password", "rustic file fallback")
  local available, status = store.available()
  assert.eq(available, false, "available without engine")
  assert.not_nil(status.error, "available error")
  print("  ok rustic fallback remains read-only")
end

do
  local seen = nil
  reset_runtime({
    ENGINE_URL = "http://engine.local",
    VAULT_KV_PREFIX = "env-prefix",
  }, function(opts)
    seen = opts
    return {
      status = 200,
      body = json.encode({ path = "env-prefix/scope%20name/key%2Fname", data = "encoded" }),
      headers = { ["content-type"] = "application/json" },
    }
  end)

  local store = vault.secret_store({ app = "hostops" })
  assert.eq(store.read("scope name", "key/name"), "encoded", "encoded path read")
  assert.eq(
    seen.url,
    "http://engine.local/api/v1/vault/kv/env-prefix/scope%20name/key%2Fname",
    "encoded URL"
  )
  print("  ok prefixes and path segments are encoded")
end

print("[sysops.vault] all passed")
