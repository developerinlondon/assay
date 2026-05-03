--- @module assay.rustic
--- @description rustic backup CLI wrapper. Snapshots, backup, restore, init, check. Repository + credentials passed via env (kept off the cmdline).
--- @keywords rustic, backup, restic, snapshot, restore, repository, s3, b2, azure, gcs, retention
--- @quickref rustic.snapshots(opts) -> [snap]|nil, err | List all snapshots in a repo
--- @quickref rustic.snapshot_detail(opts, id) -> snap|nil, err | Detail for one snapshot
--- @quickref rustic.init(opts) -> {ok}|nil, err | Initialise a fresh repository
--- @quickref rustic.backup(opts, args) -> {ok, summary?}|nil, err | Run a backup
--- @quickref rustic.restore(opts, id, target) -> {ok}|nil, err | Restore a snapshot
--- @quickref rustic.check(opts) -> {ok}|nil, err | Verify repo connectivity + integrity
--- @quickref rustic.forget(opts, args) -> {ok, removed?}|nil, err | Apply retention policy
---
--- Connection table (`opts`) shape:
---   repository           string  required, e.g. "s3:https://...", "/srv/restic", "b2:..."
---   password             string  required
---   region               string  optional (S3)
---   access_key_id        string  optional (S3)
---   secret_access_key    string  optional (S3)
---
--- Secrets travel as environment variables (`RUSTIC_PASSWORD`,
--- `AWS_ACCESS_KEY_ID`, …) so they don't appear in /proc/<pid>/cmdline.

local M = {}

-- Build the env table rustic reads for repo + creds.
local function build_env(opts)
  local env_t = {}
  if opts.repository           then env_t.RUSTIC_REPOSITORY     = opts.repository           end
  if opts.password             then env_t.RUSTIC_PASSWORD       = opts.password             end
  if opts.access_key_id        then env_t.AWS_ACCESS_KEY_ID     = opts.access_key_id        end
  if opts.secret_access_key    then env_t.AWS_SECRET_ACCESS_KEY = opts.secret_access_key    end
  if opts.region               then env_t.AWS_REGION            = opts.region               end
  return env_t
end

local function shell_quote(s)
  return "'" .. tostring(s):gsub("'", [['\'']]) .. "'"
end

local function run(opts, args, exec_opts)
  exec_opts = exec_opts or {}
  exec_opts.env = build_env(opts)
  local cmd = "rustic " .. table.concat(args, " ")
  return shell.exec(cmd, exec_opts)
end

local function err_from_result(name, r)
  return ("rustic %s failed (exit %s): %s"):format(
    name,
    tostring(r and r.status),
    (r and r.stderr) or "unknown")
end

local function decode_json_or_err(r, name)
  if not r or r.status ~= 0 then return nil, err_from_result(name, r) end
  local ok, parsed = pcall(json.decode, r.stdout or "")
  if not ok then
    return nil, ("rustic %s: parse error: %s"):format(name, tostring(parsed))
  end
  return parsed
end

--- List all snapshots. Returns the JSON array rustic emits, or nil+err.
function M.snapshots(opts)
  local r = run(opts, { "snapshots", "--json" }, { timeout = opts.timeout or 60 })
  return decode_json_or_err(r, "snapshots")
end

--- Detail for snapshot `id`. Returns the JSON object/array rustic emits.
function M.snapshot_detail(opts, id)
  local r = run(opts,
    { "snapshots", shell_quote(id), "--json" },
    { timeout = opts.timeout or 60 })
  return decode_json_or_err(r, "snapshot_detail")
end

--- Verify connectivity + integrity. Read-only.
function M.check(opts)
  local r = run(opts, { "check" }, { timeout = opts.timeout or 600 })
  if not r or r.status ~= 0 then return nil, err_from_result("check", r) end
  return { ok = true }
end

--- Initialise a fresh repository at `opts.repository`.
function M.init(opts)
  local r = run(opts, { "init" }, { timeout = opts.timeout or 120 })
  if not r or r.status ~= 0 then return nil, err_from_result("init", r) end
  return { ok = true, stdout = r.stdout }
end

--- Run a backup.
---   args = {
---     sources = { "/etc", "/var/lib/foo" },   -- required
---     tags    = { "host", "daily" },          -- optional
---     exclude = { "/var/cache" },             -- optional
---     json    = true,                         -- ask rustic to emit JSON summary
---     timeout = 7200,                         -- seconds, default 7200
---   }
function M.backup(opts, args)
  args = args or {}
  local cmd = { "backup" }
  for _, t in ipairs(args.tags or {})    do cmd[#cmd+1] = "--tag";     cmd[#cmd+1] = shell_quote(t) end
  for _, e in ipairs(args.exclude or {}) do cmd[#cmd+1] = "--exclude"; cmd[#cmd+1] = shell_quote(e) end
  if args.json then cmd[#cmd+1] = "--json" end
  for _, s in ipairs(args.sources or {}) do cmd[#cmd+1] = shell_quote(s) end

  local r = run(opts, cmd, { timeout = args.timeout or 7200 })
  if not r or r.status ~= 0 then return nil, err_from_result("backup", r) end

  local out = { ok = true, stdout = r.stdout, stderr = r.stderr }
  if args.json then
    local ok, summary = pcall(json.decode, r.stdout or "")
    if ok then out.summary = summary end
  end
  return out
end

--- Restore snapshot `id` to `target` directory.
---   args = { dry_run = bool, timeout = seconds }
function M.restore(opts, id, target, args)
  args = args or {}
  local cmd = { "restore", shell_quote(id), shell_quote(target) }
  if args.dry_run then cmd[#cmd+1] = "--dry-run" end

  local r = run(opts, cmd, { timeout = args.timeout or 7200 })
  if not r or r.status ~= 0 then return nil, err_from_result("restore", r) end
  return { ok = true, stdout = r.stdout, stderr = r.stderr }
end

--- Apply retention policy via rustic forget.
---   args = {
---     keep_daily = 7, keep_weekly = 4, keep_monthly = 6, keep_yearly = 2,
---     prune = false,                  -- if true, also free space (slow)
---     tags = { ... },                 -- optional filter
---     json = true,                    -- structured output
---   }
function M.forget(opts, args)
  args = args or {}
  local cmd = { "forget" }
  for _, k in ipairs({
    "keep_daily", "keep_weekly", "keep_monthly", "keep_yearly", "keep_hourly", "keep_last",
  }) do
    if args[k] then
      cmd[#cmd+1] = "--" .. k:gsub("_", "-")
      cmd[#cmd+1] = tostring(args[k])
    end
  end
  for _, t in ipairs(args.tags or {}) do cmd[#cmd+1] = "--tag"; cmd[#cmd+1] = shell_quote(t) end
  if args.prune then cmd[#cmd+1] = "--prune" end
  if args.json  then cmd[#cmd+1] = "--json"  end

  local r = run(opts, cmd, { timeout = args.timeout or 1800 })
  if not r or r.status ~= 0 then return nil, err_from_result("forget", r) end

  local out = { ok = true, stdout = r.stdout }
  if args.json then
    local ok, parsed = pcall(json.decode, r.stdout or "")
    if ok then out.removed = parsed end
  end
  return out
end

return M
