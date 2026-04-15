--- assay.workflow — client library for the assay workflow engine.
---
--- Connect to a running `assay serve` instance and define workflows +
--- activities. Any assay Lua app becomes a workflow worker.
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
---       return http.get(input.url).body
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

--- Define a workflow type.
--- @param name string Workflow type name
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
--- @param workflow_id string
--- @param signal_name string
--- @param payload? any JSON-serializable payload
function M.signal(workflow_id, signal_name, payload)
    local body = payload and { payload = payload } or {}
    local resp = M._api("POST", "/workflows/" .. workflow_id .. "/signal/" .. signal_name, body)
    if resp.status ~= 200 then
        error("workflow.signal failed: " .. (resp.body or "unknown error"))
    end
end

--- Query a workflow's current state.
--- @param workflow_id string
--- @return table Workflow record
function M.describe(workflow_id)
    local resp = M._api("GET", "/workflows/" .. workflow_id)
    if resp.status ~= 200 then
        error("workflow.describe failed: " .. (resp.body or "unknown error"))
    end
    return json.parse(resp.body)
end

--- Cancel a running workflow.
--- @param workflow_id string
function M.cancel(workflow_id)
    local resp = M._api("POST", "/workflows/" .. workflow_id .. "/cancel")
    if resp.status ~= 200 then
        error("workflow.cancel failed: " .. (resp.body or "unknown error"))
    end
end

--- Start listening for tasks. Blocks until cancelled.
--- @param opts table { identity?, queue?, max_concurrent_workflows?, max_concurrent_activities? }
function M.listen(opts)
    if not _engine_url then
        error("workflow.listen: call workflow.connect() first")
    end

    local queue = opts.queue or "default"
    local identity = opts.identity or ("assay-worker-" .. (os.hostname and os.hostname() or "unknown"))

    -- Collect registered workflow and activity names
    local wf_names = {}
    for name in pairs(_workflows) do
        wf_names[#wf_names + 1] = name
    end
    local act_names = {}
    for name in pairs(_activities) do
        act_names[#act_names + 1] = name
    end

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

    local reg = json.parse(reg_resp.body)
    _worker_id = reg.worker_id
    log.info("Registered as worker " .. _worker_id .. " on queue '" .. queue .. "'")

    -- Poll loop
    while true do
        -- Heartbeat
        M._api("POST", "/workers/heartbeat", { worker_id = _worker_id })

        -- Poll for a task
        local poll_resp = M._api("POST", "/tasks/poll", {
            queue = queue,
            worker_id = _worker_id,
        })

        if poll_resp.status == 200 then
            local task = json.parse(poll_resp.body)

            if task and task.id then
                -- Execute the activity
                local ok, result_or_err = pcall(function()
                    return M._execute_activity(task)
                end)

                if ok then
                    M._api("POST", "/tasks/" .. task.id .. "/complete", {
                        result = result_or_err,
                    })
                else
                    M._api("POST", "/tasks/" .. task.id .. "/fail", {
                        error = tostring(result_or_err),
                    })
                end
            else
                -- No task available, wait before polling again
                sleep(1)
            end
        else
            sleep(2)
        end
    end
end

--- Execute an activity task.
function M._execute_activity(task)
    local handler = _activities[task.name]
    if not handler then
        error("No handler registered for activity: " .. (task.name or "?"))
    end

    local input = task.input
    if type(input) == "string" then
        input = json.parse(input)
    end

    local ctx = M._make_activity_ctx(task)
    return handler(ctx, input)
end

--- Create an activity context (passed to activity handlers).
function M._make_activity_ctx(task)
    local ctx = {}

    function ctx:heartbeat(details)
        M._api("POST", "/tasks/" .. task.id .. "/heartbeat", {
            details = details,
        })
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
