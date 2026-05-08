--! sysops.auth - SDK aggregator for assay-engine auth admin surfaces.
--!
--! Usage:
--!   local auth = require("sysops.auth").new(engine)
--!   local users, err = auth.users.list({ search = "alice" })
--!   local ok, err    = auth.zanzibar.check("user:alice", "viewer", "doc:foo")

local M = {}

function M.new(engine)
  return {
    session  = require("sysops.auth.session").new(engine),
    users    = require("sysops.auth.users").new(engine),
    sessions = require("sysops.auth.sessions").new(engine),
    oidc     = require("sysops.auth.oidc").new(engine),
    biscuit  = require("sysops.auth.biscuit").new(engine),
    audit    = require("sysops.auth.audit").new(engine),
    zanzibar = require("sysops.auth.zanzibar").new(engine),
  }
end

return M
