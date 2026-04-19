## process

OS-level process management — list, check, signal, spawn, wait. The
`process` module is registered as a global; no `require` needed.

### Listing and checking

| Function                          | Returns                                              | Notes                                                                                  |
| --------------------------------- | ---------------------------------------------------- | -------------------------------------------------------------------------------------- |
| `process.list()`                  | `{ {pid, name, cmdline?}, ... }`                     | Reads `/proc` on Linux; falls back to `ps -eo pid,comm` on macOS.                      |
| `process.is_running(name)`        | `bool`                                               | True if any process with the given binary name is alive.                               |
| `process.wait_idle(names, t?, i?)`| `bool` (`true` if all idle, `false` on timeout)      | Polls until none of `names` are running. `t` = timeout secs (30), `i` = interval (1).   |

### Signalling

```lua
process.kill(pid, signal?)  -- signal defaults to 15 (SIGTERM)
```

Returns `true` on success, raises on failure. The `pid` must be `> 0`
(passing `0` or `-1` would target a process group / every permitted
process and is rejected). Common signals: `15` (SIGTERM, polite ask
to exit), `9` (SIGKILL, force).

### Spawning detached children — `process.spawn` (v0.12+)

Launch a background process and return its PID. The child runs
detached; the calling Lua script keeps executing while the child runs.
Use `process.kill` + `process.wait` to terminate and reap it.

```lua
local h = process.spawn({
  cmd    = "/path/to/binary",         -- required
  args   = { "arg1", "--flag", "v" }, -- positional args (no shell parsing)
  cwd    = "/some/dir",               -- optional; defaults to caller's
  env    = { KEY = "value", ... },    -- optional; merged onto caller's env
  stdout = "/tmp/child.log",          -- optional; file path. nil = inherit
  stderr = "/tmp/child.log",          -- optional; same.
})

print("child pid:", h.pid)
```

| Field    | Required | Default                | Notes                                              |
| -------- | -------- | ---------------------- | -------------------------------------------------- |
| `cmd`    | yes      | —                      | Binary path or PATH-resolvable name.               |
| `args`   | no       | none                   | Each entry passed as a separate argv element.      |
| `cwd`    | no       | caller's dir           | Working directory for the child.                   |
| `env`    | no       | inherit caller's env   | Extra vars merged onto the caller's environment.   |
| `stdout` | no       | inherit                | File path to redirect stdout to.                   |
| `stderr` | no       | inherit                | File path to redirect stderr to.                   |

`stdin` is always redirected from `/dev/null` — backgrounded processes
should never expect input from the caller's stdin and inheriting it
can lock the parent script.

### Waiting on a spawned child — `process.wait` (v0.12+)

Reap a previously-spawned child. Required after every `process.spawn`
to avoid zombies; safe to call on any pid in the caller's process
group.

```lua
local r = process.wait(pid, { timeout = 5 })  -- timeout optional (default: blocking)

-- r contains:
--   r.status      — exit code (0..255), or 128+sig if killed by a signal
--   r.exited      — true if the process called exit() normally
--   r.signaled    — true if the process was killed by a signal
--   r.timed_out   — true if `timeout` elapsed; status is meaningless
```

Without a `timeout`, `process.wait` blocks until the child exits.
With one, it polls every ~50ms and returns `{ timed_out = true }` if
the deadline passes — the child is still running in that case; call
`process.wait` again or `process.kill` if you want to force exit.

### Patterns

**Background a daemon, run a foreground task, clean up:**

```lua
local h = process.spawn({ cmd = "./my-daemon", stdout = "/tmp/d.log" })

-- Wait for the daemon's TCP port to come up before driving it.
for _ = 1, 30 do
  local ok = pcall(http.get, "http://localhost:8080/healthz", { timeout = 1 })
  if ok then break end
  sleep(0.5)
end

-- Run the actual work.
shell.exec("./run-tests.sh")

-- Always reap.
process.kill(h.pid)
process.wait(h.pid, { timeout = 3 })
```

**Spawn-and-detach with no follow-up:** still call `process.wait` (or
`process.kill` followed by `process.wait`) at some point — the OS
keeps the child as a zombie until it's reaped, even after it exits.

### Real-world example

The dashboard end-to-end test runner at
`crates/assay-workflow/tests-e2e/run.lua` boots an assay engine + a
demo worker via `process.spawn`, polls the engine's `/version`
endpoint, seeds a workflow, runs Playwright via `shell.exec`, then
cleans up via `process.kill` + `process.wait`. It's a complete
example of using assay as an orchestration runtime instead of bash.
