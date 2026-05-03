--! Compose all stubs into the opts table mount() expects.
--!
--! Used by libs/hostops/tests-lua/smoke.test.lua.

local M = {}

function M.opts(overrides)
  overrides = overrides or {}
  return {
    prefix             = overrides.prefix or "/",
    state              = overrides.state  or require("stubs.state"),
    audit              = overrides.audit  or require("stubs.audit"),
    jobs               = overrides.jobs   or require("stubs.jobs"),
    secret             = overrides.secret or require("stubs.secret"),
    brand              = overrides.brand  or require("stubs.brand"),
    engine             = overrides.engine or require("stubs.engine"),
    lib_root           = overrides.lib_root or "libs/hostops",
    catalog_paths      = overrides.catalog_paths,
    template_paths     = overrides.template_paths,
    desired_state_path = overrides.desired_state_path,
    -- Isolate the smoke test from any /etc/rustic profile already on
    -- the dev host. The dir doesn't need to exist; backups.lua's state
    -- handles a missing profile cleanly.
    backup_profile_dir   = overrides.backup_profile_dir or "/tmp/hostops-smoke-no-profile",
    extra_sidebar_links  = overrides.extra_sidebar_links,
  }
end

return M
