-- services/nspawn/jobs.lua
--
-- nspawn-specific wrapper around services.jobs. Pre-fills stages/kind so
-- callers don't have to know about the generic shape.

local jobs = require("services.jobs")

local M = {}

local STAGES = {
  { id = "rootfs",   label = "Bootstrapping rootfs" },
  { id = "unit",     label = "Writing nspawn unit" },
  { id = "boot",     label = "Booting container" },
  { id = "packages", label = "Installing packages" },
}

function M.start(args)
  return jobs.start({
    kind   = "machine_provision",
    target = args.name,
    name   = args.name,
    params = { template = args.template },
    stages = STAGES,
  })
end

-- Re-export the generic operations for backwards-compat.
M.get          = jobs.get
M.list         = function() return jobs.list({ kind = "machine_provision" }) end
M.active       = function() return jobs.active({ kind = "machine_provision" }) end
M.update_stage = jobs.update_stage
M.append_log   = jobs.append_log
M.complete     = jobs.complete
M.fail         = jobs.fail

return M
