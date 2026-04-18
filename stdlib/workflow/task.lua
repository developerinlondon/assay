--- Task handling — turns a polled task into the commands the engine
--- needs to advance the workflow / activity. Two task types:
---
---   * `handle_workflow_task` runs the workflow handler against the
---     current event history and returns the next batch of commands
---     (snapshot, schedule, complete, fail, cancel).
---   * `execute_activity` runs a single activity to completion and
---     returns its result for the engine to persist.
---
--- The parent module (`stdlib/workflow.lua`) owns the registry of
--- workflow + activity handlers (`M._workflows`, `M._activities`) and
--- the ctx factory (`M._make_workflow_ctx`); we accept it as the
--- first arg so this submodule stays state-free.

local M = {}

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
function M.handle_workflow_task(parent, task)
    local handler = parent._workflows[task.workflow_type]
    if not handler then
        return {{
            type = "FailWorkflow",
            error = "no workflow handler registered for type: " .. tostring(task.workflow_type),
        }}
    end

    local ctx = parent._make_workflow_ctx(task.workflow_id, task.history or {})
    local co = coroutine.create(function() return handler(ctx, task.input) end)

    local ok, yielded_or_returned = coroutine.resume(co)

    -- Collect registered query results into a snapshot. Runs on every replay
    -- so the latest state is always visible via GET /workflows/{id}/state.
    -- `collect_snapshot` returns nil when no queries were registered, so
    -- workflows that don't use `ctx:register_query` don't pay the cost.
    local snapshot_cmd = M.collect_snapshot(ctx)
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
function M.collect_snapshot(ctx)
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

--- Execute an activity task (the concrete work; runs once, result persisted).
function M.execute_activity(parent, task)
    local handler = parent._activities[task.name]
    if not handler then
        error("No handler registered for activity: " .. (task.name or "?"))
    end
    local input = task.input
    if type(input) == "string" then input = json.parse(input) end

    local ctx = M.make_activity_ctx(parent, task)
    return handler(ctx, input)
end

--- Activity ctx — minimal, just exposes a heartbeat so long-running
--- activities can prove they're still alive.
function M.make_activity_ctx(parent, task)
    local ctx = {}
    function ctx:heartbeat(details)
        parent._api("POST", "/tasks/" .. task.id .. "/heartbeat", { details = details })
    end
    return ctx
end

return M
