--- @module assay.temporal
--- @description Temporal workflow orchestration. Workflows, task queues, schedules, signals.
--- @keywords temporal, workflows, task-queues, schedules, orchestration, workflow, task-queue, schedule, signal, history, search, namespace, execution
---
--- ## Two APIs
---
--- **1. HTTP REST client (this module)** — `require("assay.temporal")` — read-only access
--- to Temporal's HTTP API. List, query, signal, and cancel workflows. Does NOT start
--- workflows or execute them. Useful for dashboards and monitoring.
---
--- **2. Native gRPC client (global)** — `temporal.connect(opts)` — requires the `temporal`
--- feature flag at compile time. Provides `start_workflow`, `signal_workflow`,
--- `query_workflow`, `describe_workflow`, `get_result`, `cancel_workflow`,
--- `terminate_workflow`. This is a **client only** — it can start and interact with
--- workflows, but cannot execute them.
---
--- ## Important: no worker runtime (yet)
---
--- Neither API includes a Temporal **worker**. A worker is a process that polls a task
--- queue and executes workflow/activity code. Without a worker registered on the task
--- queue, `start_workflow` puts the workflow in the queue but nothing processes it.
---
--- To execute workflows today, you need an external worker in Go, TypeScript, Python,
--- or another language with a Temporal SDK that includes worker support.
---
--- A native Lua worker API (`temporal.worker()`) is planned — see the temporal-worker
--- feature flag proposal. This will allow registering Lua functions as activities and
--- defining workflows entirely in Lua, with no external services needed.
---
--- @quickref c:health() -> bool | Check Temporal health
--- @quickref c:system_info() -> info | Get system information
--- @quickref c.namespaces:list() -> {namespaces} | List namespaces
--- @quickref c.namespaces:get(name) -> namespace | Get namespace by name
--- @quickref c.workflows:list(opts?) -> {executions} | List workflow executions
--- @quickref c.workflows:get(workflow_id, run_id?, opts?) -> workflow | Get workflow execution
--- @quickref c.workflows:history(workflow_id, run_id?, opts?) -> {events} | Get workflow history
--- @quickref c.workflows:signal(workflow_id, signal_name, input?, opts?) -> result | Signal a workflow
--- @quickref c.workflows:terminate(workflow_id, reason?, opts?) -> result | Terminate a workflow
--- @quickref c.workflows:cancel(workflow_id, opts?) -> result | Cancel a workflow
--- @quickref c.workflows:search(query, opts?) -> {executions} | Search workflows by query
--- @quickref c.workflows:is_running(workflow_id, opts?) -> bool | Check if workflow is running
--- @quickref c.workflows:wait_complete(workflow_id, timeout_secs, opts?) -> workflow | Wait for completion
--- @quickref c.task_queues:get(name, opts?) -> queue | Get task queue info
--- @quickref c.schedules:list(opts?) -> {schedules} | List schedules
--- @quickref c.schedules:get(schedule_id, opts?) -> schedule | Get schedule by ID

local M = {}

function M.client(url, opts)
  opts = opts or {}
  local base_url = url:gsub("/+$", "")
  local default_ns = opts.namespace or "default"
  local api_key = opts.api_key

  -- Shared HTTP helpers (plain closures capturing upvalues)

  local function headers()
    local h = { ["Content-Type"] = "application/json" }
    if api_key then
      h["Authorization"] = "Bearer " .. api_key
    end
    return h
  end

  local function resolve_ns(opts_override)
    if opts_override and opts_override.namespace then
      return opts_override.namespace
    end
    return default_ns
  end

  local function api_get(path_str)
    local resp = http.get(base_url .. path_str, { headers = headers() })
    if resp.status ~= 200 then
      error("temporal: GET " .. path_str .. " HTTP " .. resp.status .. ": " .. resp.body)
    end
    return json.parse(resp.body)
  end

  local function api_post(path_str, payload)
    local resp = http.post(base_url .. path_str, payload, { headers = headers() })
    if resp.status ~= 200 then
      error("temporal: POST " .. path_str .. " HTTP " .. resp.status .. ": " .. resp.body)
    end
    return json.parse(resp.body)
  end

  local function build_query_string(params)
    if #params == 0 then return "" end
    return "?" .. table.concat(params, "&")
  end

  -- ===== Client =====

  local c = {}

  -- ===== Top-level =====

  function c:health()
    local resp = http.get(base_url .. "/health")
    return resp.status == 200
  end

  function c:system_info()
    return api_get("/api/v1/system-info")
  end

  -- ===== Namespaces =====

  c.namespaces = {}

  function c.namespaces:list()
    return api_get("/api/v1/namespaces")
  end

  function c.namespaces:get(name)
    return api_get("/api/v1/namespaces/" .. name)
  end

  -- ===== Workflows =====

  c.workflows = {}

  function c.workflows:list(wf_opts)
    wf_opts = wf_opts or {}
    local ns = resolve_ns(wf_opts)
    local params = {}
    if wf_opts.query then params[#params + 1] = "query=" .. wf_opts.query end
    if wf_opts.page_size then params[#params + 1] = "pageSize=" .. wf_opts.page_size end
    local qs = build_query_string(params)
    return api_get("/api/v1/namespaces/" .. ns .. "/workflows" .. qs)
  end

  function c.workflows:get(workflow_id, run_id, wf_opts)
    wf_opts = wf_opts or {}
    local ns = resolve_ns(wf_opts)
    local params = {}
    if run_id then params[#params + 1] = "runId=" .. run_id end
    local qs = build_query_string(params)
    return api_get("/api/v1/namespaces/" .. ns .. "/workflows/" .. workflow_id .. qs)
  end

  function c.workflows:history(workflow_id, run_id, wf_opts)
    wf_opts = wf_opts or {}
    local ns = resolve_ns(wf_opts)
    local params = {}
    if run_id then params[#params + 1] = "runId=" .. run_id end
    if wf_opts.maximum_page_size then params[#params + 1] = "maximumPageSize=" .. wf_opts.maximum_page_size end
    local qs = build_query_string(params)
    return api_get("/api/v1/namespaces/" .. ns .. "/workflows/" .. workflow_id .. "/history" .. qs)
  end

  function c.workflows:signal(workflow_id, signal_name, input, wf_opts)
    wf_opts = wf_opts or {}
    local ns = resolve_ns(wf_opts)
    local params = {}
    if wf_opts.run_id then params[#params + 1] = "runId=" .. wf_opts.run_id end
    local qs = build_query_string(params)
    local body = { signalName = signal_name }
    if input then body.input = input end
    return api_post("/api/v1/namespaces/" .. ns .. "/workflows/" .. workflow_id .. "/signal" .. qs, body)
  end

  function c.workflows:terminate(workflow_id, reason, wf_opts)
    wf_opts = wf_opts or {}
    local ns = resolve_ns(wf_opts)
    local params = {}
    if wf_opts.run_id then params[#params + 1] = "runId=" .. wf_opts.run_id end
    local qs = build_query_string(params)
    local body = {}
    if reason then body.reason = reason end
    return api_post("/api/v1/namespaces/" .. ns .. "/workflows/" .. workflow_id .. "/terminate" .. qs, body)
  end

  function c.workflows:cancel(workflow_id, wf_opts)
    wf_opts = wf_opts or {}
    local ns = resolve_ns(wf_opts)
    local params = {}
    if wf_opts.run_id then params[#params + 1] = "runId=" .. wf_opts.run_id end
    local qs = build_query_string(params)
    return api_post("/api/v1/namespaces/" .. ns .. "/workflows/" .. workflow_id .. "/cancel" .. qs, {})
  end

  function c.workflows:search(query, wf_opts)
    wf_opts = wf_opts or {}
    local ns = resolve_ns(wf_opts)
    local params = {}
    if query then params[#params + 1] = "query=" .. query end
    if wf_opts.page_size then params[#params + 1] = "pageSize=" .. wf_opts.page_size end
    local qs = build_query_string(params)
    return api_get("/api/v1/namespaces/" .. ns .. "/workflows" .. qs)
  end

  function c.workflows:is_running(workflow_id, wf_opts)
    wf_opts = wf_opts or {}
    local wf = c.workflows:get(workflow_id, nil, wf_opts)
    if wf and wf.workflowExecutionInfo and wf.workflowExecutionInfo.status then
      return wf.workflowExecutionInfo.status == "WORKFLOW_EXECUTION_STATUS_RUNNING"
    end
    return false
  end

  function c.workflows:wait_complete(workflow_id, timeout_secs, wf_opts)
    wf_opts = wf_opts or {}
    local deadline = time() + timeout_secs
    while true do
      local wf = c.workflows:get(workflow_id, nil, wf_opts)
      if wf and wf.workflowExecutionInfo and wf.workflowExecutionInfo.status then
        if wf.workflowExecutionInfo.status ~= "WORKFLOW_EXECUTION_STATUS_RUNNING" then
          return wf
        end
      end
      if time() >= deadline then
        error("temporal: timeout waiting for workflow " .. workflow_id .. " to complete")
      end
      sleep(5)
    end
  end

  -- ===== Task Queues =====

  c.task_queues = {}

  function c.task_queues:get(name, tq_opts)
    tq_opts = tq_opts or {}
    local ns = resolve_ns(tq_opts)
    local params = {}
    if tq_opts.task_queue_type then params[#params + 1] = "taskQueueType=" .. tq_opts.task_queue_type end
    local qs = build_query_string(params)
    return api_get("/api/v1/namespaces/" .. ns .. "/task-queues/" .. name .. qs)
  end

  -- ===== Schedules =====

  c.schedules = {}

  function c.schedules:list(sched_opts)
    sched_opts = sched_opts or {}
    local ns = resolve_ns(sched_opts)
    local params = {}
    if sched_opts.maximum_page_size then params[#params + 1] = "maximumPageSize=" .. sched_opts.maximum_page_size end
    local qs = build_query_string(params)
    return api_get("/api/v1/namespaces/" .. ns .. "/schedules" .. qs)
  end

  function c.schedules:get(schedule_id, sched_opts)
    sched_opts = sched_opts or {}
    local ns = resolve_ns(sched_opts)
    return api_get("/api/v1/namespaces/" .. ns .. "/schedules/" .. schedule_id)
  end

  -- ===== Backward-compatible shims =====
  -- Old API: c:workflows(opts), c:namespaces(), c:schedules(opts) collide with sub-object names.
  -- Use __call on sub-objects for the colliding ones, __index on c for the rest.

  -- c:workflows(opts) -> c.workflows:list(opts)
  setmetatable(c.workflows, { __call = function(self, _client, o)
    return c.workflows:list(o)
  end })

  -- c:namespaces() -> c.namespaces:list()
  setmetatable(c.namespaces, { __call = function(self, _client)
    return c.namespaces:list()
  end })

  -- c:schedules(opts) -> c.schedules:list(opts)
  setmetatable(c.schedules, { __call = function(self, _client, o)
    return c.schedules:list(o)
  end })

  local compat = {
    namespace = function(self, name) return c.namespaces:get(name) end,
    workflow = function(self, wid, rid, o) return c.workflows:get(wid, rid, o) end,
    workflow_history = function(self, wid, rid, o) return c.workflows:history(wid, rid, o) end,
    signal_workflow = function(self, wid, sn, inp, o) return c.workflows:signal(wid, sn, inp, o) end,
    terminate_workflow = function(self, wid, reason, o) return c.workflows:terminate(wid, reason, o) end,
    cancel_workflow = function(self, wid, o) return c.workflows:cancel(wid, o) end,
    task_queue = function(self, name, o) return c.task_queues:get(name, o) end,
    schedule = function(self, sid, o) return c.schedules:get(sid, o) end,
    search = function(self, q, o) return c.workflows:search(q, o) end,
    is_workflow_running = function(self, wid, o) return c.workflows:is_running(wid, o) end,
    wait_workflow_complete = function(self, wid, ts, o) return c.workflows:wait_complete(wid, ts, o) end,
  }

  setmetatable(c, {
    __index = function(tbl, key)
      return compat[key]
    end,
  })

  return c
end

return M
