-- services/nspawn/provision.lua
--
-- Thin product-side wrapper around assay.nspawn.provision.
--
-- Responsibilities (knowhere-specific, not framework):
--   - Load a machine_templates/<id>.toml profile via the existing pkg.templates loader
--   - Validate caller-supplied machine name
--   - Emit audit events
--   - After successful provision: seed the package manager's desired_state
--     for the new machine and trigger pkgs.reconcile() so the template's
--     packages get installed on first boot
--   - Surface a flat result shape to the API layer
--
-- The assay.nspawn module owns the privileged work (machinectl pull, .nspawn
-- file write, systemctl enable+start, wait-for-ready). This module does not
-- shell out directly.

local nspawn         = require("assay.nspawn")
local pkgs           = require("services.host.packages")
local network_bridge = require("services.host.network_bridge")
local resources      = require("services.nspawn.resources")

local ctx = require("hostops.ctx")
local M = {}

-- Load both built-in and operator-overlay machine_templates into a single
-- map by id. Mirrors the layered-loader pattern used by services.host.packages
-- for the package catalog.
local BUILTIN_TEMPLATES_DIR  = "machine_templates"
local OPERATOR_TEMPLATES_DIR = "/etc/knowhere/machine_templates.d"

local function load_templates()
  local catalog = pkgs.catalog()
  return pkgs.templates(catalog.entries)
end

-- Resolve effective resource limits from operator input + template defaults.
-- Form-supplied values win; missing/blank values fall back to the template's
-- [template.resources] defaults; missing-on-both = unlimited (nil).
--
-- Returns (cpu_cores, memory_gb) — either may be nil.
local function resolve_resources(args, tpl)
  local function num_or_nil(v)
    if v == nil or v == "" then return nil end
    local n = tonumber(v)
    if not n or n <= 0 then return nil end
    return n
  end

  local tpl_res = (tpl and tpl.resources) or {}
  local cpu = num_or_nil(args.cpu_cores) or num_or_nil(tpl_res.cpu_cores)
  local mem = num_or_nil(args.memory_gb) or num_or_nil(tpl_res.memory_gb)
  return cpu, mem
end

--- Provision a new nspawn machine from a template id.
---
--- Args:
---   args.name        machine name (validated against [A-Za-z0-9._-]+)
---   args.template    template id (from machine_templates/*.toml)
---   args.actor       audit actor (defaults to "system")
---   args.cpu_cores   optional CPU limit (overrides template default)
---   args.memory_gb   optional memory limit (overrides template default)
---
--- Returns:
---   { ok = true,  name, template, packages_seeded, install_result, resources }
---   { ok = false, error = "..." }
function M.provision(args)
  args = args or {}
  local actor = args.actor or "system"
  local name  = args.name
  local tmpl_id = args.template

  if type(name) ~= "string" or name == "" then
    return { ok = false, error = "name required" }
  end
  -- Defer charset validation to assay.nspawn; it raises with a clear message.
  if type(tmpl_id) ~= "string" or tmpl_id == "" then
    return { ok = false, error = "template required" }
  end

  local tpls = load_templates()
  local tpl  = tpls.entries[tmpl_id]
  if not tpl then
    return { ok = false, error = "unknown template: " .. tmpl_id }
  end
  if not (tpl.rootfs and tpl.nspawn) then
    return {
      ok = false,
      error = "template '" .. tmpl_id .. "' has no [template.rootfs] / [template.nspawn] " ..
              "sections — it's a packages-only profile, not a provisioning recipe",
    }
  end

  ctx.audit.append({
    actor  = actor,
    action = "machine.provision.start",
    target = name,
    meta   = { template = tmpl_id },
  })

  -- Idempotent bridge setup. If the template uses Bridge=<name>, ensure the
  -- bridge is present on the host (config files, networkctl reload, iptables
  -- FORWARD rules) before nspawn tries to attach the new container's veth
  -- to it. No-op when already configured; safe to call from every provision.
  if tpl.nspawn and type(tpl.nspawn.bridge) == "string" and tpl.nspawn.bridge ~= "" then
    local ok_b, ret_b = pcall(network_bridge.ensure, { name = tpl.nspawn.bridge })
    if not ok_b then
      ctx.audit.append({
        actor = actor, action = "machine.provision.failed",
        target = name,
        meta = { template = tmpl_id, stage = "bridge", error = tostring(ret_b) },
      })
      return { ok = false, error = "host bridge setup failed: " .. tostring(ret_b) }
    end
  end

  -- 1) Hand off to the framework. assay.nspawn does:
  --      machinectl pull-tar / clone / pull-raw / debootstrap (per rootfs.source)
  --      write /etc/systemd/nspawn/<name>.nspawn from template.nspawn
  --      systemctl enable + start systemd-nspawn@<name>.service
  --      wait until machined reports state == "running"
  local ok, ret = pcall(nspawn.provision, {
    name    = name,
    rootfs  = tpl.rootfs,
    config  = tpl.nspawn,
    systemd = tpl.systemd,            -- enable_units list — applied in-chroot
    ready_timeout = args.ready_timeout or 90,
    on_stage = args.on_stage,         -- forwarded for async-job progress reporting
  })
  if not ok then
    ctx.audit.append({
      actor = actor, action = "machine.provision.failed",
      target = name, meta = { template = tmpl_id, error = tostring(ret) },
    })
    return { ok = false, error = tostring(ret) }
  end

  -- 1.4) Write a static-IP networkd config into the rootfs and restart
  -- networkd inside the container. nsbr0's built-in DHCP server doesn't
  -- reliably issue leases on this host; without this, host0 falls back
  -- to 169.254.x link-local and apt/binary installs that need the
  -- internet fail. The container started ~seconds ago with the default
  -- DHCP config; we overwrite it with a static config and reload.
  if tpl.nspawn and type(tpl.nspawn.bridge) == "string" and tpl.nspawn.bridge ~= "" then
    if args.on_stage then args.on_stage("network", "in_progress") end
    local ip, ip_err = network_bridge.write_container_static_config(name)
    if not ip then
      ctx.audit.append({
        actor = actor, action = "machine.provision.network_warn",
        target = name, meta = { error = ip_err },
      })
      if args.on_stage then args.on_stage("network", "failed", ip_err) end
    else
      -- Apply the new config inside the running container — restart
      -- networkd. Brief network blip; container survives.
      local cmd = ("sudo -n systemd-run --machine=%s --pipe --quiet --wait /bin/sh -c 'systemctl restart systemd-networkd'"):format(name)
      local r = shell.exec(cmd, { timeout = 30 })
      if not r or r.status ~= 0 then
        ctx.audit.append({
          actor = actor, action = "machine.provision.network_warn",
          target = name,
          meta = { ip = ip, error = "networkd restart: " .. ((r and r.stderr) or "unknown") },
        })
        if args.on_stage then args.on_stage("network", "failed",
          "static config written but networkd restart failed; container may have stale DHCP fallback") end
      else
        ctx.audit.append({
          actor = actor, action = "machine.provision.network",
          target = name, meta = { ip = ip },
        })
        if args.on_stage then args.on_stage("network", "done", "Static IP " .. ip) end
      end
    end
  end

  -- 1.5) Apply CPU + memory limits via a host-side systemd drop-in +
  -- `systemctl set-property --runtime` for live application. Best-effort:
  -- a failure here does NOT roll back the provision because the container
  -- is already up and the package install (next step) is the more
  -- impactful work. We log the failure into audit + the on_stage callback
  -- so operators see it in the in-flight job card.
  local cpu_cores, memory_gb = resolve_resources(args, tpl)
  if cpu_cores or memory_gb then
    if args.on_stage then args.on_stage("resources", "in_progress") end
    local rr = resources.apply(name, cpu_cores, memory_gb)
    if rr.ok then
      ctx.audit.append({
        actor = actor, action = "machine.provision.resources",
        target = name,
        meta = { cpu_cores = cpu_cores, memory_gb = memory_gb, live = rr.live },
      })
      if args.on_stage then
        local desc = {}
        if cpu_cores then desc[#desc+1] = ("%g cores"):format(cpu_cores) end
        if memory_gb then desc[#desc+1] = ("%g GB"):format(memory_gb) end
        args.on_stage("resources", "done", "Applied " .. table.concat(desc, " + "))
      end
    else
      ctx.audit.append({
        actor = actor, action = "machine.provision.resources_warn",
        target = name,
        meta = { error = rr.error,
                 cpu_cores = cpu_cores, memory_gb = memory_gb },
      })
      if args.on_stage then
        args.on_stage("resources", "failed", rr.error)
      end
    end
  elseif args.on_stage then
    args.on_stage("resources", "done", "no limits requested")
  end

  -- 2) Seed the package manager's desired_state with the template's package
  -- list, then reconcile so the packages actually get installed on the
  -- newly-booted machine.
  local install_result
  if type(tpl.packages) == "table" and #tpl.packages > 0 then
    if args.on_stage then args.on_stage("packages", "in_progress") end
    local desired = pkgs.read_desired_state()
    desired.targets = desired.targets or {}
    desired.targets[name] = {
      template = tmpl_id,
      packages = tpl.packages,
    }
    pkgs.write_desired_state(desired)
    ctx.audit.append({
      actor = actor, action = "machine.provision.seeded",
      target = name,
      meta   = { template = tmpl_id, packages = tpl.packages },
    })
    -- pkgs.reconcile is the same path triggered from the UI's Reconcile
    -- button. Audit events for individual install ops come from there.
    install_result = pkgs.reconcile(name, { actor = actor })
    if args.on_stage then
      local r = install_result and install_result.result or {}
      local fail_n = (r.failed and #r.failed) or 0
      args.on_stage("packages", fail_n > 0 and "failed" or "done",
        fail_n > 0
          and (("%d/%d package(s) failed"):format(fail_n, #tpl.packages))
          or  (("Installed %d package(s)"):format(#tpl.packages)))
    end
  elseif args.on_stage then
    args.on_stage("packages", "done", "no packages declared")
  end

  ctx.audit.append({
    actor = actor, action = "machine.provision.end",
    target = name, meta = { template = tmpl_id },
  })

  return {
    ok = true,
    name = name,
    template = tmpl_id,
    packages_seeded = (tpl.packages or {}),
    install_result = install_result,
    resources = (cpu_cores or memory_gb) and { cpu_cores = cpu_cores, memory_gb = memory_gb } or nil,
  }
end

--- Destroy an nspawn machine and clean up its package-manager desired state.
function M.destroy(args)
  args = args or {}
  local actor = args.actor or "system"
  local name = args.name
  if type(name) ~= "string" or name == "" then
    return { ok = false, error = "name required" }
  end

  -- Existence check up front — assay's nspawn.destroy is idempotent
  -- (no-op for missing machines), but the API contract here is "delete
  -- this specific machine"; if it doesn't exist, return an error so the
  -- caller doesn't get a misleading "Deleted X." flash for a name that
  -- was never there.
  local exists = false
  do
    local list_ok, list = pcall(systemd.list_machines)
    if list_ok and type(list) == "table" then
      for _, m in ipairs(list) do
        if m.name == name then exists = true; break end
      end
    end
  end
  if not exists then
    ctx.audit.append({
      actor = actor, action = "machine.destroy.failed",
      target = name, meta = { error = "no such machine" },
    })
    return { ok = false, error = "no such machine: " .. name }
  end

  ctx.audit.append({
    actor = actor, action = "machine.destroy.start", target = name,
  })

  local ok, ret = pcall(nspawn.destroy, name)
  if not ok then
    ctx.audit.append({
      actor = actor, action = "machine.destroy.failed",
      target = name, meta = { error = tostring(ret) },
    })
    return { ok = false, error = tostring(ret) }
  end

  -- Clear the resources drop-in (and its containing dir if empty). Best-
  -- effort: nspawn.destroy already removed the unit; the dir/file may
  -- already be gone.
  pcall(resources.clear, name)

  -- Release the static-IP allocation so the next container with the same
  -- name (or any other) can claim it. Best-effort.
  pcall(network_bridge.release_ip, name)

  -- Drop from desired_state so the package manager stops reporting drift.
  -- This is best-effort: a read/write error here shouldn't fail the whole
  -- destroy (the machine itself is already gone) — log a warning instead.
  local cleanup_ok, cleanup_err = pcall(function()
    local desired = pkgs.read_desired_state()
    if desired.targets and desired.targets[name] then
      desired.targets[name] = nil
      pkgs.write_desired_state(desired)
    end
  end)
  if not cleanup_ok then
    ctx.audit.append({
      actor = actor, action = "machine.destroy.cleanup_warn",
      target = name, meta = { error = tostring(cleanup_err) },
    })
  end

  ctx.audit.append({ actor = actor, action = "machine.destroy.end", target = name })
  return { ok = true, name = name }
end

return M
