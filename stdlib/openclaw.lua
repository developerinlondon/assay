--- @module assay.openclaw
--- @description OpenClaw AI agent platform integration. Invoke tools, send messages, manage state, spawn sub-agents, approval gates, LLM tasks.
--- @keywords openclaw, clawd, agent, ai, workflow, invoke, state, diff, approve, llm, cron, spawn, message, notify
--- @quickref c:invoke(tool, action, args) -> result | Invoke an OpenClaw tool action
--- @quickref c:send(channel, target, message) -> result | Send a message via channel
--- @quickref c:notify(target, message) -> result | Send notification to target
--- @quickref c:cron_add(job) -> result | Add a cron job
--- @quickref c:cron_list() -> [job] | List cron jobs
--- @quickref c:spawn(task, opts?) -> result | Spawn a sub-agent session
--- @quickref c:state_get(key) -> value|nil | Read state value by key
--- @quickref c:state_set(key, value) -> true | Write state value by key
--- @quickref c:diff(key, new_value) -> {changed, before, after} | Compare and store state
--- @quickref c:approve(prompt, context?) -> bool | Request approval gate
--- @quickref c:llm_task(prompt, opts?) -> result | Execute an LLM task

local M = {}

function M.client(url, opts)
  opts = opts or {}

  -- Auto-discover URL from env vars
  if not url then
    url = env.get("OPENCLAW_URL") or env.get("CLAWD_URL")
    if not url then
      error("openclaw: url required (set $OPENCLAW_URL or $CLAWD_URL)")
    end
  end

  local token = opts.token
  if not token then
    token = env.get("OPENCLAW_TOKEN") or env.get("CLAWD_TOKEN")
  end

  local state_dir = opts.state_dir
  if not state_dir then
    state_dir = env.get("ASSAY_STATE_DIR") or env.get("OPENCLAW_STATE_DIR")
    if not state_dir then
      local home = env.get("HOME") or "/tmp"
      state_dir = home .. "/.assay/state"
    end
  end

  local c = {
    url = url:gsub("/+$", ""),
    token = token,
    state_dir = state_dir,
  }

  local function headers(self)
    local h = { ["Content-Type"] = "application/json" }
    if self.token then
      h["Authorization"] = "Bearer " .. self.token
    end
    return h
  end

  local function api_post(self, path_str, payload)
    local resp = http.post(self.url .. path_str, payload, { headers = headers(self) })
    if resp.status ~= 200 and resp.status ~= 201 then
      error("openclaw: POST " .. path_str .. " HTTP " .. resp.status .. ": " .. resp.body)
    end
    return json.parse(resp.body)
  end

  function c:invoke(tool, action, args)
    return api_post(self, "/tools/invoke", {
      tool = tool,
      action = action,
      args = args or {},
    })
  end

  function c:send(channel, target, message)
    return self:invoke("message", "send", {
      channel = channel,
      target = target,
      message = message,
    })
  end

  function c:notify(target, message)
    return self:invoke("message", "send", {
      target = target,
      message = message,
    })
  end

  function c:cron_add(job)
    return self:invoke("cron", "add", { job = job })
  end

  function c:cron_list()
    return self:invoke("cron", "list", {})
  end

  function c:spawn(task, spawn_opts)
    spawn_opts = spawn_opts or {}
    local args = { task = task }
    if spawn_opts.model then args.model = spawn_opts.model end
    if spawn_opts.timeout then args.timeout = spawn_opts.timeout end
    return self:invoke("sessions_spawn", "invoke", args)
  end

  function c:state_get(key)
    local path = self.state_dir .. "/" .. key .. ".json"
    if not fs.exists(path) then return nil end
    local content = fs.read(path)
    if not content or content == "" then return nil end
    return json.parse(content)
  end

  function c:state_set(key, value)
    local path = self.state_dir .. "/" .. key .. ".json"
    local dir = self.state_dir
    if not fs.exists(dir) then
      fs.mkdir(dir)
    end
    fs.write(path, json.encode(value))
    return true
  end

  function c:diff(key, new_value)
    local before = self:state_get(key)
    self:state_set(key, new_value)
    local changed = json.encode(before) ~= json.encode(new_value)
    return {
      changed = changed,
      before = before,
      after = new_value,
    }
  end

  function c:approve(prompt, context)
    local approval_result = env.get("ASSAY_APPROVAL_RESULT")
    if approval_result then
      return approval_result == "yes"
    end

    local mode = env.get("ASSAY_MODE")
    if mode == "tool" then
      error("__assay_approval_request__:" .. json.encode({
        prompt = prompt,
        context = context,
      }))
    end

    local interactive = env.get("OPENCLAW_INTERACTIVE") or env.get("TTY")
    if interactive then
      log.info("Approval required: " .. prompt)
      if context then
        log.info("Context: " .. json.encode(context))
      end
      return false
    end

    error("openclaw: approval_required: " .. json.encode({
      type = "approval_request",
      prompt = prompt,
      context = context,
    }))
  end

  function c:llm_task(prompt, llm_opts)
    llm_opts = llm_opts or {}
    local args = { prompt = prompt }
    if llm_opts.model then args.model = llm_opts.model end
    if llm_opts.artifacts then args.artifacts = llm_opts.artifacts end
    if llm_opts.output_schema then args.output_schema = llm_opts.output_schema end
    if llm_opts.temperature then args.temperature = llm_opts.temperature end
    if llm_opts.max_output_tokens then args.max_output_tokens = llm_opts.max_output_tokens end
    return self:invoke("llm-task", "invoke", args)
  end

  return c
end

return M
