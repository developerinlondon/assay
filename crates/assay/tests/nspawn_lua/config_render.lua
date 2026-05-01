-- Tests for assay.nspawn.config.render — the .nspawn INI emitter.
-- Pure-string transform, no subprocess work.
local nspawn = require("assay.nspawn")

local function check(cond, msg)
  if not cond then error(msg, 2) end
end

local function contains(s, needle)
  return s and s:find(needle, 1, true) ~= nil
end

-- ── full config: every section + every field we know about ────────────────
local body = nspawn.config.render({
  boot             = true,
  notify_ready     = true,
  private_users    = false,
  capabilities     = { "all" },
  binds            = { "/dev/kmsg", "/srv/data" },
  binds_ro         = { "/sys/module" },
  inaccessible     = { "/sys/module/apparmor" },
  virtual_ethernet = true,
  resolv_conf      = "bind-host",
  bridge           = "br0",
})
check(contains(body, "[Exec]"), "missing [Exec] section")
check(contains(body, "Boot=yes"), "boot=true should emit Boot=yes")
check(contains(body, "NotifyReady=yes"), "notify_ready emits NotifyReady=yes")
check(contains(body, "PrivateUsers=no"), "private_users=false emits PrivateUsers=no")
check(contains(body, "Capability=all"), "capabilities array → Capability=all")
check(contains(body, "[Files]"), "missing [Files] section")
check(contains(body, "Bind=/dev/kmsg"), "binds emits Bind= line")
check(contains(body, "Bind=/srv/data"), "binds repeats per element")
check(contains(body, "BindReadOnly=/sys/module"), "binds_ro emits BindReadOnly=")
check(contains(body, "Inaccessible=/sys/module/apparmor"), "inaccessible emits Inaccessible=")
check(contains(body, "[Network]"), "missing [Network] section")
check(contains(body, "VirtualEthernet=yes"), "virtual_ethernet emits VirtualEthernet=yes")
check(contains(body, "ResolvConf=bind-host"), "resolv_conf emits hyphen form (matches systemd flag)")
check(contains(body, "Bridge=br0"), "bridge emits Bridge=")

-- ── underscore form of resolv_conf normalises to hyphen ───────────────────
local body_us = nspawn.config.render({ resolv_conf = "bind_host" })
check(contains(body_us, "ResolvConf=bind-host"),
      "underscore form should emit hyphen form on the wire")

-- ── empty config produces no sections (only blank string) ─────────────────
local body_empty = nspawn.config.render({})
check(not contains(body_empty, "["), "empty config should emit no INI sections; got: " .. body_empty)

-- ── partial config: only [Exec] section ───────────────────────────────────
local body_only_exec = nspawn.config.render({ boot = true })
check(contains(body_only_exec, "[Exec]"), "only-exec missing [Exec]")
check(not contains(body_only_exec, "[Network]"), "only-exec should not emit [Network]")
check(not contains(body_only_exec, "[Files]"), "only-exec should not emit [Files]")

-- ── render must error on non-table ────────────────────────────────────────
local ok, _ = pcall(nspawn.config.render, "not a table")
check(not ok, "render(non-table) should raise")

print("config_render.lua OK")
