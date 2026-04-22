--- Worker loop — registers the calling process as a worker on a
--- (namespace, queue) and polls for tasks until cancelled. Polls
--- workflow tasks first (cheap orchestration) then activity tasks.
---
--- Resilience: every engine call is wrapped in pcall so a transient
--- network blip (DNS timeout, engine pod restart, kube-proxy hiccup)
--- doesn't kill the worker. On failure we back off exponentially up
--- to LISTEN_BACKOFF_MAX_SECS and reset to the baseline sleep on the
--- first successful call — recovery is instant once connectivity
--- returns instead of waiting out a stale long sleep.

local M = {}

local LISTEN_BACKOFF_MIN_SECS = 1
local LISTEN_BACKOFF_MAX_SECS = 30

--- Start listening for tasks. Blocks until cancelled.
--- A worker is scoped to a single (namespace, queue) pair — only
--- workflows started in the same namespace on the same queue will be
--- dispatched to it.
--- @param parent table Parent workflow module (carries _engine_url, _workflows, etc.)
--- @param opts table { identity?, queue?, namespace?, max_concurrent_workflows?, max_concurrent_activities? }
function M.listen(parent, opts)
    if not parent._engine_url then
        error("workflow.listen: call workflow.connect() first")
    end

    local queue = opts.queue or "default"
    local namespace = opts.namespace or "main"
    local identity = opts.identity or
        ("assay-worker-" .. (os.hostname and os.hostname() or "unknown"))

    -- Collect registered workflow and activity names
    local wf_names, act_names = {}, {}
    for name in pairs(parent._workflows) do wf_names[#wf_names + 1] = name end
    for name in pairs(parent._activities) do act_names[#act_names + 1] = name end

    -- Register as a worker
    local reg_resp = parent._api("POST", "/workers/register", {
        identity = identity,
        namespace = namespace,
        queue = queue,
        workflows = wf_names,
        activities = act_names,
        max_concurrent_workflows = opts.max_concurrent_workflows or 10,
        max_concurrent_activities = opts.max_concurrent_activities or 20,
    })
    if reg_resp.status ~= 200 then
        error("workflow.listen: registration failed: " .. (reg_resp.body or "unknown"))
    end
    parent._worker_id = json.parse(reg_resp.body).worker_id
    log.info(
        "Registered as worker " .. parent._worker_id
            .. " on namespace '" .. namespace
            .. "' queue '" .. queue .. "'"
    )

    local idle_sleep = 0.5
    local backoff = LISTEN_BACKOFF_MIN_SECS
    while true do
        local hb_ok, hb_err = pcall(function()
            parent._api("POST", "/workers/heartbeat", { worker_id = parent._worker_id })
        end)
        if not hb_ok then
            log.warn(
                "workflow.listen: heartbeat failed, backing off "
                    .. tostring(backoff) .. "s: " .. tostring(hb_err)
            )
            sleep(backoff)
            backoff = math.min(backoff * 2, LISTEN_BACKOFF_MAX_SECS)
            goto continue
        end

        local poll_ok, did_work = pcall(function()
            return M.poll_workflow_task(parent, queue) or M.poll_activity_task(parent, queue)
        end)
        if not poll_ok then
            log.warn(
                "workflow.listen: task poll failed, backing off "
                    .. tostring(backoff) .. "s: " .. tostring(did_work)
            )
            sleep(backoff)
            backoff = math.min(backoff * 2, LISTEN_BACKOFF_MAX_SECS)
            goto continue
        end

        -- First success after a failure resets the exponential backoff.
        backoff = LISTEN_BACKOFF_MIN_SECS
        if not did_work then sleep(idle_sleep) end
        ::continue::
    end
end

--- Poll one workflow task and process it. Returns true if work was done.
function M.poll_workflow_task(parent, queue)
    local resp = parent._api("POST", "/workflow-tasks/poll", {
        queue = queue,
        worker_id = parent._worker_id,
    })
    if resp.status ~= 200 or not resp.body or resp.body == "null" or resp.body == "" then
        return false
    end
    local task = json.parse(resp.body)
    if not task or not task.workflow_id then return false end

    local commands = parent._handle_workflow_task(task)

    parent._api("POST", "/workflow-tasks/" .. task.workflow_id .. "/commands", {
        worker_id = parent._worker_id,
        commands = commands,
    })
    return true
end

--- Poll one activity task and execute it. Returns true if work was done.
function M.poll_activity_task(parent, queue)
    local resp = parent._api("POST", "/tasks/poll", {
        queue = queue,
        worker_id = parent._worker_id,
    })
    if resp.status ~= 200 or not resp.body or resp.body == "null" or resp.body == "" then
        return false
    end
    local task = json.parse(resp.body)
    if not task or not task.id then return false end

    local ok, result_or_err = pcall(function()
        return parent._execute_activity(task)
    end)
    if ok then
        parent._api("POST", "/tasks/" .. task.id .. "/complete", { result = result_or_err })
    else
        parent._api("POST", "/tasks/" .. task.id .. "/fail", { error = tostring(result_or_err) })
    end
    return true
end

return M
