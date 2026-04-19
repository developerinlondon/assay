-- E2E runner for the assay-workflow dashboard.
--
-- Boots a fresh assay engine + the demo worker that emits the canonical
-- pipeline_state.steps[] shape, seeds a workflow, then runs the
-- Playwright suite against it. Exit code = the suite's exit code.
-- Used by both the `dashboard-e2e:test` moon task and the `e2e` mise
-- task, so local + CI behaviour stays identical.
--
-- This script is itself an example of the v0.12 process.* / shell.exec /
-- http surface that any consumer can use to orchestrate test fixtures
-- without dropping to bash.

-- http, env, process, shell, sleep are registered as globals by the
-- assay runtime — no `require` needed.

-- ── Resolve paths relative to the repo root ─────────────────────────
-- The script is invoked as `assay run crates/assay-workflow/tests-e2e/run.lua`
-- from the repo root (mise + moon both set cwd to the workspace root).
local ROOT = env.get("ASSAY_REPO_ROOT") or "."
local BIN = ROOT .. "/target/release/assay"
local HERE = ROOT .. "/crates/assay-workflow/tests-e2e"
local WORKER = HERE .. "/fixtures/demo-worker.lua"

local PORT = tonumber(env.get("ASSAY_E2E_PORT") or "8080")
local BASE = "http://localhost:" .. PORT
local DB = env.get("ASSAY_E2E_DB") or "/tmp/assay-e2e.sqlite"
local ENGINE_LOG = env.get("ASSAY_E2E_ENGINE_LOG") or "/tmp/assay-e2e-engine.log"
local WORKER_LOG = env.get("ASSAY_E2E_WORKER_LOG") or "/tmp/assay-e2e-worker.log"

-- ── Helpers ─────────────────────────────────────────────────────────
local function log(msg)
  io.write("[e2e] " .. msg .. "\n")
  io.flush()
end

local function fail(msg)
  -- error() unwinds through pcall in the main body so teardown still
  -- runs. The unhandled error at the end of the script forces `assay
  -- run` to exit non-zero — that's the signal CI / mise / moon need
  -- to mark the suite failed.
  error("[e2e] FATAL: " .. msg, 0)
end

-- Reset the SQLite backend on every run so demo-2 always lands as a
-- fresh PENDING row. fs.remove is a no-op-equivalent if the file
-- doesn't exist (raises, swallowed by pcall) and fs.write creates the
-- empty file the engine then opens.
local function reset_db()
  pcall(fs.remove, DB)
  fs.write(DB, "")
end

-- Poll /api/v1/version until the engine answers (or give up after 15s).
local function wait_for_engine()
  for _ = 1, 30 do
    local ok, resp = pcall(http.get, BASE .. "/api/v1/version", { timeout = 1 })
    if ok and resp and resp.status == 200 then return true end
    sleep(0.5)
  end
  return false
end

-- ── Boot ─────────────────────────────────────────────────────────────
local engine_pid, worker_pid

-- Always tear down regardless of how we exit. Lua doesn't have try /
-- finally; wrap the body in pcall and clean up + rethrow on failure.
local function teardown()
  if worker_pid then pcall(process.kill, worker_pid) end
  if engine_pid then pcall(process.kill, engine_pid) end
  -- Reap so we don't leave zombies.
  if worker_pid then pcall(process.wait, worker_pid, { timeout = 3 }) end
  if engine_pid then pcall(process.wait, engine_pid, { timeout = 3 }) end
end

local ok, err = pcall(function()
  reset_db()

  log("starting engine on :" .. PORT)
  local h = process.spawn({
    cmd = BIN,
    args = { "serve", "--port", tostring(PORT), "--backend", "sqlite://" .. DB },
    stdout = ENGINE_LOG,
    stderr = ENGINE_LOG,
  })
  engine_pid = h.pid

  if not wait_for_engine() then
    fail("engine never came up; tail of " .. ENGINE_LOG)
  end
  log("engine ready (pid " .. engine_pid .. ")")

  log("creating namespace 'demo'")
  local r = http.post(BASE .. "/api/v1/namespaces", { name = "demo" })
  if r.status >= 400 and r.status ~= 409 then
    fail("namespace create failed: " .. r.status .. " " .. (r.body or ""))
  end

  log("starting demo worker")
  local hw = process.spawn({
    cmd = BIN,
    args = { "run", WORKER },
    stdout = WORKER_LOG,
    stderr = WORKER_LOG,
  })
  worker_pid = hw.pid
  sleep(1.5) -- let the worker register before we POST the workflow

  log("seeding DemoPipeline (id=demo-2)")
  local rs = http.post(BASE .. "/api/v1/workflows", {
    workflow_type = "DemoPipeline",
    workflow_id = "demo-2",
    namespace = "demo",
    task_queue = "demo-q",
    input = {},
  })
  if rs.status >= 400 then
    fail("workflow seed failed: " .. rs.status .. " " .. (rs.body or ""))
  end

  log("running playwright")
  local res = shell.exec("npx playwright test", {
    cwd = HERE,
    env = { ASSAY_E2E_BASE = BASE, CI = env.get("CI") or "" },
  })
  io.write(res.stdout)
  io.stderr:write(res.stderr)
  if res.status ~= 0 then
    fail("playwright suite failed (exit " .. tostring(res.status) .. ")")
  end
end)

teardown()

if not ok then
  -- Re-raise so `assay run` exits non-zero. Without this, a Playwright
  -- failure would leave the script "succeeding" from CI's perspective.
  error(tostring(err), 0)
end
log("OK")
