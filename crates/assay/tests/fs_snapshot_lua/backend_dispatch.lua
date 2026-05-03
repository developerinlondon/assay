-- assay.fs_snapshot backend-dispatch tests.
-- Mocks shell.exec to drive each branch (btrfs / zfs / none) without
-- needing a real btrfs / zfs filesystem.

local fs_snapshot = require("assay.fs_snapshot")

local function check(cond, msg)
  if not cond then error(msg, 2) end
end

-- Mock shell.exec — returns scripted answers based on the command.
-- `script` is a table of { match=substring_pattern, status=N, stdout=S }
-- entries; the first matching entry is used per call.
local function mock_shell(script)
  local captured = {}
  local orig = shell.exec
  shell.exec = function(cmd, opts)
    table.insert(captured, { cmd = cmd, opts = opts })
    for _, entry in ipairs(script) do
      if string.find(cmd, entry.match, 1, true) then
        return { status = entry.status or 0, stdout = entry.stdout or "", stderr = entry.stderr or "" }
      end
    end
    return { status = 0, stdout = "", stderr = "" }
  end
  return captured, function() shell.exec = orig end
end

local function with_mock(script, fn)
  local cap, restore = mock_shell(script)
  local ok, ret = pcall(fn)
  restore()
  if not ok then error(ret) end
  return ret, cap
end

-- ── detect: btrfs / zfs / ext4 ──────────────────────────────────────
do
  local r = with_mock({
    { match = "id -u",   status = 0, stdout = "1000\n" },
    { match = "findmnt", status = 0, stdout = "/dev/sda3 btrfs\n" },
  }, function() return fs_snapshot.detect("/var/lib/machines") end)
  check(r.backend == "btrfs", "expected btrfs, got " .. tostring(r.backend))
  check(r.source  == "/dev/sda3", "btrfs source")
  check(r.fstype  == "btrfs", "btrfs fstype")
end

do
  local r = with_mock({
    { match = "id -u",   status = 0, stdout = "1000\n" },
    { match = "findmnt", status = 0, stdout = "tank/data zfs\n" },
  }, function() return fs_snapshot.detect("/srv/data") end)
  check(r.backend == "zfs", "expected zfs, got " .. tostring(r.backend))
  check(r.source  == "tank/data", "zfs source")
end

do
  local r = with_mock({
    { match = "id -u",   status = 0, stdout = "1000\n" },
    { match = "findmnt", status = 0, stdout = "/dev/sda1 ext4\n" },
  }, function() return fs_snapshot.detect("/etc") end)
  check(r.backend == "none", "expected none for ext4")
end

-- ── take: btrfs branch issues `btrfs subvolume snapshot -r` ─────────
do
  local _, cap = with_mock({
    { match = "id -u",   status = 0, stdout = "1000\n" },  -- non-root → sudo prefix
    { match = "findmnt", status = 0, stdout = "/dev/sda3 btrfs\n" },
    { match = "btrfs subvolume snapshot", status = 0, stdout = "" },
  }, function() return fs_snapshot.take("manual", "/var/lib/machines") end)
  -- find the btrfs cmd in the captured calls
  local found = false
  for _, c in ipairs(cap) do
    if string.find(c.cmd, "btrfs subvolume snapshot -r", 1, true) then
      check(string.find(c.cmd, "sudo -n", 1, true), "expected sudo prefix as non-root")
      check(string.find(c.cmd, "/var/lib/machines", 1, true), "source path in cmd")
      check(string.find(c.cmd, ".assay-snap-manual-", 1, true), "snap path naming")
      found = true
    end
  end
  check(found, "expected btrfs snapshot command in captured calls")
end

-- ── take: zfs branch issues `zfs snapshot tank/data@<id>` ───────────
do
  local _, cap = with_mock({
    { match = "id -u",     status = 0, stdout = "0\n" },     -- root → no sudo
    { match = "findmnt",   status = 0, stdout = "tank/data zfs\n" },
    { match = "zfs snapshot", status = 0, stdout = "" },
  }, function() return fs_snapshot.take("daily", "/srv/data") end)
  local found = false
  for _, c in ipairs(cap) do
    if string.find(c.cmd, "zfs snapshot", 1, true) then
      check(not string.find(c.cmd, "sudo -n", 1, true), "no sudo prefix as root")
      check(string.find(c.cmd, "tank/data@daily-", 1, true), "snap ref shape: " .. c.cmd)
      found = true
    end
  end
  check(found, "expected zfs snapshot command")
end

-- ── take: none backend returns no-op handle pointing at the path ────
do
  local r = with_mock({
    { match = "id -u",   status = 0, stdout = "0\n" },
    { match = "findmnt", status = 0, stdout = "/dev/sda1 ext4\n" },
  }, function() return fs_snapshot.take("x", "/etc") end)
  check(r.backend == "none", "none backend")
  check(r.path == "/etc", "no-op path is original")
end

-- ── release: btrfs branch issues `btrfs subvolume delete` ───────────
do
  local _, cap = with_mock({
    { match = "id -u", status = 0, stdout = "0\n" },
    { match = "btrfs subvolume delete", status = 0, stdout = "" },
  }, function()
    return fs_snapshot.release({ backend = "btrfs", path = "/foo/.assay-snap-x-1" })
  end)
  local found = false
  for _, c in ipairs(cap) do
    if string.find(c.cmd, "btrfs subvolume delete", 1, true) then found = true end
  end
  check(found, "expected btrfs delete cmd")
end

-- ── release: none backend is a no-op ────────────────────────────────
do
  local r = fs_snapshot.release({ backend = "none", path = "/x" })
  check(r.ok == true, "none release is ok")
end

-- ── with_snapshot brackets correctly + releases on error ────────────
do
  local released = false
  local orig_release = fs_snapshot.release
  fs_snapshot.release = function(h) released = true; return { ok = true } end

  local _, cap = with_mock({
    { match = "id -u",   status = 0, stdout = "0\n" },
    { match = "findmnt", status = 0, stdout = "/dev/sda1 ext4\n" },
  }, function()
    local ok = pcall(fs_snapshot.with_snapshot, "x", "/x", function(_h) error("boom") end)
    check(not ok, "wrapped fn error should propagate")
  end)
  fs_snapshot.release = orig_release
  check(released, "release should still run on error")
end

print("fs_snapshot backend_dispatch.lua: 8 cases passed")
