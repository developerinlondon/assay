--- @module assay.fs_snapshot
--- @description Read-only filesystem snapshots for crash-consistent backup capture. btrfs subvolume snapshot / zfs snapshot under the hood.
--- @keywords fs_snapshot, btrfs, zfs, subvolume, snapshot, backup, consistency, crash-consistent
--- @quickref fs_snapshot.detect(path) -> {backend, source, identifier} | Identify the FS backing `path`
--- @quickref fs_snapshot.take(name, path) -> handle | Take a read-only snapshot
--- @quickref fs_snapshot.release(handle) -> {ok} | Release a snapshot handle
--- @quickref fs_snapshot.with_snapshot(name, path, fn) -> any | Bracket `fn` between take + release
---
--- Backend selection is automatic from `findmnt`; `none` backend means
--- the path lives on a regular filesystem (ext4 / xfs / …) and reads
--- happen live with no bracketing.
---
--- Handles returned by `take()` are opaque tables MUST be passed to
--- `release()` later. `with_snapshot()` is the safe variant — it
--- releases even if the wrapped function errors.

local M = {}

local function shell_quote(s)
  return "'" .. tostring(s):gsub("'", [['\'']]) .. "'"
end

local function sudo_prefix()
  local r = shell.exec("id -u", {})
  if r and r.status == 0 and (r.stdout or ""):match("^0%s*$") then return "" end
  return "sudo -n "
end

--- Detect the FS backend covering `path`. Returns
---   { backend = "btrfs"|"zfs"|"none", source = "...", fstype = "..." }
--- where `source` is the raw mount source (e.g. "/dev/sda3" or
--- "tank/data"). `none` means a non-snapshot-capable FS.
function M.detect(path)
  local r = shell.exec(("findmnt -nT %s -o SOURCE,FSTYPE"):format(shell_quote(path)), {})
  if not r or r.status ~= 0 then
    return { backend = "none", source = "", fstype = "" }
  end
  local source, fstype = (r.stdout or ""):match("^(%S+)%s+(%S+)")
  source = source or ""
  fstype = (fstype or ""):lower()

  local backend = "none"
  if fstype == "btrfs"        then backend = "btrfs"
  elseif fstype == "zfs"      then backend = "zfs"
  elseif fstype:match("^zfs") then backend = "zfs"
  end

  return { backend = backend, source = source, fstype = fstype }
end

--- Take a read-only snapshot of `path`. Returns a handle table the
--- caller MUST pass back to `release()`. For the `none` backend it
--- returns a no-op handle pointing back at `path`.
function M.take(name, path)
  local info = M.detect(path)
  local sudo = sudo_prefix()
  local stamp = tostring(os.time())
  local snap_id = name .. "-" .. stamp

  if info.backend == "btrfs" then
    -- Snapshot a subvolume into <path>/.assay-snap-<id> (read-only).
    local snap_path = path:gsub("/+$", "") .. "/.assay-snap-" .. snap_id
    local cmd = sudo ..
      ("btrfs subvolume snapshot -r %s %s"):format(shell_quote(path), shell_quote(snap_path))
    local r = shell.exec(cmd, { timeout = 30 })
    if not r or r.status ~= 0 then
      error("fs_snapshot.take(btrfs): " .. ((r and r.stderr) or "unknown"))
    end
    return { backend = "btrfs", path = snap_path, source_path = path }
  end

  if info.backend == "zfs" then
    -- ZFS snapshots live in the pool namespace as <dataset>@<snap_id>.
    local snap_ref = info.source .. "@" .. snap_id
    local cmd = sudo .. ("zfs snapshot %s"):format(shell_quote(snap_ref))
    local r = shell.exec(cmd, { timeout = 30 })
    if not r or r.status ~= 0 then
      error("fs_snapshot.take(zfs): " .. ((r and r.stderr) or "unknown"))
    end
    -- ZFS snapshots are accessible at <mountpoint>/.zfs/snapshot/<snap_id>.
    return {
      backend     = "zfs",
      snap_ref    = snap_ref,
      source_path = path,
      path        = path:gsub("/+$", "") .. "/.zfs/snapshot/" .. snap_id,
    }
  end

  -- Non-snapshot FS: caller reads `path` live.
  return { backend = "none", path = path, source_path = path }
end

--- Release a previously-taken handle. No-op for the `none` backend.
function M.release(handle)
  if not handle or handle.backend == "none" then return { ok = true } end
  local sudo = sudo_prefix()

  if handle.backend == "btrfs" then
    local cmd = sudo ..
      ("btrfs subvolume delete %s"):format(shell_quote(handle.path))
    local r = shell.exec(cmd, { timeout = 30 })
    if not r or r.status ~= 0 then
      return { ok = false, error = (r and r.stderr) or "unknown" }
    end
    return { ok = true }
  end

  if handle.backend == "zfs" then
    local cmd = sudo .. ("zfs destroy %s"):format(shell_quote(handle.snap_ref))
    local r = shell.exec(cmd, { timeout = 30 })
    if not r or r.status ~= 0 then
      return { ok = false, error = (r and r.stderr) or "unknown" }
    end
    return { ok = true }
  end

  return { ok = false, error = "unknown backend: " .. tostring(handle.backend) }
end

--- Convenience: run `fn(handle)` between take + release. Releases even
--- if `fn` errors. Returns whatever `fn` returns.
function M.with_snapshot(name, path, fn)
  local handle = M.take(name, path)
  local ok, ret = pcall(fn, handle)
  M.release(handle)
  if not ok then error(ret) end
  return ret
end

return M
