local ctx = require("hostops.ctx")
-- services/nspawn/jobs.lua
--
-- nspawn-specific wrapper around services.jobs. Pre-fills stages/kind so
-- callers don't have to know about the generic shape.
local M = {}

local STAGES = {
  { id = "rootfs",   label = "Bootstrapping rootfs" },
  { id = "unit",     label = "Writing nspawn unit" },
  { id = "boot",     label = "Booting container" },
  { id = "packages", label = "Installing packages" },
}

function M.start(args)
  return ctx.jobs.start({
    kind   = "machine_provision",
    target = args.name,
    name   = args.name,
    params = { template = args.template },
    stages = STAGES,
  })
end

-- Re-export the generic operations for backwards-compat.
M.get          = ctx.jobs.get
M.list         = function() return ctx.jobs.list({ kind = "machine_provision" }) end
M.active       = function() return ctx.jobs.active({ kind = "machine_provision" }) end
M.update_stage = ctx.jobs.update_stage
M.append_log   = ctx.jobs.append_log
M.complete     = ctx.jobs.complete
M.fail         = ctx.jobs.fail

return M
