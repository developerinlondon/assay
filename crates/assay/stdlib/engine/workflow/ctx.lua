--- Workflow context (`ctx`) factory — the object passed to every
--- workflow handler. Owns the deterministic-replay machinery: each
--- `ctx:*` call increments an internal seq counter and either returns
--- a value cached from history (replay) or yields a command for the
--- engine to schedule (first time through).
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
--- @param workflow_id string
--- @param history table  Workflow event history (already fetched by the worker).
--- @return table ctx
function M.make(workflow_id, history)
  -- Pre-index history for O(1) lookups during replay. Each command
  -- type has its own seq space — activity / timer / signal counters
  -- are independent. Signals are matched by name; the queue
  -- preserves arrival order so multiple of the same name are
  -- consumed in turn.
  local activity_results, fired_timers, side_effects, child_results = {}, {}, {}, {}
  local signals_by_name = {}
  local signal_seqs_by_name = {}
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

  local signal_cursor = {}
  local activity_seq, timer_seq, side_effect_seq = 0, 0, 0
  local ctx = { workflow_id = workflow_id }

  local function check_cancel()
    if cancel_requested then error("__ASSAY_WORKFLOW_CANCELLED__") end
  end

  --- Schedule an activity and (synchronously, for the workflow author)
  --- return its result.
  function ctx:execute_activity(name, input, opts)
    activity_seq = activity_seq + 1
    local r = activity_results[activity_seq]
    if r then
      if r.ok then return r.value end
      error("activity '" .. name .. "' failed: " .. tostring(r.err))
    end
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
    error("workflow ctx: yielded but resumed unexpectedly")
  end

  --- Schedule N activities concurrently and return their results in
  --- input order. On replay all-N-cached returns immediately; missing
  --- ones yield a batch. Each completion re-dispatches the workflow.
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
            or ("activity '" .. (a.name or "?") .. "' failed: " .. tostring(r.err))
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
    check_cancel()
    coroutine.yield({ _batch = true, commands = pending_cmds })
    error("workflow ctx: yielded but resumed unexpectedly")
  end

  --- Pause the workflow durably for `seconds`.
  function ctx:sleep(seconds)
    timer_seq = timer_seq + 1
    if fired_timers[timer_seq] then return end
    check_cancel()
    coroutine.yield({ type = "ScheduleTimer", seq = timer_seq, duration_secs = seconds })
    error("workflow ctx: yielded but resumed unexpectedly")
  end

  --- Run a non-deterministic operation exactly once, recording the
  --- result so all subsequent replays return it from cache.
  function ctx:side_effect(name, fn)
    side_effect_seq = side_effect_seq + 1
    local cached = side_effects[side_effect_seq]
    if cached ~= nil then return cached end
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

  --- Start a child workflow and synchronously wait for completion.
  function ctx:start_child_workflow(workflow_type, opts)
    if not opts or not opts.workflow_id then
      error("ctx:start_child_workflow: opts.workflow_id is required")
    end
    local cached = child_results[opts.workflow_id]
    if cached then
      if cached.ok then return cached.value end
      error("child workflow '" .. opts.workflow_id .. "' failed: " .. tostring(cached.err))
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

  --- Merge a JSON object into the workflow's stored search_attributes.
  function ctx:upsert_search_attributes(patch)
    check_cancel()
    if type(patch) ~= "table" then
      error("ctx:upsert_search_attributes: patch must be a table")
    end
    coroutine.yield({ type = "UpsertSearchAttributes", patch = patch })
    error("workflow ctx: yielded but resumed unexpectedly")
  end

  --- End this run and start a fresh one. Use for unbounded-loop
  --- workflows whose event log would otherwise grow forever.
  function ctx:continue_as_new(input)
    check_cancel()
    coroutine.yield({ type = "ContinueAsNew", input = input })
    error("workflow ctx: yielded but resumed unexpectedly")
  end

  --- Register a named query handler that exposes live workflow state
  --- via GET /api/v1/engine/workflow/workflows/{id}/state.
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

  --- Self-cancel: workflow decides itself it should stop early. Lands
  --- in the same terminal state as an externally-requested cancel.
  function ctx:cancel(reason)
    if reason and reason ~= "" then
      log.info("workflow " .. tostring(self.workflow_id) ..
        " cancelling itself: " .. tostring(reason))
    end
    error("__ASSAY_WORKFLOW_CANCELLED__")
  end

  --- Block until a signal arrives (optionally bounded by timeout).
  --- Returns the signal payload, or nil on timeout.
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
      check_cancel()
      coroutine.yield({ type = "WaitForSignal", name = name })
      error("workflow ctx: yielded but resumed unexpectedly")
    end

    -- Timed path: race the next unconsumed signal against a workflow-
    -- local timer. Replay decides winner by event seq.
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
    if timer_fired_at then return nil end

    check_cancel()
    coroutine.yield({
      _batch = true,
      commands = {
        { type = "ScheduleTimer", seq = my_timer_seq, duration_secs = timeout },
        { type = "WaitForSignal", name = name, timer_seq = my_timer_seq },
      },
    })
    error("workflow ctx: yielded but resumed unexpectedly")
  end

  return ctx
end

return M
