--- @module assay.temporal
--- @description Temporal workflow orchestration. Workflows, task queues, schedules, signals.
--- @keywords temporal, workflows, task-queues, schedules, orchestration, workflow, task-queue, schedule, signal, history, search, namespace, execution
--- @quickref c:health() -> bool | Check Temporal health
--- @quickref c:system_info() -> info | Get system information
--- @quickref c:namespaces() -> {namespaces} | List namespaces
--- @quickref c:namespace(name) -> namespace | Get namespace by name
--- @quickref c:workflows(opts?) -> {executions} | List workflow executions
--- @quickref c:workflow(workflow_id, run_id?, opts?) -> workflow | Get workflow execution
--- @quickref c:workflow_history(workflow_id, run_id?, opts?) -> {events} | Get workflow history
--- @quickref c:signal_workflow(workflow_id, signal_name, input?, opts?) -> result | Signal a workflow
--- @quickref c:terminate_workflow(workflow_id, reason?, opts?) -> result | Terminate a workflow
--- @quickref c:cancel_workflow(workflow_id, opts?) -> result | Cancel a workflow
--- @quickref c:task_queue(name, opts?) -> queue | Get task queue info
--- @quickref c:schedules(opts?) -> {schedules} | List schedules
--- @quickref c:schedule(schedule_id, opts?) -> schedule | Get schedule by ID
--- @quickref c:search(query, opts?) -> {executions} | Search workflows by query
--- @quickref c:is_workflow_running(workflow_id, opts?) -> bool | Check if workflow is running
--- @quickref c:wait_workflow_complete(workflow_id, timeout_secs, opts?) -> workflow | Wait for completion

local M = {}

function M.client(url, opts)
  opts = opts or {}
  local c = {
    url = url:gsub("/+$", ""),
    default_ns = opts.namespace or "default",
    api_key = opts.api_key,
  }

  local function headers(self)
    local h = { ["Content-Type"] = "application/json" }
    if self.api_key then
      h["Authorization"] = "Bearer " .. self.api_key
    end
    return h
  end

  local function resolve_ns(self, opts_override)
    if opts_override and opts_override.namespace then
      return opts_override.namespace
    end
    return self.default_ns
  end

  local function api_get(self, path_str)
    local resp = http.get(self.url .. path_str, { headers = headers(self) })
    if resp.status ~= 200 then
      error("temporal: GET " .. path_str .. " HTTP " .. resp.status .. ": " .. resp.body)
    end
    return json.parse(resp.body)
  end

  local function api_post(self, path_str, payload)
    local resp = http.post(self.url .. path_str, payload, { headers = headers(self) })
    if resp.status ~= 200 then
      error("temporal: POST " .. path_str .. " HTTP " .. resp.status .. ": " .. resp.body)
    end
    return json.parse(resp.body)
  end

  local function build_query_string(params)
    if #params == 0 then return "" end
    return "?" .. table.concat(params, "&")
  end

  function c:health()
    local resp = http.get(self.url .. "/health")
    return resp.status == 200
  end

  function c:system_info()
    return api_get(self, "/api/v1/system-info")
  end

  function c:namespaces()
    return api_get(self, "/api/v1/namespaces")
  end

  function c:namespace(name)
    return api_get(self, "/api/v1/namespaces/" .. name)
  end

  function c:workflows(opts)
    opts = opts or {}
    local ns = resolve_ns(self, opts)
    local params = {}
    if opts.query then params[#params + 1] = "query=" .. opts.query end
    if opts.page_size then params[#params + 1] = "pageSize=" .. opts.page_size end
    local qs = build_query_string(params)
    return api_get(self, "/api/v1/namespaces/" .. ns .. "/workflows" .. qs)
  end

  function c:workflow(workflow_id, run_id, opts)
    opts = opts or {}
    local ns = resolve_ns(self, opts)
    local params = {}
    if run_id then params[#params + 1] = "runId=" .. run_id end
    local qs = build_query_string(params)
    return api_get(self, "/api/v1/namespaces/" .. ns .. "/workflows/" .. workflow_id .. qs)
  end

  function c:workflow_history(workflow_id, run_id, opts)
    opts = opts or {}
    local ns = resolve_ns(self, opts)
    local params = {}
    if run_id then params[#params + 1] = "runId=" .. run_id end
    if opts.maximum_page_size then params[#params + 1] = "maximumPageSize=" .. opts.maximum_page_size end
    local qs = build_query_string(params)
    return api_get(self, "/api/v1/namespaces/" .. ns .. "/workflows/" .. workflow_id .. "/history" .. qs)
  end

  function c:signal_workflow(workflow_id, signal_name, input, opts)
    opts = opts or {}
    local ns = resolve_ns(self, opts)
    local params = {}
    if opts.run_id then params[#params + 1] = "runId=" .. opts.run_id end
    local qs = build_query_string(params)
    local body = { signalName = signal_name }
    if input then body.input = input end
    return api_post(self, "/api/v1/namespaces/" .. ns .. "/workflows/" .. workflow_id .. "/signal" .. qs, body)
  end

  function c:terminate_workflow(workflow_id, reason, opts)
    opts = opts or {}
    local ns = resolve_ns(self, opts)
    local params = {}
    if opts.run_id then params[#params + 1] = "runId=" .. opts.run_id end
    local qs = build_query_string(params)
    local body = {}
    if reason then body.reason = reason end
    return api_post(self, "/api/v1/namespaces/" .. ns .. "/workflows/" .. workflow_id .. "/terminate" .. qs, body)
  end

  function c:cancel_workflow(workflow_id, opts)
    opts = opts or {}
    local ns = resolve_ns(self, opts)
    local params = {}
    if opts.run_id then params[#params + 1] = "runId=" .. opts.run_id end
    local qs = build_query_string(params)
    return api_post(self, "/api/v1/namespaces/" .. ns .. "/workflows/" .. workflow_id .. "/cancel" .. qs, {})
  end

  function c:task_queue(name, opts)
    opts = opts or {}
    local ns = resolve_ns(self, opts)
    local params = {}
    if opts.task_queue_type then params[#params + 1] = "taskQueueType=" .. opts.task_queue_type end
    local qs = build_query_string(params)
    return api_get(self, "/api/v1/namespaces/" .. ns .. "/task-queues/" .. name .. qs)
  end

  function c:schedules(opts)
    opts = opts or {}
    local ns = resolve_ns(self, opts)
    local params = {}
    if opts.maximum_page_size then params[#params + 1] = "maximumPageSize=" .. opts.maximum_page_size end
    local qs = build_query_string(params)
    return api_get(self, "/api/v1/namespaces/" .. ns .. "/schedules" .. qs)
  end

  function c:schedule(schedule_id, opts)
    opts = opts or {}
    local ns = resolve_ns(self, opts)
    return api_get(self, "/api/v1/namespaces/" .. ns .. "/schedules/" .. schedule_id)
  end

  function c:search(query, opts)
    opts = opts or {}
    local ns = resolve_ns(self, opts)
    local params = {}
    if query then params[#params + 1] = "query=" .. query end
    if opts.page_size then params[#params + 1] = "pageSize=" .. opts.page_size end
    local qs = build_query_string(params)
    return api_get(self, "/api/v1/namespaces/" .. ns .. "/workflows" .. qs)
  end

  function c:is_workflow_running(workflow_id, opts)
    opts = opts or {}
    local wf = self:workflow(workflow_id, nil, opts)
    if wf and wf.workflowExecutionInfo and wf.workflowExecutionInfo.status then
      return wf.workflowExecutionInfo.status == "WORKFLOW_EXECUTION_STATUS_RUNNING"
    end
    return false
  end

  function c:wait_workflow_complete(workflow_id, timeout_secs, opts)
    opts = opts or {}
    local deadline = time() + timeout_secs
    while true do
      local wf = self:workflow(workflow_id, nil, opts)
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

  return c
end

return M
