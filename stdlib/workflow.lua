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

--- Minimal URL-encoder for query params. Covers the characters we actually
--- emit from this stdlib: namespace names, schedule names, JSON filter
--- payloads. Not a full RFC 3986 encoder.
local function url_encode(s)
    return (tostring(s):gsub("([^A-Za-z0-9%-_.~])", function(c)
        return string.format("%%%02X", string.byte(c))
    end))
end

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

--- Terminate a workflow immediately with a reason (harder than cancel: no
--- graceful handler cleanup, workflow goes straight to FAILED).
function M.terminate(workflow_id, reason)
    local body = reason and { reason = reason } or {}
    local resp = M._api("POST", "/workflows/" .. workflow_id .. "/terminate", body)
    if resp.status ~= 200 then
        error("workflow.terminate failed: " .. (resp.body or "unknown error"))
    end
end

--- List workflows in a namespace with optional filters.
--- @param opts? table { namespace?, status?, type?, search_attrs?, limit?, offset? }
--- @return table array of workflow records
function M.list(opts)
    opts = opts or {}
    local params = {}
    if opts.namespace then params[#params + 1] = "namespace=" .. url_encode(opts.namespace) end
    if opts.status then params[#params + 1] = "status=" .. url_encode(opts.status) end
    if opts.type then params[#params + 1] = "type=" .. url_encode(opts.type) end
    if opts.search_attrs then
        params[#params + 1] = "search_attrs="
            .. url_encode(json.encode(opts.search_attrs))
    end
    if opts.limit then params[#params + 1] = "limit=" .. tostring(opts.limit) end
    if opts.offset then params[#params + 1] = "offset=" .. tostring(opts.offset) end
    local path = "/workflows"
    if #params > 0 then path = path .. "?" .. table.concat(params, "&") end
    local resp = M._api("GET", path)
    if resp.status ~= 200 then
        error("workflow.list failed: " .. (resp.body or "unknown error"))
    end
    return json.parse(resp.body)
end

--- Fetch the full event history of a workflow.
function M.get_events(workflow_id)
    local resp = M._api("GET", "/workflows/" .. workflow_id .. "/events")
    if resp.status ~= 200 then
        error("workflow.get_events failed: " .. (resp.body or "unknown error"))
    end
    return json.parse(resp.body)
end

--- Read the latest snapshot written by `ctx:register_query` handlers.
--- With a query name, returns just that query's value. Without, returns
--- the full snapshot table.
--- @param workflow_id string
--- @param name? string Specific query handler name
--- @return table|any state or single value
function M.get_state(workflow_id, name)
    local path = "/workflows/" .. workflow_id .. "/state"
    if name then path = path .. "/" .. name end
    local resp = M._api("GET", path)
    if resp.status == 404 then
        return nil -- no snapshot recorded yet (or unknown query name)
    end
    if resp.status ~= 200 then
        error("workflow.get_state failed: " .. (resp.body or "unknown error"))
    end
    return json.parse(resp.body)
end

--- List a parent workflow's child workflows.
function M.list_children(workflow_id)
    local resp = M._api("GET", "/workflows/" .. workflow_id .. "/children")
    if resp.status ~= 200 then
        error("workflow.list_children failed: " .. (resp.body or "unknown error"))
    end
    return json.parse(resp.body)
end

--- Close out `workflow_id` and start a fresh run with the same type,
--- namespace, and task queue. This is the *client-side* variant of
--- continue-as-new: called from outside the workflow handler. The
--- worker-side `ctx:continue_as_new(input)` does the same thing from
--- inside a workflow handler.
--- @param workflow_id string The workflow to close out
--- @param input? any JSON-encodable input for the new run
--- @return table workflow record for the new run
function M.continue_as_new(workflow_id, input)
    local body = { input = input }
    local resp = M._api(
        "POST",
        "/workflows/" .. workflow_id .. "/continue-as-new",
        body
    )
    if resp.status ~= 201 then
        error("workflow.continue_as_new failed: " .. (resp.body or "unknown error"))
    end
    return json.parse(resp.body)
end

-- ── Schedules ───────────────────────────────────────────────
--- Schedule CRUD + lifecycle. Each call returns the schedule record
--- (create/describe/patch/pause/resume) or raises on HTTP error.

M.schedules = {}

--- Create a new cron schedule.
--- @param opts table {
---   name, workflow_type, cron_expr, timezone?, input?,
---   task_queue?, overlap_policy?, namespace?
--- }
function M.schedules.create(opts)
    if not opts or not opts.name or not opts.workflow_type or not opts.cron_expr then
        error("workflow.schedules.create: name, workflow_type, cron_expr required")
    end
    local resp = M._api("POST", "/schedules", opts)
    if resp.status ~= 201 then
        error("workflow.schedules.create failed: " .. (resp.body or "unknown error"))
    end
    return json.parse(resp.body)
end

--- List schedules in a namespace.
--- @param opts? table { namespace? }
function M.schedules.list(opts)
    local ns = (opts and opts.namespace) or "main"
    local resp = M._api("GET", "/schedules?namespace=" .. url_encode(ns))
    if resp.status ~= 200 then
        error("workflow.schedules.list failed: " .. (resp.body or "unknown error"))
    end
    return json.parse(resp.body)
end

--- Describe one schedule.
function M.schedules.describe(name, opts)
    local ns = (opts and opts.namespace) or "main"
    local resp = M._api(
        "GET",
        "/schedules/" .. url_encode(name) .. "?namespace=" .. url_encode(ns)
    )
    if resp.status == 404 then return nil end
    if resp.status ~= 200 then
        error("workflow.schedules.describe failed: " .. (resp.body or "unknown error"))
    end
    return json.parse(resp.body)
end

--- Apply an in-place patch to a schedule. Only fields present in the
--- patch are updated; unchanged fields keep their values.
--- @param name string
--- @param patch table { cron_expr?, timezone?, input?, task_queue?, overlap_policy? }
--- @param opts? table { namespace? }
function M.schedules.patch(name, patch, opts)
    local ns = (opts and opts.namespace) or "main"
    local resp = M._api(
        "PATCH",
        "/schedules/" .. url_encode(name) .. "?namespace=" .. url_encode(ns),
        patch or {}
    )
    if resp.status ~= 200 then
        error("workflow.schedules.patch failed: " .. (resp.body or "unknown error"))
    end
    return json.parse(resp.body)
end

--- Pause a schedule (the scheduler skips it until resumed).
function M.schedules.pause(name, opts)
    local ns = (opts and opts.namespace) or "main"
    local resp = M._api(
        "POST",
        "/schedules/" .. url_encode(name) .. "/pause?namespace=" .. url_encode(ns)
    )
    if resp.status ~= 200 then
        error("workflow.schedules.pause failed: " .. (resp.body or "unknown error"))
    end
    return json.parse(resp.body)
end

--- Resume a paused schedule. Does not backfill missed fires — next fire
--- is whatever the cron expression says from now.
function M.schedules.resume(name, opts)
    local ns = (opts and opts.namespace) or "main"
    local resp = M._api(
        "POST",
        "/schedules/" .. url_encode(name) .. "/resume?namespace=" .. url_encode(ns)
    )
    if resp.status ~= 200 then
        error("workflow.schedules.resume failed: " .. (resp.body or "unknown error"))
    end
    return json.parse(resp.body)
end

--- Delete a schedule. Any in-flight workflow it triggered keeps running
--- — deletion only stops future fires.
function M.schedules.delete(name, opts)
    local ns = (opts and opts.namespace) or "main"
    local resp = M._api(
        "DELETE",
        "/schedules/" .. url_encode(name) .. "?namespace=" .. url_encode(ns)
    )
    if resp.status ~= 200 then
        error("workflow.schedules.delete failed: " .. (resp.body or "unknown error"))
    end
end

-- ── Namespaces ──────────────────────────────────────────────

M.namespaces = {}

--- Create a namespace. The endpoint returns no body on success; returns
--- nothing from Lua on success, raises on error.
function M.namespaces.create(name)
    local resp = M._api("POST", "/namespaces", { name = name })
    if resp.status ~= 201 and resp.status ~= 200 then
        error("workflow.namespaces.create failed: " .. (resp.body or "unknown error"))
    end
end

--- List namespaces.
function M.namespaces.list()
    local resp = M._api("GET", "/namespaces")
    if resp.status ~= 200 then
        error("workflow.namespaces.list failed: " .. (resp.body or "unknown error"))
    end
    return json.parse(resp.body)
end

--- Get per-namespace stats (total, running, pending, completed, failed,
--- schedules, workers). Same endpoint as describe — the stats are the
--- response body.
function M.namespaces.stats(name)
    local resp = M._api("GET", "/namespaces/" .. url_encode(name))
    if resp.status ~= 200 then
        error("workflow.namespaces.stats failed: " .. (resp.body or "unknown error"))
    end
    return json.parse(resp.body)
end

--- Alias of `stats` for symmetry with schedules.describe etc.
function M.namespaces.describe(name)
    return M.namespaces.stats(name)
end

--- Delete a namespace.
function M.namespaces.delete(name)
    local resp = M._api("DELETE", "/namespaces/" .. url_encode(name))
    if resp.status ~= 200 then
        error("workflow.namespaces.delete failed: " .. (resp.body or "unknown error"))
    end
end

-- ── API Keys ────────────────────────────────────────────────

M.api_keys = {}

--- Generate (or idempotently retrieve) an API key.
---
--- @param label string|nil  Optional label to tag the key with.
--- @param opts table|nil    `{ idempotent = bool }`. When true and a key
---                          with this `label` already exists, the server
---                          returns the existing record's metadata without
---                          a plaintext (which is only ever retrievable at
---                          generation time).
--- @return table            `{ plaintext?, prefix, label, created_at }`.
---                          `plaintext` is present only on a fresh mint.
---
--- Bootstrap window: this call works without authentication iff the
--- server's `api_keys` table is empty (first-ever key). After that, a
--- valid Bearer token is required.
function M.api_keys.generate(label, opts)
    opts = opts or {}
    local body = { label = label }
    if opts.idempotent ~= nil then
        body.idempotent = opts.idempotent
    end
    local resp = M._api("POST", "/api-keys", body)
    if resp.status ~= 200 and resp.status ~= 201 then
        error("workflow.api_keys.generate failed: " .. (resp.body or "unknown error"))
    end
    return json.parse(resp.body)
end

--- List API key metadata (hashes never exposed). Returns prefix + label
--- + created_at per key.
function M.api_keys.list()
    local resp = M._api("GET", "/api-keys")
    if resp.status ~= 200 then
        error("workflow.api_keys.list failed: " .. (resp.body or "unknown error"))
    end
    return json.parse(resp.body)
end

--- Revoke an API key by its prefix (e.g. "assay_abcd1234...").
function M.api_keys.delete(prefix)
    local resp = M._api("DELETE", "/api-keys/" .. url_encode(prefix))
    if resp.status ~= 204 and resp.status ~= 404 then
        error("workflow.api_keys.delete failed: " .. (resp.body or "unknown error"))
    end
end

-- ── Workers ─────────────────────────────────────────────────

M.workers = {}

--- List registered workers (and their last heartbeat).
--- @param opts? table { namespace? }
function M.workers.list(opts)
    local ns = (opts and opts.namespace) or "main"
    local resp = M._api("GET", "/workers?namespace=" .. url_encode(ns))
    if resp.status ~= 200 then
        error("workflow.workers.list failed: " .. (resp.body or "unknown error"))
    end
    return json.parse(resp.body)
end

-- ── Queues ──────────────────────────────────────────────────

M.queues = {}

--- Pending/running activity counts per task queue.
--- @param opts? table { namespace? }
function M.queues.stats(opts)
    local ns = (opts and opts.namespace) or "main"
    local resp = M._api("GET", "/queues?namespace=" .. url_encode(ns))
    if resp.status ~= 200 then
        error("workflow.queues.stats failed: " .. (resp.body or "unknown error"))
    end
    return json.parse(resp.body)
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
--- the list of commands to submit back to the engine. Shape:
---   * (optional) `RecordSnapshot` command when `ctx:register_query` was
---     called — always emitted first so the latest state is persisted
---     before any scheduling / completion decisions
---   * then one terminal or scheduling command:
---     - a single yielded command (`ScheduleActivity`, etc.) when the
---       handler reached an unfulfilled step
---     - `CompleteWorkflow` when the handler returned normally
---     - `FailWorkflow` when the handler raised an error
---     - `CancelWorkflow` when cancellation was requested
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

    -- Collect registered query results into a snapshot. Runs on every replay
    -- so the latest state is always visible via GET /workflows/{id}/state.
    -- `_collect_snapshot` returns nil when no queries were registered, so
    -- workflows that don't use `ctx:register_query` don't pay the cost.
    local snapshot_cmd = M._collect_snapshot(ctx)
    local function with_snapshot(cmds)
        if snapshot_cmd then
            table.insert(cmds, 1, snapshot_cmd)
        end
        return cmds
    end

    if not ok then
        local err = tostring(yielded_or_returned)
        -- Cancellation is a clean exit, not a failure — translate the
        -- sentinel raised by ctx:check_cancel back into a CancelWorkflow
        -- command so the engine flips status to CANCELLED (not FAILED).
        if err:find("__ASSAY_WORKFLOW_CANCELLED__", 1, true) then
            return with_snapshot({{ type = "CancelWorkflow" }})
        end
        return with_snapshot({{ type = "FailWorkflow", error = err }})
    end
    if coroutine.status(co) == "dead" then
        return with_snapshot({{ type = "CompleteWorkflow", result = yielded_or_returned }})
    end
    -- Yielded a command (or a batch) — submit and let a subsequent replay continue.
    -- Parallel activities yield `{ _batch = true, commands = {...} }` so we
    -- can submit N commands from a single handler run without bouncing the
    -- workflow N times through the dispatch loop.
    if type(yielded_or_returned) == "table" and yielded_or_returned._batch then
        return with_snapshot(yielded_or_returned.commands)
    end
    return with_snapshot({ yielded_or_returned })
end

--- Build a RecordSnapshot command from any query handlers registered via
--- `ctx:register_query`. Returns nil if no handlers were registered, so
--- workflows that don't expose state don't emit snapshot commands.
---
--- Each handler runs once per replay. A handler that errors is dropped
--- from the snapshot rather than crashing the workflow — queries are a
--- best-effort read-through, not load-bearing.
function M._collect_snapshot(ctx)
    if not ctx._queries or not next(ctx._queries) then
        return nil
    end
    local state = {}
    for name, fn in pairs(ctx._queries) do
        local ok, value = pcall(fn)
        if ok then
            state[name] = value
        end
    end
    if next(state) == nil then
        return nil
    end
    return { type = "RecordSnapshot", state = state }
end

--- Build the workflow ctx object used during replay. Each `ctx:` call
--- increments an internal seq counter and either returns the cached
--- value from history (replay) or yields a command (first time through).
function M._make_workflow_ctx(workflow_id, history)
    -- Pre-index history by per-command seq for O(1) lookups during replay.
    -- Each command type has its own seq space — activity, timer, signal
    -- counters are independent. Signals are matched by name (workflows
    -- typically wait on a specific signal name), and the signal queue
    -- preserves arrival order so multiple of the same name are consumed
    -- in turn.
    local activity_results, fired_timers, side_effects, child_results = {}, {}, {}, {}
    local signals_by_name = {} -- [name] = list of payloads in arrival order
    local cancel_requested = false
    for _, event in ipairs(history) do
        local p = event.payload
        if event.event_type == "ActivityCompleted" and p and p.activity_seq then
            activity_results[p.activity_seq] = { ok = true, value = p.result }
        elseif event.event_type == "ActivityFailed" and p and p.activity_seq then
            activity_results[p.activity_seq] = { ok = false, err = p.error }
        elseif event.event_type == "TimerFired" and p and p.timer_seq then
            fired_timers[p.timer_seq] = true
        elseif event.event_type == "SignalReceived" and p and p.signal then
            signals_by_name[p.signal] = signals_by_name[p.signal] or {}
            table.insert(signals_by_name[p.signal], p.payload)
        elseif event.event_type == "SideEffectRecorded" and p and p.side_effect_seq then
            side_effects[p.side_effect_seq] = p.value
        elseif event.event_type == "ChildWorkflowCompleted" and p and p.child_workflow_id then
            child_results[p.child_workflow_id] = { ok = true, value = p.result }
        elseif event.event_type == "ChildWorkflowFailed" and p and p.child_workflow_id then
            child_results[p.child_workflow_id] = { ok = false, err = p.error }
        elseif event.event_type == "WorkflowCancelRequested" then
            cancel_requested = true
        end
    end

    -- Per-workflow-execution signal cursors track how many signals of each
    -- name have already been consumed by ctx:wait_for_signal calls in
    -- this replay, so a workflow that waits twice for "approve" gets the
    -- first arrival on the first call, the second on the second call.
    local signal_cursor = {}
    local activity_seq, timer_seq, side_effect_seq = 0, 0, 0
    local ctx = { workflow_id = workflow_id }

    -- Helper: any ctx method bails out via this if the workflow has been
    -- requested to cancel. The runner catches the sentinel and emits a
    -- CancelWorkflow command, which finalises the workflow state.
    local function check_cancel()
        if cancel_requested then
            error("__ASSAY_WORKFLOW_CANCELLED__")
        end
    end

    --- Schedule an activity and (synchronously, for the workflow author)
    --- return its result. On replay, returns the cached result from
    --- history; on first execution at this seq, yields a ScheduleActivity
    --- command and the workflow run ends until the activity completes
    --- and the workflow becomes dispatchable again.
    function ctx:execute_activity(name, input, opts)
        check_cancel()
        activity_seq = activity_seq + 1
        local r = activity_results[activity_seq]
        if r then
            if r.ok then return r.value end
            error("activity '" .. name .. "' failed: " .. tostring(r.err))
        end
        coroutine.yield({
            type = "ScheduleActivity",
            seq = activity_seq,
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

    --- Schedule multiple activities concurrently and return their results in
    --- the same order. On replay: if all N activities have completed events
    --- in history, returns immediately with a list of results. If any are
    --- missing, yields a batch of ScheduleActivity commands for the missing
    --- ones — the engine schedules them idempotently (on `seq`) and the
    --- workflow is re-dispatched on each completion. The workflow proceeds
    --- past this call only when every activity has a terminal event.
    ---
    --- Each completion triggers a replay that yields another batch of
    --- missing commands; because `schedule_activity` is idempotent on
    --- `(workflow_id, seq)`, repeated scheduling of the same seq is a
    --- no-op at the store layer.
    ---
    --- If any activity fails after exhausting its retries, this call raises
    --- with that activity's error. Per-activity retry/timeout opts are
    --- passed through, same as `ctx:execute_activity`.
    ---
    --- Usage:
    ---   local results = ctx:execute_parallel({
    ---       { name = "check_a", input = { id = 1 } },
    ---       { name = "check_b", input = { id = 2 } },
    ---       { name = "check_c", input = { id = 3 } },
    ---   })
    ---   -- results[1], results[2], results[3] are in input order
    function ctx:execute_parallel(activities)
        check_cancel()
        if type(activities) ~= "table" or #activities == 0 then
            error("ctx:execute_parallel: activities must be a non-empty list")
        end
        local seqs, results, all_done, first_error = {}, {}, true, nil
        local pending_cmds = {}
        for i, a in ipairs(activities) do
            activity_seq = activity_seq + 1
            seqs[i] = activity_seq
            local r = activity_results[activity_seq]
            if r then
                if r.ok then
                    results[i] = r.value
                else
                    first_error = first_error
                        or ("activity '" .. (a.name or "?")
                            .. "' failed: " .. tostring(r.err))
                end
            else
                all_done = false
                local opts = a.opts or {}
                pending_cmds[#pending_cmds + 1] = {
                    type = "ScheduleActivity",
                    seq = activity_seq,
                    name = a.name,
                    task_queue = opts.task_queue or "default",
                    input = a.input,
                    max_attempts = opts.max_attempts,
                    initial_interval_secs = opts.initial_interval_secs,
                    backoff_coefficient = opts.backoff_coefficient,
                    start_to_close_secs = opts.start_to_close_secs,
                    heartbeat_timeout_secs = opts.heartbeat_timeout_secs,
                }
            end
        end
        if all_done then
            if first_error then error(first_error) end
            return results
        end
        -- Yield a BATCH: a table with `_batch = true` so the worker's
        -- replay loop recognises it and submits every command in order
        -- instead of wrapping a single command.
        coroutine.yield({ _batch = true, commands = pending_cmds })
        error("workflow ctx: yielded but resumed unexpectedly")
    end

    --- Pause the workflow durably for `seconds`. The timer is persisted in
    --- the engine; if the worker dies the timer still fires when due and
    --- another worker picks up the workflow. On replay, returns immediately
    --- once the matching TimerFired event is in history.
    function ctx:sleep(seconds)
        check_cancel()
        timer_seq = timer_seq + 1
        if fired_timers[timer_seq] then return end
        coroutine.yield({
            type = "ScheduleTimer",
            seq = timer_seq,
            duration_secs = seconds,
        })
        error("workflow ctx: yielded but resumed unexpectedly")
    end

    --- Run a non-deterministic operation exactly once. The result is
    --- recorded in the workflow event log and returned from cache on all
    --- subsequent replays — so calls like `crypto.uuid()`, `os.time()`,
    --- or anything reading external mutable state can safely live inside
    --- a workflow handler.
    ---
    --- Conceptually a checkpoint: the function runs in the worker, the
    --- worker yields the value to the engine to record, and the engine
    --- re-dispatches the workflow so it continues with the cached value.
    function ctx:side_effect(name, fn)
        check_cancel()
        side_effect_seq = side_effect_seq + 1
        local cached = side_effects[side_effect_seq]
        if cached ~= nil then
            return cached
        end
        local value = fn()
        coroutine.yield({
            type = "RecordSideEffect",
            seq = side_effect_seq,
            name = name,
            value = value,
        })
        error("workflow ctx: yielded but resumed unexpectedly")
    end

    --- Start a child workflow and (synchronously, for the workflow author)
    --- wait for it to complete. Returns the child's result, or raises if
    --- the child failed. The parent yields and is paused until the child
    --- reaches a terminal state — at which point the engine appends a
    --- ChildWorkflowCompleted/Failed event to the parent and re-dispatches
    --- so the parent's handler can replay past this call.
    ---
    --- `opts.workflow_id` MUST be deterministic — repeated calls during
    --- replay must produce the same id, otherwise idempotency breaks.
    function ctx:start_child_workflow(workflow_type, opts)
        check_cancel()
        if not opts or not opts.workflow_id then
            error("ctx:start_child_workflow: opts.workflow_id is required")
        end
        local cached = child_results[opts.workflow_id]
        if cached then
            if cached.ok then return cached.value end
            error("child workflow '" .. opts.workflow_id ..
                "' failed: " .. tostring(cached.err))
        end
        coroutine.yield({
            type = "StartChildWorkflow",
            workflow_type = workflow_type,
            workflow_id = opts.workflow_id,
            input = opts.input,
            task_queue = opts.task_queue or "default",
        })
        error("workflow ctx: yielded but resumed unexpectedly")
    end

    --- Merge a JSON object into the workflow's stored `search_attributes`
    --- so external callers can filter the list endpoint on application-
    --- level metadata. Keys in the patch overwrite existing keys; keys
    --- not in the patch are preserved.
    ---
    --- Typical use: tag the workflow with progress / tenant / env so
    --- dashboards can filter by them:
    ---
    ---   ctx:upsert_search_attributes({ progress = 0.5, stage = "deploy" })
    function ctx:upsert_search_attributes(patch)
        check_cancel()
        if type(patch) ~= "table" then
            error("ctx:upsert_search_attributes: patch must be a table")
        end
        coroutine.yield({
            type = "UpsertSearchAttributes",
            patch = patch,
        })
        error("workflow ctx: yielded but resumed unexpectedly")
    end

    --- End this run and start a fresh one with the same workflow type,
    --- namespace, and task queue but an empty event history. Use for
    --- unbounded-loop workflows (pollers, schedulers) whose event log
    --- would otherwise grow forever.
    ---
    --- The new run's id is derived from the current one (with a timestamp
    --- suffix) and its `input` is whatever you pass here. The current run
    --- is marked COMPLETED.
    ---
    --- Typically the last thing a handler calls:
    ---
    ---   workflow.define("Poller", function(ctx, input)
    ---       local items = ctx:execute_activity("poll", input)
    ---       ctx:sleep(60)
    ---       return ctx:continue_as_new({ cursor = items.next_cursor })
    ---   end)
    function ctx:continue_as_new(input)
        check_cancel()
        coroutine.yield({
            type = "ContinueAsNew",
            input = input,
        })
        error("workflow ctx: yielded but resumed unexpectedly")
    end

    --- Register a named query handler that exposes live workflow state to
    --- external callers via `GET /api/v1/workflows/{id}/state` (all handlers)
    --- or `GET /api/v1/workflows/{id}/state/{name}` (one handler).
    ---
    --- The handler is invoked on every worker replay, after the workflow
    --- coroutine yields or returns. Its result is serialised as JSON and
    --- persisted as a snapshot keyed by the current event seq. Because
    --- workflow handlers replay deterministically, the closure captures
    --- the latest values of any local variables it references.
    ---
    --- Usage:
    ---   workflow.define("Pipeline", function(ctx, input)
    ---       local state = { stage = "init" }
    ---       ctx:register_query("pipeline_state", function() return state end)
    ---       state.stage = "running"
    ---       ctx:execute_activity("step1", {})
    ---       state.stage = "done"
    ---   end)
    ---
    --- A handler that raises is dropped from the snapshot rather than
    --- crashing the workflow — queries are a best-effort read-through.
    function ctx:register_query(name, fn)
        if type(name) ~= "string" or name == "" then
            error("ctx:register_query: name must be a non-empty string")
        end
        if type(fn) ~= "function" then
            error("ctx:register_query: handler must be a function")
        end
        self._queries = self._queries or {}
        self._queries[name] = fn
    end

    --- Block until a signal with the given name arrives. Returns the
    --- signal's JSON payload (or nil if signaled with no payload).
    --- The "wait" is purely declarative — the workflow yields, the worker
    --- releases its lease, and a future call to send_signal wakes the
    --- workflow back up via mark_workflow_dispatchable. Multiple waits for
    --- the same signal name consume signals in arrival order.
    function ctx:wait_for_signal(name)
        check_cancel()
        local consumed = signal_cursor[name] or 0
        local arrivals = signals_by_name[name] or {}
        if consumed < #arrivals then
            consumed = consumed + 1
            signal_cursor[name] = consumed
            return arrivals[consumed]
        end
        coroutine.yield({
            type = "WaitForSignal",
            name = name,
        })
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
    elseif method == "PATCH" then
        return http.patch(url, body or {}, opts)
    elseif method == "DELETE" then
        return http.delete(url, opts)
    else
        error("workflow._api: unsupported method: " .. method)
    end
end

return M
