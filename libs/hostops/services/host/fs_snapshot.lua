-- services/host/fs_snapshot.lua
--
-- Thin Lua wrapper over knowhere.fs_snapshot.* (Rust-backed in
-- src/lua_bindings/fs_snapshot.rs). Used by the run-now flow to
-- bracket the rustic backup call between take() and release() so
-- the capture is crash-consistent.

local M = {}

local function bindings()
  if not (knowhere and knowhere.fs_snapshot) then
    error("knowhere.fs_snapshot binding missing — Rust foundation not registered")
  end
  return knowhere.fs_snapshot
end

--- Detect the FS backend covering `path`. Returns
---   { backend = "btrfs"|"zfs"|"none", source = "...", identifier = "..." }
function M.detect(path)
  return bindings().detect(path)
end

--- Take a read-only snapshot of `path`. Returns a handle table that
--- the caller MUST pass back to `release()` later (use a closure or
--- a finalizer to make leaks impossible).
function M.take(name, path)
  return bindings().take(name, path)
end

--- Release a previously-taken handle. No-op for `none` backend.
function M.release(handle)
  if not handle then return { ok = true } end
  return bindings().release(handle)
end

--- Reap orphan staging directories from a crashed run. Best-effort.
--- Called at engine boot.
function M.gc()
  bindings().gc()
end

--- Convenience wrapper: run `fn` with `take()` and `release()`
--- bracketing it. Returns whatever `fn` returns. Releases even if
--- `fn` errors.
function M.with_snapshot(name, path, fn)
  local handle = M.take(name, path)
  local ok, ret = pcall(fn, handle)
  M.release(handle)
  if not ok then error(ret) end
  return ret
end

return M
