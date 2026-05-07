--! Assay Lua runner for the assay-engine Lua client tests.
--!
--! Boots a temporary assay-engine, seeds it with examples/init/init.lua,
--! starts the demo worker, runs each *.test.lua through `assay run`, and
--! tears everything down. Moon/mise own the build step; this script owns
--! runtime lifecycle so the suite stays inside Assay instead of bash.

local ROOT = env.get("ASSAY_REPO_ROOT") or "."
local TESTS_DIR = ROOT .. "/crates/assay-engine/tests-lua"
local INIT_LUA = ROOT .. "/crates/assay-engine/examples/init/init.lua"

local ASSAY_BIN = env.get("ASSAY_BIN") or ROOT .. "/target/release/assay"
local ENGINE_BIN = env.get("ASSAY_ENGINE_BIN") or ROOT .. "/target/release/assay-engine"

local PORT = tonumber(env.get("ASSAY_ENGINE_LUA_PORT") or env.get("ASSAY_E2E_PORT") or "18420")
local BASE = "http://127.0.0.1:" .. tostring(PORT)
local ADMIN_KEY = env.get("ASSAY_ENGINE_LUA_ADMIN_KEY") or ("lua-tests-key-" .. tostring(time()))
local DATA_DIR = fs.tempdir()
local ENGINE_TOML = DATA_DIR .. "/engine.toml"
local ENGINE_LOG = DATA_DIR .. "/engine.log"
local WORKER_LOG = DATA_DIR .. "/worker.log"

local engine_pid
local worker_pid

local function say(msg)
  print("==> " .. msg)
end

local function tail(path, max_lines)
  local ok, body = pcall(fs.read, path)
  if not ok or not body then
    print("  <unable to read " .. path .. ">")
    return
  end
  local lines = {}
  for line in (body .. "\n"):gmatch("([^\n]*)\n") do
    table.insert(lines, line)
  end
  local start = math.max(1, #lines - max_lines + 1)
  for i = start, #lines do
    print(lines[i])
  end
end

local function cleanup()
  if worker_pid then pcall(process.kill, worker_pid) end
  if engine_pid then pcall(process.kill, engine_pid) end
  if worker_pid then pcall(process.wait, worker_pid, { timeout = 3 }) end
  if engine_pid then pcall(process.wait, engine_pid, { timeout = 3 }) end
  pcall(fs.remove, DATA_DIR)
end

local function write_engine_config()
  fs.write(ENGINE_TOML, string.format([[
[server]
bind_addr = "127.0.0.1:%d"
public_url = "http://127.0.0.1:%d"

[backend]
type = "sqlite"
data_dir = "%s"

[auth]
admin_api_keys = ["%s"]

[logging]
level = "warn"
format = "pretty"
]], PORT, PORT, DATA_DIR, ADMIN_KEY))
end

local function wait_for_engine()
  local deadline = time() + 30
  while time() < deadline do
    local ok, resp = pcall(http.get, BASE .. "/api/v1/engine/core/health", { timeout = 1 })
    if ok and resp and resp.status == 200 then
      return true
    end
    sleep(0.2)
  end
  return false
end

local function run_process(label, opts, timeout)
  say(label)
  local h = process.spawn(opts)
  local res = process.wait(h.pid, { timeout = timeout or 60 })
  if res.timed_out then
    pcall(process.kill, h.pid)
    pcall(process.wait, h.pid, { timeout = 3 })
    return false, "timed out"
  end
  if res.status ~= 0 then
    return false, "exit " .. tostring(res.status)
  end
  return true
end

local ok, err = pcall(function()
  write_engine_config()

  say("starting engine on port " .. tostring(PORT))
  local engine = process.spawn({
    cmd = ENGINE_BIN,
    args = { "serve", "--config", ENGINE_TOML },
    stdout = ENGINE_LOG,
    stderr = ENGINE_LOG,
  })
  engine_pid = engine.pid

  if not wait_for_engine() then
    error("engine did not become ready")
  end
  say("engine ready")

  local child_env = {
    ASSAY_ENGINE_URL = BASE,
    ASSAY_ADMIN_KEY = ADMIN_KEY,
  }

  local seeded, seed_err = run_process("running init.lua", {
    cmd = ASSAY_BIN,
    args = {
      "run", INIT_LUA, "--",
      "--email", "admin@example.com",
      "--password", "lua-tests-pw",
    },
    env = child_env,
  }, 30)
  if not seeded then
    error("init.lua failed: " .. tostring(seed_err))
  end

  say("spawning worker")
  local worker = process.spawn({
    cmd = ASSAY_BIN,
    args = { "run", TESTS_DIR .. "/worker.lua" },
    env = child_env,
    stdout = WORKER_LOG,
    stderr = WORKER_LOG,
  })
  worker_pid = worker.pid
  sleep(1)

  local failed = false
  for _, name in ipairs({ "core", "auth", "workflow", "e2e" }) do
    local passed, test_err = run_process("running " .. name .. ".test.lua", {
      cmd = ASSAY_BIN,
      args = { "run", TESTS_DIR .. "/" .. name .. ".test.lua" },
      env = child_env,
    }, 60)
    if not passed then
      print("  FAILED: " .. tostring(test_err))
      failed = true
    end
  end

  if failed then
    error("one or more engine Lua tests failed")
  end
end)

if not ok then
  print("engine log tail:")
  tail(ENGINE_LOG, 50)
  print("worker log tail:")
  tail(WORKER_LOG, 20)
  cleanup()
  error(err, 0)
end

cleanup()
print("all Lua tests passed")
