--- @module assay.engine.workflow
--- @description Lua client for assay-engine's workflow module mounted at `/api/v1/engine/workflow/*`. Covers workflow CRUD + signal/cancel/state, schedules, namespaces, workers, queues, plus a worker-mode (`define` + `activity` + `listen`) that turns the calling process into an assay worker.
--- @keywords workflow, engine, scheduler, signal, queue, namespace, worker
--- @quickref workflow.client(opts) -> client | Build a workflow client
--- @quickref c:start({workflow_type, workflow_id, namespace?, input?, task_queue?}) -> {workflow_id, run_id, status} | Start a workflow
--- @quickref c:signal(workflow_id, signal_name, payload?) -> nil | Send a signal
--- @quickref c:describe(workflow_id) -> WorkflowRecord | Query workflow state
--- @quickref c:cancel(workflow_id) -> nil | Request cancellation
--- @quickref c:terminate(workflow_id, reason?) -> nil | Hard-terminate (no graceful cleanup)
--- @quickref c:list({namespace?, status?, type?, search_attrs?, limit?, offset?}) -> [WorkflowRecord] | List workflows
--- @quickref c:get_events(workflow_id) -> [Event] | Full event history
--- @quickref c:get_state(workflow_id, name?) -> table|any | Latest snapshot
--- @quickref c:list_children(workflow_id) -> [WorkflowRecord] | Child workflows
--- @quickref c:continue_as_new(workflow_id, input?) -> WorkflowRecord | Client-side continue-as-new
--- @quickref c.namespaces:create(name) | Create a namespace
--- @quickref c.namespaces:list() -> [NamespaceRecord] | List namespaces
--- @quickref c.namespaces:stats(name) -> NamespaceStats | Per-namespace counters
--- @quickref c.namespaces:delete(name) | Delete a namespace
--- @quickref c.schedules:create(opts) -> ScheduleRecord | Create a cron schedule
--- @quickref c.schedules:list({namespace?}) -> [ScheduleRecord] | List schedules
--- @quickref c.schedules:describe(name, {namespace?}) -> ScheduleRecord|nil | Describe one
--- @quickref c.schedules:patch(name, patch, {namespace?}) -> ScheduleRecord | Patch in place
--- @quickref c.schedules:pause(name, {namespace?}) | Pause schedule
--- @quickref c.schedules:resume(name, {namespace?}) | Resume schedule
--- @quickref c.schedules:delete(name, {namespace?}) | Delete schedule
--- @quickref c.workers:list({namespace?}) -> [Worker] | List registered workers
--- @quickref c.queues:stats({namespace?}) -> [QueueStats] | Pending/running counts per queue
--- @quickref c:register_workflow(name, handler) | Register a workflow handler (worker mode)
--- @quickref c:register_activity(name, handler) | Register an activity handler (worker mode)
--- @quickref c:listen({queue?, namespace?, identity?, max_concurrent_workflows?, max_concurrent_activities?}) | Become a worker on (namespace, queue)

local worker_mod = require("assay.engine.workflow.worker")

local M = {}

local function trim_slash(s) return (s or ""):gsub("/+$", "") end

local function url_encode(s)
  return (tostring(s):gsub("([^A-Za-z0-9%-_.~])", function(ch)
    return string.format("%%%02X", string.byte(ch))
  end))
end

--- Build a workflow client.
---
--- opts:
---   engine_url       (string, required)  base URL of the assay-engine
---   api_key          (string, optional)  admin bearer; ASSAY_ADMIN_KEY fallback
---   session_cookie   (string, optional)  session cookie for user-flow auth
function M.client(opts)
  opts = opts or {}
  local engine_url = trim_slash(opts.engine_url or env.get("ASSAY_ENGINE_URL") or "")
  if engine_url == "" then
    error("assay.engine.workflow: engine_url required (or set ASSAY_ENGINE_URL)")
  end
  local api_key = opts.api_key or env.get("ASSAY_ADMIN_KEY")
  local session_cookie = opts.session_cookie

  local function build_headers()
    local h = { ["Content-Type"] = "application/json" }
    if api_key and api_key ~= "" then
      h["Authorization"] = "Bearer " .. api_key
    end
    if session_cookie and session_cookie ~= "" then
      h["Cookie"] = "assay_session=" .. session_cookie
    end
    return h
  end

  local BASE = "/api/v1/engine/workflow"

  --- HTTP wrapper. `path` is relative to /api/v1/engine/workflow.
  local function api_call(method, path, body)
    local url = engine_url .. BASE .. path
    local opts2 = { headers = build_headers() }
    if method == "GET" then return http.get(url, opts2)
    elseif method == "POST" then return http.post(url, body or {}, opts2)
    elseif method == "PATCH" then return http.patch(url, body or {}, opts2)
    elseif method == "PUT" then return http.put(url, body or {}, opts2)
    elseif method == "DELETE" then return http.delete(url, opts2)
    else error("engine.workflow: unsupported method: " .. method)
    end
  end

  -- Internal handle held by the worker module so its loop functions
  -- can hit the engine without re-deriving URL/auth state. Underscore-
  -- prefixed to mark private — public API below.
  local client = {
    _engine_url = engine_url,
    _api = api_call,
    _workflows = {},
    _activities = {},
    _worker_id = nil,
  }

  local function expect(resp, status, fn_name)
    if type(status) == "table" then
      for _, s in ipairs(status) do
        if resp.status == s then return end
      end
      error(fn_name .. ": HTTP " .. resp.status .. ": " .. (resp.body or ""))
    end
    if resp.status ~= status then
      error(fn_name .. ": HTTP " .. resp.status .. ": " .. (resp.body or ""))
    end
  end

  -- ===== Workflow lifecycle =====

  --- Start a workflow. `opts.namespace` defaults to "main" engine-side.
  function client:start(start_opts)
    local body = {
      workflow_type = start_opts.workflow_type,
      workflow_id = start_opts.workflow_id,
      namespace = start_opts.namespace,
      input = start_opts.input,
      task_queue = start_opts.task_queue or "default",
      search_attributes = start_opts.search_attributes,
    }
    local resp = api_call("POST", "/workflows", body)
    expect(resp, 201, "engine.workflow.start")
    return json.parse(resp.body)
  end

  function client:signal(workflow_id, signal_name, payload)
    local body = payload and { payload = payload } or {}
    local resp = api_call("POST",
      "/workflows/" .. url_encode(workflow_id) .. "/signal/" .. url_encode(signal_name), body)
    expect(resp, 200, "engine.workflow.signal")
  end

  function client:describe(workflow_id)
    local resp = api_call("GET", "/workflows/" .. url_encode(workflow_id))
    expect(resp, 200, "engine.workflow.describe")
    return json.parse(resp.body)
  end

  function client:cancel(workflow_id)
    local resp = api_call("POST", "/workflows/" .. url_encode(workflow_id) .. "/cancel")
    expect(resp, 200, "engine.workflow.cancel")
  end

  function client:terminate(workflow_id, reason)
    local body = reason and { reason = reason } or {}
    local resp = api_call("POST",
      "/workflows/" .. url_encode(workflow_id) .. "/terminate", body)
    expect(resp, 200, "engine.workflow.terminate")
  end

  function client:list(qopts)
    qopts = qopts or {}
    local parts = {}
    if qopts.namespace then parts[#parts + 1] = "namespace=" .. url_encode(qopts.namespace) end
    if qopts.status then parts[#parts + 1] = "status=" .. url_encode(qopts.status) end
    if qopts.type then parts[#parts + 1] = "type=" .. url_encode(qopts.type) end
    if qopts.search_attrs then
      parts[#parts + 1] = "search_attrs=" .. url_encode(json.encode(qopts.search_attrs))
    end
    if qopts.limit then parts[#parts + 1] = "limit=" .. tostring(qopts.limit) end
    if qopts.offset then parts[#parts + 1] = "offset=" .. tostring(qopts.offset) end
    local path = "/workflows"
    if #parts > 0 then path = path .. "?" .. table.concat(parts, "&") end
    local resp = api_call("GET", path)
    expect(resp, 200, "engine.workflow.list")
    return json.parse(resp.body)
  end

  function client:get_events(workflow_id)
    local resp = api_call("GET", "/workflows/" .. url_encode(workflow_id) .. "/events")
    expect(resp, 200, "engine.workflow.get_events")
    return json.parse(resp.body)
  end

  --- Read the latest snapshot written by `ctx:register_query` handlers.
  --- With a query name returns just that value; without, the full table.
  --- Returns nil if no snapshot has been recorded yet.
  function client:get_state(workflow_id, name)
    local path = "/workflows/" .. url_encode(workflow_id) .. "/state"
    if name then path = path .. "/" .. url_encode(name) end
    local resp = api_call("GET", path)
    if resp.status == 404 then return nil end
    expect(resp, 200, "engine.workflow.get_state")
    return json.parse(resp.body)
  end

  function client:list_children(workflow_id)
    local resp = api_call("GET", "/workflows/" .. url_encode(workflow_id) .. "/children")
    expect(resp, 200, "engine.workflow.list_children")
    return json.parse(resp.body)
  end

  --- Close out `workflow_id` and start a fresh run with the same
  --- workflow type, namespace, and task queue. Client-side variant of
  --- continue-as-new; the worker-side `ctx:continue_as_new(input)` is
  --- the in-handler equivalent.
  function client:continue_as_new(workflow_id, input)
    local resp = api_call("POST",
      "/workflows/" .. url_encode(workflow_id) .. "/continue-as-new", { input = input })
    expect(resp, 201, "engine.workflow.continue_as_new")
    return json.parse(resp.body)
  end

  -- ===== Schedules =====

  client.schedules = {}

  function client.schedules:create(sopts)
    if not sopts or not sopts.name or not sopts.workflow_type or not sopts.cron_expr then
      error("engine.workflow.schedules.create: name, workflow_type, cron_expr required")
    end
    local resp = api_call("POST", "/schedules", sopts)
    expect(resp, 201, "engine.workflow.schedules.create")
    return json.parse(resp.body)
  end

  function client.schedules:list(sopts)
    local ns = (sopts and sopts.namespace) or "main"
    local resp = api_call("GET", "/schedules?namespace=" .. url_encode(ns))
    expect(resp, 200, "engine.workflow.schedules.list")
    return json.parse(resp.body)
  end

  function client.schedules:describe(name, sopts)
    local ns = (sopts and sopts.namespace) or "main"
    local resp = api_call("GET",
      "/schedules/" .. url_encode(name) .. "?namespace=" .. url_encode(ns))
    if resp.status == 404 then return nil end
    expect(resp, 200, "engine.workflow.schedules.describe")
    return json.parse(resp.body)
  end

  function client.schedules:patch(name, patch, sopts)
    local ns = (sopts and sopts.namespace) or "main"
    local resp = api_call("PATCH",
      "/schedules/" .. url_encode(name) .. "?namespace=" .. url_encode(ns), patch or {})
    expect(resp, 200, "engine.workflow.schedules.patch")
    return json.parse(resp.body)
  end

  function client.schedules:pause(name, sopts)
    local ns = (sopts and sopts.namespace) or "main"
    local resp = api_call("POST",
      "/schedules/" .. url_encode(name) .. "/pause?namespace=" .. url_encode(ns))
    expect(resp, 200, "engine.workflow.schedules.pause")
    return json.parse(resp.body)
  end

  function client.schedules:resume(name, sopts)
    local ns = (sopts and sopts.namespace) or "main"
    local resp = api_call("POST",
      "/schedules/" .. url_encode(name) .. "/resume?namespace=" .. url_encode(ns))
    expect(resp, 200, "engine.workflow.schedules.resume")
    return json.parse(resp.body)
  end

  function client.schedules:delete(name, sopts)
    local ns = (sopts and sopts.namespace) or "main"
    local resp = api_call("DELETE",
      "/schedules/" .. url_encode(name) .. "?namespace=" .. url_encode(ns))
    expect(resp, 200, "engine.workflow.schedules.delete")
  end

  -- ===== Namespaces =====

  client.namespaces = {}

  function client.namespaces:create(name)
    local resp = api_call("POST", "/namespaces", { name = name })
    expect(resp, { 200, 201 }, "engine.workflow.namespaces.create")
  end

  function client.namespaces:list()
    local resp = api_call("GET", "/namespaces")
    expect(resp, 200, "engine.workflow.namespaces.list")
    return json.parse(resp.body)
  end

  function client.namespaces:stats(name)
    local resp = api_call("GET", "/namespaces/" .. url_encode(name))
    expect(resp, 200, "engine.workflow.namespaces.stats")
    return json.parse(resp.body)
  end

  --- Alias of `stats` for symmetry with schedules.describe etc.
  function client.namespaces:describe(name) return client.namespaces:stats(name) end

  function client.namespaces:delete(name)
    local resp = api_call("DELETE", "/namespaces/" .. url_encode(name))
    expect(resp, 200, "engine.workflow.namespaces.delete")
  end

  -- ===== Workers =====

  client.workers = {}

  function client.workers:list(wopts)
    local ns = (wopts and wopts.namespace) or "main"
    local resp = api_call("GET", "/workers?namespace=" .. url_encode(ns))
    expect(resp, 200, "engine.workflow.workers.list")
    return json.parse(resp.body)
  end

  -- ===== Queues =====

  client.queues = {}

  function client.queues:stats(qopts)
    local ns = (qopts and qopts.namespace) or "main"
    local resp = api_call("GET", "/queues?namespace=" .. url_encode(ns))
    expect(resp, 200, "engine.workflow.queues.stats")
    return json.parse(resp.body)
  end

  -- ===== Worker mode (define handlers + listen) =====

  --- Register a workflow type handler. The handler receives `(ctx, input)`
  --- where `ctx` carries the deterministic-replay machinery
  --- (`ctx:execute_activity`, `ctx:sleep`, `ctx:wait_for_signal`, ...).
  --- See engine/workflow/ctx.lua for the full ctx API.
  function client:register_workflow(name, handler) self._workflows[name] = handler end

  --- Register an activity implementation. `(ctx, input) -> result`.
  --- The activity ctx exposes `ctx:heartbeat(details)` for long-running
  --- work; everything else lives on the engine.
  function client:register_activity(name, handler) self._activities[name] = handler end

  --- Become a worker on (namespace, queue). Blocks until cancelled.
  --- See engine/workflow/worker.lua for the loop body.
  function client:listen(lopts) return worker_mod.listen(self, lopts) end

  return client
end

return M
