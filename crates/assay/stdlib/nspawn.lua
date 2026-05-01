--- @module assay.nspawn
--- @description nspawn machine provisioning + lifecycle. Composes the
---              machinectl, systemd, and fs builtins into a high-level
---              "create from template, destroy, start, stop, configure" API.
--- @keywords nspawn, container, machine, provision, machinectl, systemd

local M = {}

-- ── Privilege detection (mirrors pkg.lua) ─────────────────────────────────
--
-- nspawn operations talk to systemd-machined and write to /etc/systemd/nspawn/.
-- Both require root. When running as a non-root user we shell out via
-- `sudo -n` for the file-write paths; the Rust builtins (machinectl.*,
-- systemd.unit_action) run subprocesses that already need root themselves
-- (sudo wrapping is applied at invocation time).
local _is_root_cached = nil
local function is_root()
  if _is_root_cached == nil then
    local r = shell.exec("id -u", {})
    _is_root_cached = (r and r.stdout and r.stdout:match("^0") ~= nil) or false
  end
  return _is_root_cached
end

local function sudo_prefix()
  return is_root() and "" or "sudo -n "
end

-- Shell-quote one arg so it's safe inside a /bin/sh -c '...' invocation.
-- Wraps in single quotes; embedded ' becomes '"'"'.
local function shell_quote(s)
  if type(s) ~= "string" then return "''" end
  return "'" .. s:gsub("'", [['"'"']]) .. "'"
end

-- Validate a machine name. Same charset as machinectl's Rust validator
-- (alphanumerics + . _ -), no leading dash. Mirrors the systemd-machined
-- accepted set.
local function validate_name(name)
  if type(name) ~= "string" or name == "" then
    error("nspawn: name must be a non-empty string", 2)
  end
  if not name:match("^[A-Za-z0-9._%-]+$") then
    error("nspawn: name must match [A-Za-z0-9._-]+ (got " .. tostring(name) .. ")", 2)
  end
  if name:sub(1, 1) == "-" then
    error("nspawn: name must not start with '-'", 2)
  end
end

-- ── Introspection ─────────────────────────────────────────────────────────

--- List all nspawn machines on the host.
--- Returns the same shape as systemd.list_machines (array of records).
function M.list()
  return systemd.list_machines()
end

--- Look up a single machine by name. Returns the systemd.machine_status
--- table, or nil if the machine isn't present.
function M.get(name)
  validate_name(name)
  local ok, info = pcall(systemd.machine_status, name)
  if ok then return info end
  return nil
end

--- True iff a machine with this name is registered with machined OR has a
--- rootfs at /var/lib/machines/<name>. (machinectl pull-tar creates the
--- rootfs but the machine isn't "registered" until started.)
function M.exists(name)
  validate_name(name)
  if M.get(name) ~= nil then return true end
  return fs.exists("/var/lib/machines/" .. name)
end

-- ── .nspawn unit config (read/write) ──────────────────────────────────────
--
-- Format reference: man systemd.nspawn(5). Sections we emit:
--   [Exec]      Boot, NotifyReady, PrivateUsers, Capability, SystemCallFilter
--   [Files]     Bind, BindReadOnly, Inaccessible, ReadOnly
--   [Network]   VirtualEthernet, ResolvConf, Bridge, MACVLAN, IPVLAN, Private
--
-- We don't try to be a complete parser — read/write target the subset
-- we'll emit ourselves. Operator-edited extra lines are PRESERVED on read
-- but a write replaces the file entirely.

M.config = {}

local NSPAWN_DIR = "/etc/systemd/nspawn"

local function unit_path(name)
  return NSPAWN_DIR .. "/" .. name .. ".nspawn"
end

--- Read an existing .nspawn unit file. Returns the raw text (not parsed)
--- so callers can see the existing content verbatim. Returns nil if the
--- file doesn't exist.
function M.config.read(name)
  validate_name(name)
  local p = unit_path(name)
  if not fs.exists(p) then return nil end
  return fs.read(p)
end

-- Format a list of values as repeated `Key=value` lines for an INI section.
local function ini_repeat(key, values)
  if type(values) ~= "table" then return "" end
  local out = {}
  for _, v in ipairs(values) do
    out[#out+1] = key .. "=" .. tostring(v)
  end
  return table.concat(out, "\n") .. (#out > 0 and "\n" or "")
end

local function bool_to_yesno(v)
  if v == true then return "yes" end
  if v == false then return "no" end
  return nil
end

-- Map our resolv_conf identifier to systemd's --resolv-conf= flag spelling.
-- We accept both hyphen and underscore; on the wire systemd wants hyphens.
local function nspawn_resolv_conf_emit(s)
  if type(s) ~= "string" then return nil end
  return (s:gsub("_", "-"))
end

--- Render a config table into the .nspawn INI text body.
---
--- The shape mirrors `[template.nspawn]` from pkg templates plus a few
--- machine-specific extras (per-instance binds, etc.):
---
---   {
---     boot             = true,
---     notify_ready     = true,
---     private_users    = false,
---     capabilities     = {"all"},
---     binds            = {"/dev/kmsg", "/srv/data"},
---     binds_ro         = {"/sys/module"},
---     inaccessible     = {"/sys/module/apparmor"},
---     virtual_ethernet = true,
---     resolv_conf      = "bind-host",  -- or "auto", "off", etc.
---     bridge           = "br0",        -- optional
---   }
function M.config.render(cfg)
  if type(cfg) ~= "table" then
    error("nspawn.config.render: cfg must be a table", 2)
  end
  local parts = {}

  -- [Exec]
  -- Note: ResolvConf= lives in [Exec], not [Network] — that's where
  -- systemd.nspawn(5) places it (the option is about how the container
  -- binary is launched, not network config). Confused me too.
  local exec_lines = {}
  local b = bool_to_yesno(cfg.boot)
  if b then exec_lines[#exec_lines+1] = "Boot=" .. b end
  b = bool_to_yesno(cfg.notify_ready)
  if b then exec_lines[#exec_lines+1] = "NotifyReady=" .. b end
  b = bool_to_yesno(cfg.private_users)
  if b then exec_lines[#exec_lines+1] = "PrivateUsers=" .. b end
  if cfg.capabilities and type(cfg.capabilities) == "table" then
    exec_lines[#exec_lines+1] = "Capability=" .. table.concat(cfg.capabilities, ",")
  end
  if cfg.resolv_conf then
    local rc = nspawn_resolv_conf_emit(cfg.resolv_conf)
    if rc then exec_lines[#exec_lines+1] = "ResolvConf=" .. rc end
  end
  if #exec_lines > 0 then
    parts[#parts+1] = "[Exec]\n" .. table.concat(exec_lines, "\n") .. "\n"
  end

  -- [Files]
  local files_lines = {}
  if cfg.binds then
    files_lines[#files_lines+1] = ini_repeat("Bind", cfg.binds):sub(1, -2)
  end
  if cfg.binds_ro then
    files_lines[#files_lines+1] = ini_repeat("BindReadOnly", cfg.binds_ro):sub(1, -2)
  end
  if cfg.inaccessible then
    files_lines[#files_lines+1] = ini_repeat("Inaccessible", cfg.inaccessible):sub(1, -2)
  end
  -- Filter out empty strings (when *_lines functions returned only "\n").
  local filtered = {}
  for _, s in ipairs(files_lines) do
    if s and s ~= "" then filtered[#filtered+1] = s end
  end
  if #filtered > 0 then
    parts[#parts+1] = "[Files]\n" .. table.concat(filtered, "\n") .. "\n"
  end

  -- [Network]
  local net_lines = {}
  b = bool_to_yesno(cfg.virtual_ethernet)
  if b then net_lines[#net_lines+1] = "VirtualEthernet=" .. b end
  if type(cfg.bridge) == "string" and cfg.bridge ~= "" then
    net_lines[#net_lines+1] = "Bridge=" .. cfg.bridge
  end
  if #net_lines > 0 then
    parts[#parts+1] = "[Network]\n" .. table.concat(net_lines, "\n") .. "\n"
  end

  return table.concat(parts, "\n")
end

--- Atomically write the .nspawn unit file for a machine. Requires root for
--- the actual write to /etc/systemd/nspawn (uses sudo install when not root).
function M.config.write(name, cfg)
  validate_name(name)
  local body = M.config.render(cfg)
  local dst = unit_path(name)

  -- Write to user-owned tmp first, then sudo install into /etc.
  local tmp = "/tmp/assay-nspawn-" .. name .. "." .. tostring(os.time()) .. ".nspawn"
  fs.write(tmp, body)
  local cmd = sudo_prefix() ..
    ("install -D -m 0644 -o root -g root %s %s"):format(shell_quote(tmp), shell_quote(dst))
  local r = shell.exec(cmd, {})
  fs.remove(tmp)
  if not r or r.status ~= 0 then
    error("nspawn.config.write: install failed: " .. ((r and r.stderr) or "unknown"))
  end
  return { ok = true, path = dst, bytes = #body }
end

-- ── Lifecycle ─────────────────────────────────────────────────────────────

--- Start an existing nspawn machine. Enables systemd-nspawn@<name> if not
--- already enabled, then starts it.
function M.start(name)
  validate_name(name)
  local r = systemd.unit_action("systemd-nspawn@" .. name .. ".service", "start", { timeout = 60 })
  if not r or r.status ~= 0 then
    error("nspawn.start: " .. name .. ": " .. ((r and r.stderr) or "unknown"))
  end
  return { ok = true }
end

--- Stop an nspawn machine via systemd. Graceful by default.
function M.stop(name, opts)
  validate_name(name)
  opts = opts or {}
  local action = opts.force and "stop" or "stop"  -- systemctl stop is graceful via SIGTERM
  local r = systemd.unit_action("systemd-nspawn@" .. name .. ".service", action,
                                { timeout = opts.timeout or 60 })
  if not r or r.status ~= 0 then
    -- Already stopped is fine.
    if r and r.stderr and r.stderr:find("not loaded", 1, true) then
      return { ok = true, was_running = false }
    end
    error("nspawn.stop: " .. name .. ": " .. ((r and r.stderr) or "unknown"))
  end
  return { ok = true }
end

--- Wait for a machine to be live: registered with systemd-machined AND
--- has a leader pid > 0. systemd.machine_status returns nil-on-error
--- (GetMachine D-Bus call fails when not registered), and emits a
--- leader_pid field once the container's PID 1 is up. There's no separate
--- "state" field on the D-Bus interface — leader_pid being present is the
--- canonical "container is running" signal.
--- Returns { ok = true, info = ... } once ready, or errors after timeout.
function M.wait_ready(name, opts)
  validate_name(name)
  opts = opts or {}
  local timeout = opts.timeout or 60
  local poll_ms = opts.poll_ms or 500
  local deadline = os.time() + timeout
  while os.time() < deadline do
    local info = M.get(name)
    if info and type(info.leader_pid) == "number" and info.leader_pid > 0 then
      return { ok = true, info = info }
    end
    if sleep then sleep(poll_ms / 1000.0)
    else shell.exec("sleep " .. tostring(poll_ms / 1000.0), {}) end
  end
  error("nspawn.wait_ready: " .. name .. " did not register with leader_pid within " ..
        tostring(timeout) .. "s")
end

-- ── Provision / destroy ───────────────────────────────────────────────────

--- High-level provision: bootstrap rootfs + write unit + enable + start + wait.
---
--- spec = {
---   name   = "myhost",
---   rootfs = { source = "machinectl-pull-tar", url = "..." }
---           | { source = "machinectl-clone",   from = "_golden" }
---           | { source = "machinectl-pull-raw", url = "..." }
---           | { source = "debootstrap", suite = "bookworm", mirror = "..." }
---   config = { ... },                -- as in M.config.render
---   ready_timeout = 60,              -- seconds to wait for running
---   verify_image = false,            -- pass through to machinectl pull-*
--- }
function M.provision(spec)
  if type(spec) ~= "table" then
    error("nspawn.provision: spec table required", 2)
  end
  validate_name(spec.name)
  if M.exists(spec.name) then
    error("nspawn.provision: '" .. spec.name .. "' already exists; destroy it first", 2)
  end
  local rootfs = spec.rootfs or {}
  local cfg = spec.config or {}

  -- Optional progress callback. Called as on_stage(stage, status, msg?)
  -- where stage ∈ {"rootfs","unit","boot"} and status ∈ {"in_progress",
  -- "done","failed"}. The orchestrator uses these to drive the in-flight
  -- provisioning card on /machines.
  local on_stage = spec.on_stage or function(_,_,_) end

  -- 1) Bootstrap rootfs
  on_stage("rootfs", "in_progress")
  if rootfs.source == "machinectl-pull-tar" then
    if type(rootfs.url) ~= "string" then
      error("nspawn.provision: rootfs.url required for pull-tar", 2)
    end
    local r = machinectl.pull_tar(rootfs.url, spec.name, {
      verify = spec.verify_image or false,
      timeout = rootfs.timeout or 1800,
    })
    if not r or r.status ~= 0 then
      error("nspawn.provision: pull-tar failed: " .. ((r and r.stderr) or "unknown"))
    end
  elseif rootfs.source == "machinectl-pull-raw" then
    if type(rootfs.url) ~= "string" then
      error("nspawn.provision: rootfs.url required for pull-raw", 2)
    end
    local r = machinectl.pull_raw(rootfs.url, spec.name, {
      verify = spec.verify_image or false,
      timeout = rootfs.timeout or 1800,
    })
    if not r or r.status ~= 0 then
      error("nspawn.provision: pull-raw failed: " .. ((r and r.stderr) or "unknown"))
    end
  elseif rootfs.source == "machinectl-clone" then
    if type(rootfs.from) ~= "string" then
      error("nspawn.provision: rootfs.from required for clone", 2)
    end
    local r = machinectl.clone(rootfs.from, spec.name, { timeout = rootfs.timeout or 600 })
    if not r or r.status ~= 0 then
      error("nspawn.provision: clone failed: " .. ((r and r.stderr) or "unknown"))
    end
  elseif rootfs.source == "debootstrap" then
    -- debootstrap shell-out. Caller's responsibility to have debootstrap on PATH.
    if type(rootfs.suite) ~= "string" or type(rootfs.mirror) ~= "string" then
      error("nspawn.provision: debootstrap requires suite + mirror", 2)
    end
    local target = "/var/lib/machines/" .. spec.name
    -- Build flag list. Defaults match the previous behavior; optional fields
    -- (variant, components, keyring, include) extend it for Ubuntu / non-default uses.
    local flags = { "--variant=" .. (rootfs.variant or "minbase") }
    if type(rootfs.components) == "string" and rootfs.components ~= "" then
      flags[#flags+1] = "--components=" .. rootfs.components
    end
    if type(rootfs.keyring) == "string" and rootfs.keyring ~= "" then
      flags[#flags+1] = "--keyring=" .. shell_quote(rootfs.keyring)
    end
    -- --include lets the template name extra packages to install during
    -- bootstrap. Mandatory in practice for nspawn `Boot=yes` containers
    -- because --variant=minbase doesn't include systemd-sysv (so /sbin/init
    -- doesn't exist and the machine fails to boot with execv ENOENT).
    if type(rootfs.include) == "string" and rootfs.include ~= "" then
      flags[#flags+1] = "--include=" .. rootfs.include
    end
    -- debootstrap doesn't support `--` end-of-options separator (legacy argv);
    -- safety comes from the schema validator which constrains suite to
    -- ^[a-z][a-z0-9.-]*$ so it can't be misread as a flag.
    local cmd = sudo_prefix() ..
      ("debootstrap %s %s %s %s"):format(
        table.concat(flags, " "),
        shell_quote(rootfs.suite),
        shell_quote(target),
        shell_quote(rootfs.mirror))
    local r = shell.exec(cmd, { timeout = rootfs.timeout or 1800 })
    if not r or r.status ~= 0 then
      error("nspawn.provision: debootstrap failed: " .. ((r and r.stderr) or "unknown"))
    end
  else
    on_stage("rootfs", "failed", "unknown rootfs.source: " .. tostring(rootfs.source))
    error("nspawn.provision: unknown rootfs.source: " .. tostring(rootfs.source), 2)
  end
  on_stage("rootfs", "done")

  -- 1b) Enable in-rootfs systemd units before first boot. debootstrap puts
  -- units like systemd-networkd in /usr/lib/systemd/system/ but doesn't
  -- enable them; without this step the container boots without networking
  -- (host0 stays DOWN, no DHCP, no DNS, no apt). chroot into the rootfs
  -- and create the standard wants-symlinks for each service.
  if spec.systemd and type(spec.systemd.enable) == "table"
     and #spec.systemd.enable > 0 then
    local rootfs_path = "/var/lib/machines/" .. spec.name
    for _, svc in ipairs(spec.systemd.enable) do
      -- Validate to keep chroot+exec injection-safe. Service names follow
      -- the unit-name charset.
      if type(svc) ~= "string" or not svc:match("^[A-Za-z0-9._%-:@\\]+$") then
        error("nspawn.provision: invalid systemd unit name: " .. tostring(svc), 2)
      end
      local cmd = sudo_prefix() ..
        ("chroot %s systemctl enable %s"):format(
          shell_quote(rootfs_path), shell_quote(svc))
      local r = shell.exec(cmd, { timeout = 30 })
      if not r or r.status ~= 0 then
        -- Tolerate "Failed to enable unit: Unit X already enabled" — chroot
        -- systemctl exits non-zero on that warning.
        if not (r and r.stderr and r.stderr:find("already enabled", 1, true)) then
          error("nspawn.provision: chroot enable " .. svc ..
                " failed: " .. ((r and r.stderr) or "unknown"))
        end
      end
    end
  end

  -- 2) Write .nspawn unit
  on_stage("unit", "in_progress")
  M.config.write(spec.name, cfg)
  on_stage("unit", "done")

  -- 3) Enable + start systemd-nspawn@<name>.service, then wait for ready
  on_stage("boot", "in_progress")
  local en = systemd.unit_action("systemd-nspawn@" .. spec.name .. ".service", "enable",
                                 { timeout = 30 })
  -- Enable returns non-zero if already enabled; tolerate that.
  if en and en.status ~= 0 and not (en.stderr or ""):find("already", 1, true) then
    error("nspawn.provision: enable failed: " .. (en.stderr or "unknown"))
  end
  M.start(spec.name)

  -- 4) Wait for ready
  local ready = M.wait_ready(spec.name, { timeout = spec.ready_timeout or 60 })
  on_stage("boot", "done")
  return { ok = true, name = spec.name, info = ready.info }
end

--- Destroy a machine: stop it (if running), disable the service unit,
--- remove the unit file, and remove the rootfs via machinectl remove.
function M.destroy(name, opts)
  validate_name(name)
  opts = opts or {}

  -- 1) Stop if running
  if M.get(name) ~= nil then
    pcall(M.stop, name, { timeout = 30 })
    -- Wait briefly for unit to settle.
    local deadline = os.time() + 15
    while os.time() < deadline and M.get(name) ~= nil do
      if sleep then sleep(0.25) else shell.exec("sleep 0.25", {}) end
    end
  end

  -- 2) Disable the service unit (ignore "no such unit").
  local _ = systemd.unit_action("systemd-nspawn@" .. name .. ".service", "disable",
                                { timeout = 30 })

  -- 3) Remove the .nspawn unit file (sudo rm).
  local unit = unit_path(name)
  if fs.exists(unit) then
    local r = shell.exec(sudo_prefix() .. ("rm -f %s"):format(shell_quote(unit)), {})
    if not r or r.status ~= 0 then
      error("nspawn.destroy: rm " .. unit .. " failed: " .. ((r and r.stderr) or "unknown"))
    end
  end

  -- 4) machinectl remove for the rootfs (handles both directory and image
  --    storage backends).
  if fs.exists("/var/lib/machines/" .. name) then
    local r = machinectl.remove(name, { timeout = 60 })
    if not r or r.status ~= 0 then
      error("nspawn.destroy: machinectl remove failed: " .. ((r and r.stderr) or "unknown"))
    end
  end

  return { ok = true }
end

return M
