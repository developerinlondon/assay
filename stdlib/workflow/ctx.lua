--- Workflow context (`ctx`) factory — the object passed to every
--- workflow handler. Owns the deterministic-replay machinery: each
--- `ctx:*` call increments an internal seq counter and either returns
--- a value cached from history (replay) or yields a command for the
--- engine to schedule (first time through).
---
--- This is the bulk of the workflow stdlib by line count, so it lives
--- in its own file. `assay.workflow` (the parent module) imports it
--- and exposes a thin `M._make_workflow_ctx` wrapper that just calls
--- `ctx_mod.make(M, workflow_id, history)`.
---
--- Cancellation discipline:
--- `check_cancel` is called AFTER cache lookups in each ctx method,
--- never before. On a cancel-replay the handler must re-consume all
--- prior activity results / signals / timers so the local state
--- mutations up to that point take effect — only then should
--- cancellation raise. If check_cancel fired at the very start of
--- every ctx call, the first wait_for_signal on replay would raise
--- before any state had been rebuilt, leading to a stale snapshot
--- (all steps back to initial).

local M = {}

--- Build the workflow ctx object used during replay.
--- @param parent table  The parent workflow module (unused today; passed for symmetry + future hooks).
--- @param workflow_id string
--- @param history table  Workflow event history (already fetched by the worker).
--- @return table ctx
function M.make(parent, workflow_id, history)
    -- Pre-index history by per-command seq for O(1) lookups during replay.
    -- Each command type has its own seq space — activity, timer, signal
    -- counters are independent. Signals are matched by name (workflows
    -- typically wait on a specific signal name), and the signal queue
    -- preserves arrival order so multiple of the same name are consumed
    -- in turn.
    local activity_results, fired_timers, side_effects, child_results = {}, {}, {}, {}
    local signals_by_name = {} -- [name] = list of payloads in arrival order
    -- Parallel to signals_by_name, holds the event seq of each arrival so
    -- ctx:wait_for_signal with a timeout can race a specific signal against
    -- a specific timer by comparing their history event seqs.
    local signal_seqs_by_name = {}
    -- Event seq at which each timer fired, keyed by the timer's workflow-
    -- local seq. Used by the timed wait_for_signal path to decide winner.
    local timer_fired_seqs = {}
    local cancel_requested = false
    for _, event in ipairs(history) do
        local p = event.payload
        if event.event_type == "ActivityCompleted" and p and p.activity_seq then
            activity_results[p.activity_seq] = { ok = true, value = p.result }
        elseif event.event_type == "ActivityFailed" and p and p.activity_seq then
            activity_results[p.activity_seq] = { ok = false, err = p.error }
        elseif event.event_type == "TimerFired" and p and p.timer_seq then
            fired_timers[p.timer_seq] = true
            timer_fired_seqs[p.timer_seq] = event.seq
        elseif event.event_type == "SignalReceived" and p and p.signal then
            signals_by_name[p.signal] = signals_by_name[p.signal] or {}
            signal_seqs_by_name[p.signal] = signal_seqs_by_name[p.signal] or {}
            table.insert(signals_by_name[p.signal], p.payload)
            table.insert(signal_seqs_by_name[p.signal], event.seq)
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
    -- requested to cancel AND the handler has run out of cached history
    -- to replay (i.e. is at the "frontier" trying to do new work). The
    -- runner catches the sentinel and emits a CancelWorkflow command.
    --
    -- See module docstring for why this is post-cache, not pre-cache.
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
        activity_seq = activity_seq + 1
        local r = activity_results[activity_seq]
        if r then
            if r.ok then return r.value end
            error("activity '" .. name .. "' failed: " .. tostring(r.err))
        end
        -- Cache miss — replay has caught up with history and we're
        -- about to yield a new ScheduleActivity. Check for cancel
        -- NOW: state mutations up to this point have all applied so
        -- a snapshot taken post-raise reflects real progress.
        check_cancel()
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
        -- Cache miss on at least one activity — frontier, check
        -- cancellation now.
        check_cancel()
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
        timer_seq = timer_seq + 1
        if fired_timers[timer_seq] then return end
        check_cancel()
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
        side_effect_seq = side_effect_seq + 1
        local cached = side_effects[side_effect_seq]
        if cached ~= nil then
            return cached
        end
        check_cancel()
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
        if not opts or not opts.workflow_id then
            error("ctx:start_child_workflow: opts.workflow_id is required")
        end
        local cached = child_results[opts.workflow_id]
        if cached then
            if cached.ok then return cached.value end
            error("child workflow '" .. opts.workflow_id ..
                "' failed: " .. tostring(cached.err))
        end
        check_cancel()
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

    --- Terminate the current workflow with engine status `CANCELLED`.
    ---
    --- Use this when the workflow has decided itself that it should
    --- stop early — typically after a human-approval signal comes back
    --- as a reject, or a precondition fails — and you want downstream
    --- observers (dashboards, audit queries, `workflow.list` filters)
    --- to see `CANCELLED` rather than `COMPLETED`.
    ---
    --- Implemented by raising the internal cancellation sentinel which
    --- the task runner already translates into a `CancelWorkflow`
    --- command. Distinct from an externally-requested cancel
    --- (`workflow.cancel(id)` → `WorkflowCancelRequested` event), but
    --- lands in the same terminal state.
    ---
    --- Usage:
    ---   if decision.action == "reject" then
    ---       state.rejected_by = decision.user
    ---       ctx:cancel("rejected by " .. decision.user)
    ---   end
    function ctx:cancel(reason)
        -- reason is accepted for symmetry with Temporal's cancel/fail
        -- APIs and for callsite-readability; it's logged but not wired
        -- into the sentinel because the engine's CancelWorkflow command
        -- already carries enough context (status change, timestamps).
        if reason and reason ~= "" then
            log.info("workflow " .. tostring(self.workflow_id) ..
                " cancelling itself: " .. tostring(reason))
        end
        error("__ASSAY_WORKFLOW_CANCELLED__")
    end

    --- Block until a signal with the given name arrives, optionally
    --- bounded by a timeout. Returns the signal's JSON payload (or nil
    --- if signaled with no payload). With `opts.timeout`, returns nil
    --- when the timeout elapses before any matching signal arrives.
    ---
    --- The "wait" is purely declarative — the workflow yields, the worker
    --- releases its lease, and a future call to send_signal wakes the
    --- workflow back up via mark_workflow_dispatchable. Multiple waits for
    --- the same signal name consume signals in arrival order.
    ---
    --- With a timeout, the ctx yields a batch of two commands:
    ---   * `ScheduleTimer{seq = T, duration_secs = opts.timeout}`
    ---   * `WaitForSignal{name, timer_seq = T}`
    --- On replay the winner is chosen by history event seq — whichever
    --- of the next unconsumed `SignalReceived{signal = name}` or the
    --- paired `TimerFired{timer_seq = T}` has the lower seq wins. If
    --- neither has happened yet, the batch is re-yielded (idempotent on
    --- `timer_seq`). Determinism matches `ctx:sleep` and `ctx:execute_parallel`.
    ---
    --- Usage:
    ---   local payload = ctx:wait_for_signal("approve")
    ---   local payload = ctx:wait_for_signal("approve", { timeout = 86400 })
    ---   if payload == nil then
    ---       -- treat as rejected / cancelled
    ---   end
    function ctx:wait_for_signal(name, opts)
        if type(name) ~= "string" or name == "" then
            error("ctx:wait_for_signal: name must be a non-empty string")
        end
        if opts ~= nil and type(opts) ~= "table" then
            error("ctx:wait_for_signal: opts must be a table if provided")
        end
        local timeout = opts and opts.timeout
        if timeout ~= nil and (type(timeout) ~= "number" or timeout <= 0) then
            error("ctx:wait_for_signal: opts.timeout must be a positive number")
        end

        if not timeout then
            local consumed = signal_cursor[name] or 0
            local arrivals = signals_by_name[name] or {}
            if consumed < #arrivals then
                consumed = consumed + 1
                signal_cursor[name] = consumed
                return arrivals[consumed]
            end
            -- No cached signal — frontier. Check cancel here so prior
            -- signals are consumed during replay even if cancellation
            -- has been requested.
            check_cancel()
            coroutine.yield({
                type = "WaitForSignal",
                name = name,
            })
            error("workflow ctx: yielded but resumed unexpectedly")
        end

        -- Timed path: race the next unconsumed signal of this name against
        -- a workflow-local timer. Each call increments timer_seq so repeat
        -- calls schedule distinct timers; `ScheduleTimer` is idempotent on
        -- (workflow_id, seq) at the engine so replays don't duplicate.
        timer_seq = timer_seq + 1
        local my_timer_seq = timer_seq
        local consumed = signal_cursor[name] or 0
        local arrivals = signals_by_name[name] or {}
        local seqs = signal_seqs_by_name[name] or {}
        local next_signal_seq = seqs[consumed + 1]
        local timer_fired_at = timer_fired_seqs[my_timer_seq]

        if next_signal_seq and (not timer_fired_at or next_signal_seq < timer_fired_at) then
            signal_cursor[name] = consumed + 1
            return arrivals[consumed + 1]
        end
        if timer_fired_at then
            return nil
        end

        -- No cached signal or timer-fire — frontier, check cancel.
        check_cancel()
        coroutine.yield({
            _batch = true,
            commands = {
                {
                    type = "ScheduleTimer",
                    seq = my_timer_seq,
                    duration_secs = timeout,
                },
                {
                    type = "WaitForSignal",
                    name = name,
                    timer_seq = my_timer_seq,
                },
            },
        })
        error("workflow ctx: yielded but resumed unexpectedly")
    end

    return ctx
end

return M
