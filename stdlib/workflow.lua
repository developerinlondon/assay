--- assay.workflow — client library for the assay workflow engine.
---
--- An assay Lua app that calls `workflow.listen()` becomes a worker for a
--- running `assay serve` instance. Workflows are written as plain Lua
--- functions; activities are also plain Lua functions. The engine drives
--- execution via two task types polled by the same worker:
---
---  - **Workflow tasks**: orchestration. The handler is re-run from scratch
---    on every "step" — each `ctx:execute_activity / sleep / wait_for_signal`
---    call either returns a value cached in the workflow's event history
---    (deterministic replay) or yields a command for the engine to schedule.
---  - **Activity tasks**: concrete work. The handler runs once; its return
---    value is persisted as `ActivityCompleted` so future replays of the
---    parent workflow short-circuit at that step.
---
--- The worker survives engine restarts, network blips, and worker crashes:
--- on resume any other worker on the queue can pick up the workflow task,
--- replay from the event log, and reach the same point. Side effects are
--- never executed twice as long as workflow code is deterministic — see
--- `ctx:side_effect` for the escape hatch when it isn't.
---
--- Usage:
---   local workflow = require("assay.workflow")
---   workflow.connect("http://assay-server:8080")
---
---   workflow.define("IngestData", function(ctx, input)
---       local data = ctx:execute_activity("fetch_s3", { bucket = input.source })
---       ctx:sleep(10)
---       ctx:execute_activity("load_warehouse", { data = data })
---       return { status = "done" }
---   end)
---
---   workflow.activity("fetch_s3", function(ctx, input)
---       return http.get("https://s3/" .. input.bucket).body
---   end)
---
---   workflow.listen({ queue = "data-pipeline" })

local M = {}

-- Internal state
local _engine_url = nil
local _workflows = {}
local _activities = {}
local _worker_id = nil
local _auth_token = nil

--- Connect to the workflow engine.
--- @param url string Engine URL (e.g. "http://localhost:8080")
--- @param opts? table Optional: { token = "Bearer ..." }
function M.connect(url, opts)
    _engine_url = url:gsub("/$", "") -- strip trailing slash
    if opts and opts.token then
        _auth_token = opts.token
    end
    -- Verify connectivity
    local resp = M._api("GET", "/health")
    if resp.status ~= 200 then
        error("workflow.connect: cannot reach engine at " .. url)
    end
    log.info("Connected to workflow engine at " .. url)
end

--- Define a workflow type. The handler receives a `ctx` whose methods
--- (`execute_activity`, `sleep`, `wait_for_signal`, `side_effect`) drive
--- the engine — see the module-level docstring for the replay model.
--- @param name string Workflow type name (matches `workflow_type` on start)
--- @param handler function(ctx, input) -> result
function M.define(name, handler)
    _workflows[name] = handler
end

--- Define an activity implementation.
--- @param name string Activity name
--- @param handler function(ctx, input) -> result
function M.activity(name, handler)
    _activities[name] = handler
end

--- Start a workflow on the engine (client-side, not as a worker).
--- @param opts table { workflow_type, workflow_id, input?, task_queue? }
--- @return table { workflow_id, run_id, status }
function M.start(opts)
    local body = {
        workflow_type = opts.workflow_type,
        workflow_id = opts.workflow_id,
        input = opts.input,
        task_queue = opts.task_queue or "default",
    }
    local resp = M._api("POST", "/workflows", body)
    if resp.status ~= 201 then
        error("workflow.start failed: " .. (resp.body or "unknown error"))
    end
    return json.parse(resp.body)
end

--- Send a signal to a running workflow.
function M.signal(workflow_id, signal_name, payload)
    local body = payload and { payload = payload } or {}
    local resp = M._api("POST", "/workflows/" .. workflow_id .. "/signal/" .. signal_name, body)
    if resp.status ~= 200 then
        error("workflow.signal failed: " .. (resp.body or "unknown error"))
    end
end

--- Query a workflow's current state.
function M.describe(workflow_id)
    local resp = M._api("GET", "/workflows/" .. workflow_id)
    if resp.status ~= 200 then
        error("workflow.describe failed: " .. (resp.body or "unknown error"))
    end
    return json.parse(resp.body)
end

--- Cancel a running workflow.
function M.cancel(workflow_id)
    local resp = M._api("POST", "/workflows/" .. workflow_id .. "/cancel")
    if resp.status ~= 200 then
        error("workflow.cancel failed: " .. (resp.body or "unknown error"))
    end
end

--- Start listening for tasks. Blocks until cancelled.
--- Polls workflow tasks first (cheap orchestration) then activity tasks.
--- @param opts table { identity?, queue?, max_concurrent_workflows?, max_concurrent_activities? }
function M.listen(opts)
    if not _engine_url then
        error("workflow.listen: call workflow.connect() first")
    end

    local queue = opts.queue or "default"
    local identity = opts.identity or
        ("assay-worker-" .. (os.hostname and os.hostname() or "unknown"))

    -- Collect registered workflow and activity names
    local wf_names, act_names = {}, {}
    for name in pairs(_workflows) do wf_names[#wf_names + 1] = name end
    for name in pairs(_activities) do act_names[#act_names + 1] = name end

    -- Register as a worker
    local reg_resp = M._api("POST", "/workers/register", {
        identity = identity,
        queue = queue,
        workflows = wf_names,
        activities = act_names,
        max_concurrent_workflows = opts.max_concurrent_workflows or 10,
        max_concurrent_activities = opts.max_concurrent_activities or 20,
    })
    if reg_resp.status ~= 200 then
        error("workflow.listen: registration failed: " .. (reg_resp.body or "unknown"))
    end
    _worker_id = json.parse(reg_resp.body).worker_id
    log.info("Registered as worker " .. _worker_id .. " on queue '" .. queue .. "'")

    -- Poll loop
    while true do
        M._api("POST", "/workers/heartbeat", { worker_id = _worker_id })

        local did_work = M._poll_workflow_task(queue) or M._poll_activity_task(queue)

        if not did_work then sleep(0.5) end
    end
end

--- Poll one workflow task and process it. Returns true if work was done.
function M._poll_workflow_task(queue)
    local resp = M._api("POST", "/workflow-tasks/poll", {
        queue = queue,
        worker_id = _worker_id,
    })
    if resp.status ~= 200 or not resp.body or resp.body == "null" or resp.body == "" then
        return false
    end
    local task = json.parse(resp.body)
    if not task or not task.workflow_id then return false end

    local commands = M._handle_workflow_task(task)

    M._api("POST", "/workflow-tasks/" .. task.workflow_id .. "/commands", {
        worker_id = _worker_id,
        commands = commands,
    })
    return true
end

--- Poll one activity task and execute it. Returns true if work was done.
function M._poll_activity_task(queue)
    local resp = M._api("POST", "/tasks/poll", {
        queue = queue,
        worker_id = _worker_id,
    })
    if resp.status ~= 200 or not resp.body or resp.body == "null" or resp.body == "" then
        return false
    end
    local task = json.parse(resp.body)
    if not task or not task.id then return false end

    local ok, result_or_err = pcall(function()
        return M._execute_activity(task)
    end)
    if ok then
        M._api("POST", "/tasks/" .. task.id .. "/complete", { result = result_or_err })
    else
        M._api("POST", "/tasks/" .. task.id .. "/fail", { error = tostring(result_or_err) })
    end
    return true
end

--- Run the workflow handler against the current event history. Returns
--- the list of commands to submit back to the engine — exactly one of:
---  * a single yielded command (`ScheduleActivity`, etc.) when the
---    handler reached an unfulfilled step
---  * `CompleteWorkflow` when the handler returned normally
---  * `FailWorkflow` when the handler raised an error
---
--- One yield per replay keeps the model simple; future versions can batch
--- commands when a workflow yields multiple parallel awaits in one go.
function M._handle_workflow_task(task)
    local handler = _workflows[task.workflow_type]
    if not handler then
        return {{
            type = "FailWorkflow",
            error = "no workflow handler registered for type: " .. tostring(task.workflow_type),
        }}
    end

    local ctx = M._make_workflow_ctx(task.workflow_id, task.history or {})
    local co = coroutine.create(function() return handler(ctx, task.input) end)

    local ok, yielded_or_returned = coroutine.resume(co)
    if not ok then
        return {{ type = "FailWorkflow", error = tostring(yielded_or_returned) }}
    end
    if coroutine.status(co) == "dead" then
        return {{ type = "CompleteWorkflow", result = yielded_or_returned }}
    end
    -- Yielded a command — submit it and let a subsequent replay continue
    return { yielded_or_returned }
end

--- Build the workflow ctx object used during replay. Each `ctx:` call
--- increments an internal seq counter and either returns the cached
--- value from history (replay) or yields a command (first time through).
function M._make_workflow_ctx(workflow_id, history)
    -- Pre-index history by activity_seq for O(1) lookups during replay
    local activity_results = {}
    for _, event in ipairs(history) do
        local p = event.payload
        if event.event_type == "ActivityCompleted" and p and p.activity_seq then
            activity_results[p.activity_seq] = { ok = true, value = p.result }
        elseif event.event_type == "ActivityFailed" and p and p.activity_seq then
            activity_results[p.activity_seq] = { ok = false, err = p.error }
        end
    end

    local seq = 0
    local ctx = { workflow_id = workflow_id }

    --- Schedule an activity and (synchronously, for the workflow author)
    --- return its result. On replay, returns the cached result from
    --- history; on first execution at this seq, yields a ScheduleActivity
    --- command and the workflow run ends until the activity completes
    --- and the workflow becomes dispatchable again.
    function ctx:execute_activity(name, input, opts)
        seq = seq + 1
        local r = activity_results[seq]
        if r then
            if r.ok then return r.value end
            error("activity '" .. name .. "' failed: " .. tostring(r.err))
        end
        coroutine.yield({
            type = "ScheduleActivity",
            seq = seq,
            name = name,
            task_queue = (opts and opts.task_queue) or "default",
            input = input,
            max_attempts = opts and opts.max_attempts,
            initial_interval_secs = opts and opts.initial_interval_secs,
            backoff_coefficient = opts and opts.backoff_coefficient,
            start_to_close_secs = opts and opts.start_to_close_secs,
            heartbeat_timeout_secs = opts and opts.heartbeat_timeout_secs,
        })
        -- Unreachable in single-yield mode — yielding ends this replay.
        error("workflow ctx: yielded but resumed unexpectedly")
    end

    return ctx
end

--- Execute an activity task (the concrete work; runs once, result persisted).
function M._execute_activity(task)
    local handler = _activities[task.name]
    if not handler then
        error("No handler registered for activity: " .. (task.name or "?"))
    end
    local input = task.input
    if type(input) == "string" then input = json.parse(input) end

    local ctx = M._make_activity_ctx(task)
    return handler(ctx, input)
end

--- Activity ctx — minimal, just exposes a heartbeat so long-running
--- activities can prove they're still alive.
function M._make_activity_ctx(task)
    local ctx = {}
    function ctx:heartbeat(details)
        M._api("POST", "/tasks/" .. task.id .. "/heartbeat", { details = details })
    end
    return ctx
end

--- Internal: make an API call to the engine.
function M._api(method, path, body)
    local url = _engine_url .. "/api/v1" .. path
    local opts = { headers = {} }

    if _auth_token then
        opts.headers["Authorization"] = "Bearer " .. _auth_token
    end

    if method == "GET" then
        return http.get(url, opts)
    elseif method == "POST" then
        return http.post(url, body or {}, opts)
    elseif method == "DELETE" then
        return http.delete(url, opts)
    else
        error("workflow._api: unsupported method: " .. method)
    end
end

return M
