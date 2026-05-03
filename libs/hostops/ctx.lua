--! Runtime context for hostops pages and api handlers.
--!
--! `mount.lua` populates this module's fields when the consumer app
--! calls `hostops.mount(routes, opts)`. Pages and api handlers read
--! fields at request time:
--!
--!   local ctx = require("hostops.ctx")
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

return {
  prefix = "/",
  url    = function(p) return p end,
  state  = nil,
  audit  = nil,
  jobs   = nil,
  secret = nil,
  brand  = nil,
  engine = nil,
}
