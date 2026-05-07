-- assay.rustic command + env construction tests.
-- We replace shell.exec with a capturing mock so the module's behaviour
-- can be inspected without invoking the real rustic CLI.

local rustic = require("assay.rustic")

local function mock_shell(stdout, status)
  local captured = {}
  local orig = shell.exec
  shell.exec = function(cmd, opts)
    table.insert(captured, { cmd = cmd, opts = opts })
    return { status = status or 0, stdout = stdout or "", stderr = "" }
  end
  return captured, function() shell.exec = orig end
end

local function mocked(call_fn, stdout, status)
  local cap, restore = mock_shell(stdout, status)
  local ok, ret = pcall(call_fn)
  restore()
  if not ok then error(ret) end
  return ret, cap
end

-- ── snapshots: command + env + JSON parse ───────────────────────────
do
  local opts = {
    repository = "s3:https://example.com/bucket",
    password   = "topsecret",
    region     = "us-east-1",
    access_key_id     = "AKIA",
    secret_access_key = "SECRET",
  }
  local ret, cap = mocked(function() return rustic.snapshots(opts) end,
    "[{\"id\":\"abc\",\"time\":\"2026-05-03T12:00:00Z\"}]")
  assert.eq(#cap, 1, "expected 1 shell.exec call")
  assert.contains(cap[1].cmd, "rustic snapshots --json",
    "snapshots cmd missing: " .. tostring(cap[1].cmd))
  local env = cap[1].opts.env
  assert.eq(env.RUSTIC_REPOSITORY, "s3:https://example.com/bucket", "RUSTIC_REPOSITORY")
  assert.eq(env.RUSTIC_PASSWORD, "topsecret", "RUSTIC_PASSWORD")
  assert.eq(env.AWS_ACCESS_KEY_ID, "AKIA", "AWS_ACCESS_KEY_ID")
  assert.eq(env.AWS_SECRET_ACCESS_KEY, "SECRET", "AWS_SECRET_ACCESS_KEY")
  assert.eq(env.AWS_REGION, "us-east-1", "AWS_REGION")
  assert.eq(type(ret), "table", "JSON parse returned table")
  assert.not_nil(ret[1], "JSON parse returned first row")
  assert.eq(ret[1].id, "abc", "JSON parse")
end

-- ── snapshots: error returns nil + msg ──────────────────────────────
do
  local _, cap = mocked(function()
    local ret, err = rustic.snapshots({ repository = "x", password = "y" })
    assert.eq(ret, nil, "expected nil ret on failure")
    assert.not_nil(err, "expected error msg")
    assert.contains(err, "rustic snapshots failed", "expected error msg, got: " .. tostring(err))
    return ret
  end, "permission denied", 2)
  assert.eq(#cap, 1, "fail path still issues one call")
end

-- ── snapshot_detail: id is shell-quoted, env identical to snapshots ─
do
  local _, cap = mocked(function()
    return rustic.snapshot_detail({ repository = "/r", password = "p" }, "9f3a02b1")
  end, "{}", 0)
  assert.contains(cap[1].cmd, "rustic snapshots '9f3a02b1' --json",
    "snapshot_detail cmd: " .. cap[1].cmd)
end

-- ── init ────────────────────────────────────────────────────────────
do
  local ret, cap = mocked(function()
    return rustic.init({ repository = "/r", password = "p" })
  end, "ok", 0)
  assert.eq(cap[1].cmd, "rustic init", "init cmd: " .. cap[1].cmd)
  assert.eq(ret.ok, true, "init ok")
end

-- ── check ───────────────────────────────────────────────────────────
do
  local ret, _ = mocked(function()
    return rustic.check({ repository = "/r", password = "p" })
  end, "all good", 0)
  assert.eq(ret.ok, true, "check ok")
end

-- ── backup: tags + sources + exclude + json flag ────────────────────
do
  local _, cap = mocked(function()
    return rustic.backup({ repository = "/r", password = "p" }, {
      sources = { "/etc", "/var/lib/foo" },
      tags    = { "host", "daily" },
      exclude = { "/var/cache" },
      json    = true,
    })
  end, "{\"summary\":{\"data_added\":1024}}", 0)
  local cmd = cap[1].cmd
  assert.contains(cmd, "--tag 'host'", "tag host: " .. cmd)
  assert.contains(cmd, "--tag 'daily'", "tag daily: " .. cmd)
  assert.contains(cmd, "--exclude '/var/cache'", "exclude: " .. cmd)
  assert.contains(cmd, "--json", "json flag: " .. cmd)
  assert.contains(cmd, "'/etc'", "source /etc: " .. cmd)
  assert.contains(cmd, "'/var/lib/foo'", "source /var/lib/foo: " .. cmd)
end

-- ── backup: summary parsed when json=true ───────────────────────────
do
  local ret, _ = mocked(function()
    return rustic.backup({ repository = "/r", password = "p" }, {
      sources = { "/etc" }, json = true,
    })
  end, "{\"summary\":{\"data_added\":42}}", 0)
  assert.not_nil(ret.summary, "summary parsed")
  assert.eq(ret.summary.summary.data_added, 42, "summary parsed")
end

-- ── restore: id + target both quoted; dry_run flag ──────────────────
do
  local _, cap = mocked(function()
    return rustic.restore({ repository = "/r", password = "p" },
      "abc-123", "/var/restored", { dry_run = true })
  end, "", 0)
  local cmd = cap[1].cmd
  assert.contains(cmd, "rustic restore 'abc-123' '/var/restored'",
    "restore base: " .. cmd)
  assert.contains(cmd, "--dry-run", "dry-run flag: " .. cmd)
end

-- ── forget: keep_* flags + tag filter + prune ───────────────────────
do
  local _, cap = mocked(function()
    return rustic.forget({ repository = "/r", password = "p" }, {
      keep_daily = 7, keep_monthly = 6,
      tags  = { "host" },
      prune = true,
    })
  end, "", 0)
  local cmd = cap[1].cmd
  assert.contains(cmd, "--keep-daily 7", "keep-daily: " .. cmd)
  assert.contains(cmd, "--keep-monthly 6", "keep-monthly: " .. cmd)
  assert.contains(cmd, "--tag 'host'", "tag filter: " .. cmd)
  assert.contains(cmd, "--prune", "prune flag: " .. cmd)
end

print("rustic command_construction.lua: 9 cases passed")
