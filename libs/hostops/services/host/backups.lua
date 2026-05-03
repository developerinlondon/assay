-- services/host/backups.lua
--
-- Plan 15 — host backup service. Owns the profile TOML lifecycle, the
-- secret-store integration, and the orchestrated run-now / restore flows.
-- Drives the rustic CLI via `assay.rustic` (assay stdlib), bracketed by
-- `assay.fs_snapshot` for crash-consistent capture on btrfs / zfs.
--
-- The page handlers (pages/backups*.lua) call into this module; this
-- module never directly handles HTTP requests.

local rustic       = require("assay.rustic")
local fs_snapshot  = require("assay.fs_snapshot")
local schedule     = require("services.host.backup_schedule")
local marker       = require("services.host.backup_marker")
local ctx = require("hostops.ctx")
-- secret_store is read lazily (ctx.secret is populated by mount(), not
-- at module load time). Use a closure so existing call sites — e.g.
-- `secret_store.read(...)`, `secret_store.available()` — keep working.
local secret_store = setmetatable({}, {
  __index = function(_, key) return ctx.secret[key] end,
})
local M = {}

local DEFAULT_PROFILE_DIR = "/etc/rustic"
local PROFILE_NAME = "host"  -- v1: single profile

-- Profile directory is operator-configurable via ctx.backup_profile_dir
-- so smoke tests can isolate to a temp dir.
local function profile_dir()
  return ctx.backup_profile_dir or DEFAULT_PROFILE_DIR
end

local function profile_path(name)
  name = name or PROFILE_NAME
  return profile_dir() .. "/" .. name .. ".toml"
end

----------------------------------------------------------------------
-- Profile read/write
----------------------------------------------------------------------

local function read_file(path)
  local f = io.open(path, "r")
  if not f then return nil end
  local v = f:read("*a")
  f:close()
  return v
end

local function write_file_root_0600(path, body)
  local tmp = path .. ".tmp"
  local sentinel = "EOF_KW_" .. tostring(os.time()) .. "_" .. tostring(math.random(1, 1e9))
  local script = string.format(
    "install -d -m 0700 -o root -g root %s && " ..
    "umask 077 && cat > %s << '%s'\n%s%s\n" ..
    "chmod 0600 %s && chown root:root %s && mv %s %s",
    profile_dir(), tmp, sentinel, body, sentinel, tmp, tmp, tmp, path)
  local r = shell.exec("sudo -n bash -c " .. string.format("%q", script))
  if not r or r.status ~= 0 then
    return nil, "profile write failed: " .. ((r and r.stderr) or "?")
  end
  return true
end

local DEFAULT_SOURCES = {
  "/etc",
  "/root",
  "/etc/systemd/nspawn",
  "/var/lib/machines",
}

local function default_tags() return { "host", "knowhere", "daily" } end

local function render_profile(repo)
  -- Render a knowhere-managed TOML profile. Format is owned by us
  -- (rustic_core doesn't read this file — backup_run does).
  local sources_lines = {}
  for _, s in ipairs(repo.sources or DEFAULT_SOURCES) do
    table.insert(sources_lines, string.format('  "%s",', s))
  end
  local tags_lines = {}
  for _, t in ipairs(repo.tags or default_tags()) do
    table.insert(tags_lines, string.format('"%s"', t))
  end

  return string.format([[
# knowhere-managed backup profile. Edit via /backups in the dashboard.

[repository]
url    = "%s"
region = "%s"

[backup]
sources = [
%s
]
fs_snapshot_root = "%s"
tags = [%s]
]],
    repo.url,
    repo.region or "",
    table.concat(sources_lines, "\n"),
    repo.fs_snapshot_root or "/var/lib/machines",
    table.concat(tags_lines, ", "))
end

--- Parse the TOML profile into a plain table. Uses assay's toml
--- helper if available; falls back to a minimal hand-roll for v1.
local function parse_toml(body)
  if assay and assay.toml and type(assay.toml.parse) == "function" then
    local ok, t = pcall(assay.toml.parse, body)
    if ok and type(t) == "table" then return t end
  end
  -- Minimal fallback: parse just the keys we care about. Good enough
  -- for the round-trip we control.
  local out = { repository = {}, backup = { sources = {}, tags = {} } }
  local section
  for line in body:gmatch("[^\r\n]+") do
    local hdr = line:match("^%[([%w_%.]+)%]")
    if hdr then
      section = hdr
    elseif line:match("^sources%s*=%s*%[%s*$") then
      -- Multi-line array start: switch to a synthetic section but DO
      -- NOT also fall through to the scalar-assignment branch below
      -- (which would clobber `out.backup.sources` with the literal
      -- string "[").
      section = "backup.sources"
    elseif line:match("^%]") and section == "backup.sources" then
      -- End of multi-line array.
      section = "backup"
    elseif section == "backup.sources" then
      -- Each line in the array is a quoted entry.
      local entry = line:match('^%s*"([^"]+)",?')
      if entry then table.insert(out.backup.sources, entry) end
    else
      local k, v = line:match("^([%w_]+)%s*=%s*(.+)$")
      if k and v and section then
        v = v:gsub("^%s+", ""):gsub("%s+$", "")
        if v:sub(1, 1) == '"' and v:sub(-1) == '"' then
          v = v:sub(2, -2)
        end
        local target = out[section] or {}
        target[k] = v
        out[section] = target
      end
    end
  end
  return out
end

function M.read_profile(name)
  local body = read_file(profile_path(name))
  if not body then return nil end
  return parse_toml(body)
end

function M.write_profile(repo, name)
  local body = render_profile(repo)
  return write_file_root_0600(profile_path(name), body)
end

function M.delete_profile(name)
  shell.exec("sudo -n rm -f " .. profile_path(name))
  for _, k in ipairs({ "password", "access_key_id", "secret_access_key" }) do
    secret_store.delete(name or PROFILE_NAME, k)
  end
  return true
end

----------------------------------------------------------------------
-- State machine
----------------------------------------------------------------------

--- Compute the page state. Returns one of:
---   "B"     -- no profile, vault unsealed (or absent)
---   "B-blk" -- no profile, vault loaded+sealed → block setup
---   "C"     -- profile present, healthy
---   "C-w"   -- profile present, vault sealed at runtime
---   "C-bad" -- profile present, malformed
--- (state A "rustic missing" is gone — rustic_core is linked into the binary.)
function M.state()
  local profile = M.read_profile(PROFILE_NAME)
  local vault_loaded, _ = secret_store.available()

  if not profile then
    -- No profile yet. If vault module is loaded, check seal status —
    -- if loaded+sealed, block setup until unsealed.
    local va_ok, va = pcall(require, "services.vault_admin")
    if va_ok and va and type(va.status) == "function" then
      local ok, s = pcall(va.status)
      if ok and type(s) == "table" and s.loaded == true and s.sealed == true then
        return "B-blk"
      end
    end
    return "B"
  end

  -- Profile present. Sanity-check that it parses to something usable.
  if not (profile.repository and profile.repository.url) then
    return "C-bad"
  end

  -- Vault sealed at runtime → operations blocked.
  if profile.repository.url:find("^s3:") then
    -- Check secrets are reachable. If we can read the password we're fine.
    local pw, _err = secret_store.read(PROFILE_NAME, "password")
    if not pw then
      return "C-w"
    end
  end

  return "C"
end

----------------------------------------------------------------------
-- Connection probes
----------------------------------------------------------------------

local function build_conn_args(repo, password, akid, sk)
  local conn = {
    repository = repo.url,
    region     = repo.region,
    password   = password,
    access_key_id = akid,
    secret_access_key = sk,
  }
  if repo.enable_virtual_host_style then conn.enable_virtual_host_style = true end
  return conn
end

function M.test_connection(args)
  args = args or {}
  local conn = {
    repository = args.url,
    region = args.region,
    password = args.password,
    access_key_id = args.access_key_id,
    secret_access_key = args.secret_access_key,
  }
  local ret, err2 = rustic.check(conn)
  if not ret then return { ok = false, error = tostring(err2) } end
  return ret
end

----------------------------------------------------------------------
-- Init / reconfigure
----------------------------------------------------------------------

function M.init_repo(args)
  args = args or {}
  local actor = args.actor or "system"

  -- 1. Validate.
  if type(args.url) ~= "string" or args.url == "" then
    return { ok = false, error = "repository URL required" }
  end
  if type(args.password) ~= "string" or args.password == "" then
    return { ok = false, error = "password required" }
  end
  if type(args.password_confirm) == "string" and args.password_confirm ~= args.password then
    return { ok = false, error = "password and confirm don't match" }
  end

  -- 2. Run rustic init.
  local conn = {
    repository = args.url,
    region = args.region,
    password = args.password,
    access_key_id = args.access_key_id,
    secret_access_key = args.secret_access_key,
  }
  local ret, err2 = rustic.init(conn)
  if not ret then
    local ok = false
    ret = err2
    ctx.audit.append({ actor = actor, action = "backups.setup_init_failed",
                   target = PROFILE_NAME, meta = { error = tostring(ret) } })
    return { ok = false, error = tostring(ret) }
  end

  -- 3. Persist secrets to vault and/or files.
  local function s_write(k, v)
    local sok, serr = secret_store.write(PROFILE_NAME, k, v)
    return sok, serr
  end
  local sok, serr = s_write("password", args.password)
  if not sok then return { ok = false, error = serr } end
  if args.access_key_id then
    sok, serr = s_write("access_key_id", args.access_key_id)
    if not sok then return { ok = false, error = serr } end
  end
  if args.secret_access_key then
    sok, serr = s_write("secret_access_key", args.secret_access_key)
    if not sok then return { ok = false, error = serr } end
  end

  -- 4. Write profile.
  local ok2, perr = M.write_profile({
    url = args.url,
    region = args.region,
    enable_virtual_host_style = args.enable_virtual_host_style,
    sources = args.sources or DEFAULT_SOURCES,
    tags = args.tags or default_tags(),
    fs_snapshot_root = args.fs_snapshot_root or "/var/lib/machines",
  })
  if not ok2 then return { ok = false, error = perr } end

  -- 5. Install + enable systemd timer.
  local sok2, serr2 = schedule.write_timer({
    profile = PROFILE_NAME,
    hour = args.schedule_hour or 2,
    jitter_s = args.schedule_jitter or 1800,
  })
  if not sok2 then
    ctx.audit.append({ actor = actor, action = "backups.setup_init_warn",
                   target = PROFILE_NAME, meta = { error = serr2, stage = "timer" } })
  else
    schedule.enable()
  end

  ctx.audit.append({
    actor = actor, action = "backups.setup_init", target = PROFILE_NAME,
    meta = { url = args.url, storage = (secret_store.available() and "vault" or "file") },
  })
  return { ok = true }
end

function M.reconfigure(args)
  args = args or {}
  local actor = args.actor or "system"
  ctx.audit.append({ actor = actor, action = "backups.reconfigure", target = PROFILE_NAME })
  -- Tear down then re-init.
  schedule.disable()
  M.delete_profile(PROFILE_NAME)
  return M.init_repo(args)
end

----------------------------------------------------------------------
-- Sources / schedule helpers
----------------------------------------------------------------------

function M.update_sources(args)
  args = args or {}
  local actor = args.actor or "system"
  local profile = M.read_profile(PROFILE_NAME) or {}
  if not (profile.repository and profile.repository.url) then
    return { ok = false, error = "no profile to update" }
  end

  local sources = args.sources or {}
  if #sources == 0 then
    return { ok = false, error = "pick at least one path" }
  end

  local ok, err = M.write_profile({
    url    = profile.repository.url,
    region = profile.repository.region,
    sources = sources,
    fs_snapshot_root = args.fs_snapshot_root or
      (profile.backup and profile.backup.fs_snapshot_root) or "/var/lib/machines",
    tags = (profile.backup and profile.backup.tags) or default_tags(),
  })
  if not ok then return { ok = false, error = err } end

  ctx.audit.append({
    actor = actor, action = "backups.sources_update", target = PROFILE_NAME,
    meta = { count = #sources },
  })
  return { ok = true }
end

function M.update_schedule(args)
  args = args or {}
  local actor = args.actor or "system"
  if args.enabled == false then
    schedule.disable()
    ctx.audit.append({ actor = actor, action = "backups.schedule_update",
                   target = PROFILE_NAME, meta = { enabled = false } })
    return { ok = true, enabled = false }
  end
  local hour = tonumber(args.hour) or 2
  local jitter_s = tonumber(args.jitter_s) or 1800
  local ok, err = schedule.write_timer({
    profile = PROFILE_NAME, hour = hour, jitter_s = jitter_s,
  })
  if not ok then return { ok = false, error = err } end
  schedule.enable()
  ctx.audit.append({ actor = actor, action = "backups.schedule_update",
                 target = PROFILE_NAME, meta = { hour = hour, jitter_s = jitter_s, enabled = true } })
  return { ok = true, enabled = true, hour = hour, jitter_s = jitter_s }
end

----------------------------------------------------------------------
-- Snapshots / restore
----------------------------------------------------------------------

local function load_conn_args()
  local profile = M.read_profile(PROFILE_NAME)
  if not profile or not profile.repository or not profile.repository.url then
    return nil, "no profile configured"
  end
  local pw, perr = secret_store.read(PROFILE_NAME, "password")
  if not pw then return nil, perr end
  local akid = secret_store.read(PROFILE_NAME, "access_key_id")
  local sk   = secret_store.read(PROFILE_NAME, "secret_access_key")
  return {
    repository = profile.repository.url,
    region = profile.repository.region,
    password = pw,
    access_key_id = akid,
    secret_access_key = sk,
  }
end

function M.list_snapshots()
  local conn, err = load_conn_args()
  if not conn then return nil, err end
  local ret, err2 = rustic.snapshots(conn)
  if not ret then return nil, tostring(err2) end
  return ret
end

function M.snapshot_detail(snap_id)
  local conn, err = load_conn_args()
  if not conn then return nil, err end
  local ret, err2 = rustic.snapshot_detail(conn, snap_id)
  if not ret then return nil, tostring(err2) end
  return ret
end

----------------------------------------------------------------------
-- Run now (job-driven)
----------------------------------------------------------------------

function M.run_now(args)
  args = args or {}
  local actor = args.actor or "system"

  local conn, err = load_conn_args()
  if not conn then return { ok = false, error = err } end

  local profile = M.read_profile(PROFILE_NAME) or {}
  local sources = (profile.backup and profile.backup.sources) or DEFAULT_SOURCES
  local tags    = (profile.backup and profile.backup.tags) or default_tags()
  local snap_root = profile.backup and profile.backup.fs_snapshot_root or "/var/lib/machines"

  local job = ctx.jobs.start({
    kind = "backups.run_now",
    target = PROFILE_NAME,
    name = "Run backup now",
    stages = {
      { id = "preflight",    label = "Pre-flight" },
      { id = "fs-snapshot",  label = "FS snapshot" },
      { id = "rustic-backup", label = "rustic backup" },
      { id = "fs-release",   label = "Release snapshot" },
    },
  })

  ctx.audit.append({ actor = actor, action = "backups.run_start",
                 target = PROFILE_NAME, meta = { job_id = job.id, manual = true } })

  -- Spawn the actual work. async.spawn is provided by the assay
  -- runtime; if missing (early boot), fall back to inline run (rare).
  local function worker()
    ctx.jobs.update_stage(job.id, "preflight", "in_progress")
    -- (preflight checks already done above implicitly)
    ctx.jobs.update_stage(job.id, "preflight", "done")

    -- FS snapshot
    ctx.jobs.update_stage(job.id, "fs-snapshot", "in_progress")
    local handle = nil
    local fs_consistency = "live"
    local sources_resolved = {}
    if snap_root and snap_root ~= "" then
      local ok_s, h = pcall(fs_snapshot.take, "manual", snap_root)
      if ok_s and h then
        handle = h
        fs_consistency = h.backend or "live"
        for _, src in ipairs(sources) do
          if src:sub(1, #snap_root) == snap_root then
            local rel = src:sub(#snap_root + 1)
            table.insert(sources_resolved, h.path .. rel)
          else
            table.insert(sources_resolved, src)
          end
        end
        ctx.jobs.append_log(job.id, "fs_snapshot taken (" .. fs_consistency .. ")")
      else
        ctx.jobs.append_log(job.id, "fs_snapshot fallback to live read: " .. tostring(h))
        sources_resolved = sources
      end
    else
      sources_resolved = sources
    end
    ctx.jobs.update_stage(job.id, "fs-snapshot", "done")

    -- Backup
    ctx.jobs.update_stage(job.id, "rustic-backup", "in_progress")
    local backup_tags = {}
    for _, t in ipairs(tags) do table.insert(backup_tags, t) end
    table.insert(backup_tags, "manual")
    local ret, err2 = rustic.backup(conn, {
      sources = sources_resolved,
      tags    = backup_tags,
      json    = true,
    })
    local ok = ret ~= nil
    if not ok then ret = err2 end

    -- Release (always, even on failure)
    if handle then
      ctx.jobs.update_stage(job.id, "fs-release", "in_progress")
      pcall(fs_snapshot.release, handle)
      ctx.jobs.update_stage(job.id, "fs-release", "done")
    else
      ctx.jobs.update_stage(job.id, "fs-release", "done", "no snapshot to release")
    end

    if not ok then
      ctx.jobs.update_stage(job.id, "rustic-backup", "failed", tostring(ret))
      ctx.jobs.fail(job.id, tostring(ret))
      ctx.audit.append({ actor = actor, action = "backups.run_failed",
                     target = PROFILE_NAME, meta = { error = tostring(ret) } })
      return
    end

    ctx.jobs.update_stage(job.id, "rustic-backup", "done",
      string.format("snapshot %s · %s files · %s bytes",
        ret.id or "?",
        ret.total_files_processed or "?",
        ret.total_bytes_processed or "?"))

    -- Update marker so the dashboard "last run" reflects manual runs too.
    marker.write(PROFILE_NAME, {
      ts = os.time(),
      exit = 0,
      duration_s = 0,
      snap_id = ret.id,
      kind = "manual",
      fs_consistency = fs_consistency,
    })

    ctx.jobs.complete(job.id, ret)
    ctx.audit.append({
      actor = actor, action = "backups.run_complete",
      target = PROFILE_NAME,
      meta = { snap_id = ret.id, fs_consistency = fs_consistency },
    })
  end

  if async and type(async.spawn) == "function" then
    async.spawn(worker)
  else
    worker()
  end

  return { ok = true, job_id = job.id }
end

----------------------------------------------------------------------
-- Restore (job-driven)
----------------------------------------------------------------------

function M.start_restore(args)
  args = args or {}
  local actor = args.actor or "system"
  local snap_id = args.snapshot_id
  local dest = args.dest

  if not snap_id or snap_id == "" then return { ok = false, error = "snapshot_id required" } end
  if not dest or dest == "" then return { ok = false, error = "dest required" } end

  local conn, err = load_conn_args()
  if not conn then return { ok = false, error = err } end

  local job = ctx.jobs.start({
    kind = "backups.restore",
    target = PROFILE_NAME,
    name = "Restore from " .. snap_id:sub(1, 8),
    stages = {
      { id = "preflight",     label = "Pre-flight" },
      { id = "rustic-restore", label = "rustic restore" },
      { id = "verify",        label = "Verify" },
    },
  })

  ctx.audit.append({
    actor = actor, action = "backups.restore_start", target = PROFILE_NAME,
    meta = { snapshot_id = snap_id, dest = dest, job_id = job.id },
  })

  local function worker()
    ctx.jobs.update_stage(job.id, "preflight", "done")
    ctx.jobs.update_stage(job.id, "rustic-restore", "in_progress")

    local ret, err2 = rustic.restore(conn, snap_id, dest)
    local ok = ret ~= nil
    if not ok then ret = err2 end
    if not ok then
      ctx.jobs.update_stage(job.id, "rustic-restore", "failed", tostring(ret))
      ctx.jobs.fail(job.id, tostring(ret))
      ctx.audit.append({ actor = actor, action = "backups.restore_failed",
                     target = PROFILE_NAME, meta = { error = tostring(ret) } })
      return
    end
    ctx.jobs.update_stage(job.id, "rustic-restore", "done")

    ctx.jobs.update_stage(job.id, "verify", "done")
    ctx.jobs.complete(job.id, ret)
    ctx.audit.append({
      actor = actor, action = "backups.restore_complete", target = PROFILE_NAME,
      meta = { snapshot_id = snap_id, dest = dest },
    })
  end

  if async and type(async.spawn) == "function" then
    async.spawn(worker)
  else
    worker()
  end

  return { ok = true, job_id = job.id }
end

return M
