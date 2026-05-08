--! sysops.vault - Assay Engine Vault SDK + legacy secret_store helper.
--!
--! Backwards-compat (0.1.4): `vault.secret_store(opts)` still works unchanged.
--!
--! New (0.1.5): `vault.new(engine)` returns a namespaced SDK client.
--!   local sdk = require("sysops.vault").new(ctx.engine)
--!   local val, err = sdk.kv.get("apps/foo/db_url")

local secret_store = require("sysops.vault.secret_store")

local M = {}

-- Backwards-compat: existing 0.1.4 callers using `vault.secret_store(opts)` still work.
M.secret_store = secret_store.secret_store

-- New 0.1.5 SDK aggregator. Pass the engine HTTP client (e.g. ctx.engine).
function M.new(engine)
  return {
    kv          = require("sysops.vault.kv").new(engine),
    transit     = require("sysops.vault.transit").new(engine),
    sealing     = require("sysops.vault.sealing").new(engine),
    dynamic     = require("sysops.vault.dynamic").new(engine),
    share       = require("sysops.vault.share").new(engine),
    collections = require("sysops.vault.collections").new(engine),
    me          = require("sysops.vault.me").new(engine),
  }
end

return M
