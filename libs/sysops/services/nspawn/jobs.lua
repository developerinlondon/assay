local ctx = require("sysops.ctx")
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

-- Re-export the generic operations. Wrapped as closures so the lookup
-- happens at call time (after mount() populates ctx.jobs), not at
-- module-load time.
M.get          = function(...) return ctx.jobs.get(...) end
M.list         = function() return ctx.jobs.list({ kind = "machine_provision" }) end
M.active       = function() return ctx.jobs.active({ kind = "machine_provision" }) end
M.update_stage = function(...) return ctx.jobs.update_stage(...) end
M.append_log   = function(...) return ctx.jobs.append_log(...) end
M.complete     = function(...) return ctx.jobs.complete(...) end
M.fail         = function(...) return ctx.jobs.fail(...) end

return M
