-- Demo worker for the v0.12.0 Pipeline-tab e2e suite.
-- Emits the canonical pipeline_state { status, current_step, steps[], log[] }
-- shape so the dashboard can render circles + connectors + log tail and
-- the suite can verify each state transition end-to-end.

local workflow = require("assay.workflow")

workflow.connect("http://localhost:8080", { namespace = "demo" })

local STEP_NAMES = { "Approval", "Tag & Retag", "GitOps Update", "ArgoCD Sync", "Health Check" }

local function init_steps()
  local s = {}
  for i, name in ipairs(STEP_NAMES) do
    s[i] = {
      name = name,
      status = i == 1 and "running" or "waiting",
      started_at = i == 1 and os.date("!%Y-%m-%dT%H:%M:%SZ") or nil,
    }
  end
  return s
end

local function add_log(state, msg, idx)
  state.log[#state.log + 1] = {
    time = os.date("!%H:%M:%S"),
    msg = msg,
    step = idx or state.current_step,
  }
end

-- 10-minute approval timeout. After that the workflow auto-cancels
-- (the workflow author's choice — could just as well auto-approve or
-- escalate). Production CC uses 24h on the same pattern; this is
-- shorter for live preview / e2e so operators don't have to wait.
local APPROVAL_TIMEOUT_SECS = 600

workflow.define("DemoPipeline", function(ctx, input)
  local now_unix = os.time()
  local state = {
    status = "pending_approval",
    current_step = 1,
    steps = init_steps(),
    log = {},
  }
  state.steps[1].actions = { "approve", "reject" }
  -- Surface the deadline so the dashboard can render a live countdown
  -- on the step's circle. Convention: `expires_at` is a unix epoch
  -- (seconds since 1970, UTC). Steps without it just don't show a
  -- countdown — the dashboard renders the rest of the step the same.
  state.steps[1].expires_at = now_unix + APPROVAL_TIMEOUT_SECS

  ctx:register_query("pipeline_state", function() return state end)

  add_log(state, "Pipeline started — waiting for approval (timeout in "
    .. (APPROVAL_TIMEOUT_SECS / 60) .. " min)", 1)

  -- Step 1: wait for the step_action signal carrying {step, action, user},
  -- bounded by APPROVAL_TIMEOUT_SECS. If no decision arrives in that
  -- time, ctx:wait_for_signal returns nil and the workflow takes the
  -- default-action path below (auto-cancel).
  local sig = ctx:wait_for_signal("step_action", { timeout = APPROVAL_TIMEOUT_SECS })
  -- Clear the step's action buttons + countdown immediately — the
  -- dashboard's action-bar reconciliation reads `actions` on every
  -- snapshot and leaves the buttons up as long as the array is
  -- present; same with `expires_at` for the countdown badge.
  state.steps[1].actions = nil
  state.steps[1].expires_at = nil
  if not sig then
    -- Timeout — no decision in APPROVAL_TIMEOUT_SECS. Production
    -- author choice: auto-cancel. Could just as well auto-approve,
    -- escalate, or transition to a different default.
    state.steps[1].status = "cancelled"
    state.steps[1].completed_at = os.date("!%Y-%m-%dT%H:%M:%SZ")
    state.status = "cancelled"
    for i = 2, #state.steps do state.steps[i].status = "skipped" end
    add_log(state, "Approval timed out after "
      .. (APPROVAL_TIMEOUT_SECS / 60) .. " min — auto-cancelled", 1)
    ctx:cancel("approval timed out after " .. APPROVAL_TIMEOUT_SECS .. "s")
    return
  end
  if sig.action ~= "approve" then
    state.steps[1].status = "cancelled"
    state.steps[1].completed_at = os.date("!%Y-%m-%dT%H:%M:%SZ")
    state.status = "cancelled"
    for i = 2, #state.steps do state.steps[i].status = "skipped" end
    add_log(state, "Pipeline rejected by " .. (sig.user or "unknown"), 1)
    ctx:cancel("rejected by " .. (sig.user or "unknown"))
    return
  end

  state.steps[1].status = "done"
  state.steps[1].completed_at = os.date("!%Y-%m-%dT%H:%M:%SZ")
  add_log(state, "Approved by " .. (sig.user or "unknown"), 1)

  -- Steps 2..5: each runs for ~6 seconds with progress log lines so
  -- the live tail has something to render. Wrapped in pcall so a
  -- cancel raised mid-pipeline (engine-side cancel API) updates the
  -- step states for a clean visual: the in-flight step → cancelled,
  -- everything after → skipped. The cancel sentinel is then
  -- re-raised so the engine still sees CancelWorkflow and finalises
  -- the run as CANCELLED.
  local ok, err = pcall(function()
    for i = 2, #state.steps do
      state.current_step = i
      state.steps[i].status = "running"
      state.steps[i].started_at = os.date("!%Y-%m-%dT%H:%M:%SZ")
      add_log(state, "Starting " .. state.steps[i].name, i)
      for tick = 1, 3 do
        ctx:sleep(2)
        add_log(state, state.steps[i].name .. " progress " .. (tick * 33) .. "%", i)
      end
      state.steps[i].status = "done"
      state.steps[i].completed_at = os.date("!%Y-%m-%dT%H:%M:%SZ")
      add_log(state, state.steps[i].name .. " complete", i)
    end
  end)

  if not ok then
    if tostring(err):find("__ASSAY_WORKFLOW_CANCELLED__", 1, true) then
      -- Cancel raised mid-pipeline. Mark whichever step was running
      -- as cancelled and everything after as skipped, then re-raise.
      local at = state.current_step or 1
      if state.steps[at] and state.steps[at].status == "running" then
        state.steps[at].status = "cancelled"
        state.steps[at].completed_at = os.date("!%Y-%m-%dT%H:%M:%SZ")
        add_log(state, state.steps[at].name .. " cancelled by operator", at)
      end
      for j = (at or 1) + 1, #state.steps do
        if state.steps[j].status == "waiting" then
          state.steps[j].status = "skipped"
        end
      end
      state.status = "cancelled"
      error(err, 0)
    end
    -- Other errors propagate as failure.
    error(err, 0)
  end

  state.status = "done"
  add_log(state, "Pipeline complete", #state.steps)
end)

workflow.listen({ queue = "demo-q", namespace = "demo" })
