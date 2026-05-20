--! Runtime context for sysops pages and api handlers.
--!
--! `mount.lua` populates this module's fields when the consumer app
--! calls `sysops.mount(routes, opts)`. Pages and api handlers read
--! fields at request time:
--!
--!   local ctx = require("sysops.ctx")
--!   function M.dashboard(req)
--!     local m = ctx.state.machines()
--!     ...
--!   end
--!
--! The require-time table is cached by Lua's module system, so all
--! callers see the same shared instance — mount() mutates this table
--! and every page sees the updated values from then on.
--!
--! Required fields (set by mount()):
--!   prefix  string                 -- mount prefix, e.g. "/host"
--!   url     function(path)→string  -- prefix-safe URL builder
--!   state   table                  -- machine/disk/proc state
--!   audit   table                  -- audit-log writer
--!   jobs    table                  -- job/task tracker
--!   secret  table                  -- secret-store reader
--!   brand   table                  -- brand pack (logo/colors/strings)
--!   engine  table                  -- HTTP wrapper to engine sidecar
--!
--! Optional package-management config (set by mount() from opts when
--! provided; nil-safe defaults are honoured by `services/pkg_view.lua`):
--!   catalog_paths       list of catalog directories
--!   template_paths      list of template directories
--!   desired_state_path  file path for the persisted desired-state JSON
--!
--! Optional 0.1.5 fields (set by mount() from opts when provided):
--!   active_modules      list of module names ("auth", "vault") that opt
--!                       the consumer into the new sidebar links + page
--!                       routes. Empty / nil = legacy 0.1.4 behaviour
--!                       (host-ops only, link to engine SPA).
--!   engine_admin_key    bearer token for admin-scoped engine calls
--!                       (auth/admin/*, vault/sys/*). Used by SDK
--!                       modules — page handlers don't read it directly.

return {
  prefix              = "/",
  url                 = function(p) return p end,
  lib_root            = ".",
  state               = nil,
  audit               = nil,
  jobs                = nil,
  secret              = nil,
  brand               = nil,
  engine              = nil,
  catalog_paths       = nil,
  template_paths      = nil,
  desired_state_path  = nil,
  backup_profile_dir  = nil,
  engine_base_url     = nil,
  extra_sidebar_links = nil,
  active_modules      = {},
  engine_admin_key    = nil,
  -- Auth-gateway 0.2.0 (set by mount.lua when opts.oidc is provided).
  -- nil otherwise → backward-compat path skips auth routes.
  oidc_client          = nil, -- sysops.oidc.new(opts.oidc)
  session_signer       = nil, -- sysops.session.new(opts.session)
  session_store        = nil, -- sysops.session.store_new()
  gateway_admin_bearer = nil, -- engine admin bearer the gateway injects
  authz_require_admin  = false, -- v1: cookie alone is enough; flip to true once
                                -- a zanzibar_check is wired so /api/v1/engine/*
                                -- requires user:<sub> to hold an admin tuple.
  zanzibar_check       = nil, -- function(sub) -> bool (truthy = admin)
}
