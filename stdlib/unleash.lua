--- @module assay.unleash
--- @description Unleash feature flag management. Projects, features, environments, strategies, API tokens.
--- @keywords unleash, feature-flags, toggles, projects, environments, strategies, feature, toggle, strategy, environment, token, api-token, archive, flag, gradual-rollout
--- @quickref c.health:check() -> {health} | Check Unleash health
--- @quickref c.projects:list() -> [project] | List projects
--- @quickref c.projects:get(id) -> project|nil | Get project by ID
--- @quickref c.projects:create(project) -> project | Create a project
--- @quickref c.projects:update(id, project) -> project | Update a project
--- @quickref c.projects:delete(id) -> nil | Delete a project
--- @quickref c.environments:list() -> [environment] | List environments
--- @quickref c.environments:enable(project_id, env_name) -> nil | Enable environment on project
--- @quickref c.environments:disable(project_id, env_name) -> nil | Disable environment on project
--- @quickref c.features:list(project_id) -> [feature] | List features in project
--- @quickref c.features:get(project_id, name) -> feature|nil | Get feature by name
--- @quickref c.features:create(project_id, feature) -> feature | Create a feature
--- @quickref c.features:update(project_id, name, feature) -> feature | Update a feature
--- @quickref c.features:archive(project_id, name) -> nil | Archive a feature
--- @quickref c.features:toggle_on(project_id, name, env) -> nil | Enable feature in environment
--- @quickref c.features:toggle_off(project_id, name, env) -> nil | Disable feature in environment
--- @quickref c.strategies:list(project_id, feature_name, env) -> [strategy] | List feature strategies
--- @quickref c.strategies:add(project_id, feature_name, env, strategy) -> strategy | Add strategy to feature
--- @quickref c.tokens:list() -> [token] | List API tokens
--- @quickref c.tokens:create(token_config) -> token | Create API token
--- @quickref c.tokens:delete(secret) -> nil | Delete API token
--- @quickref M.wait(url, opts?) -> true | Wait for Unleash to become healthy
--- @quickref M.ensure_project(client, project_id, opts?) -> project | Ensure project exists
--- @quickref M.ensure_environment(client, project_id, env_name) -> true | Ensure environment enabled
--- @quickref M.ensure_token(client, opts) -> token | Ensure API token exists

local M = {}

function M.client(url, opts)
  opts = opts or {}
  local base_url = url:gsub("/+$", "")
  local token = opts.token

  -- Shared HTTP helpers (captured by all sub-object methods as upvalues)

  local function headers()
    local h = { ["Content-Type"] = "application/json" }
    if token then h["Authorization"] = token end
    return h
  end

  local function api_get(path_str)
    local resp = http.get(base_url .. path_str, { headers = headers() })
    if resp.status == 404 then return nil end
    if resp.status ~= 200 then
      error("unleash: GET " .. path_str .. " HTTP " .. resp.status .. ": " .. resp.body)
    end
    return json.parse(resp.body)
  end

  local function api_post(path_str, payload)
    local resp = http.post(base_url .. path_str, payload, { headers = headers() })
    if resp.status ~= 200 and resp.status ~= 201 then
      error("unleash: POST " .. path_str .. " HTTP " .. resp.status .. ": " .. resp.body)
    end
    if resp.body and #resp.body > 0 then
      return json.parse(resp.body)
    end
    return nil
  end

  local function api_put(path_str, payload)
    local resp = http.put(base_url .. path_str, payload, { headers = headers() })
    if resp.status ~= 200 then
      error("unleash: PUT " .. path_str .. " HTTP " .. resp.status .. ": " .. resp.body)
    end
    if resp.body and #resp.body > 0 then
      return json.parse(resp.body)
    end
    return nil
  end

  local function api_delete(path_str)
    local resp = http.delete(base_url .. path_str, { headers = headers() })
    if resp.status ~= 200 then
      error("unleash: DELETE " .. path_str .. " HTTP " .. resp.status .. ": " .. resp.body)
    end
    if resp.body and #resp.body > 0 then
      return json.parse(resp.body)
    end
    return nil
  end

  -- ===== Client =====

  local c = {}

  -- ===== Health =====

  c.health = {}

  function c.health:check()
    local resp = http.get(base_url .. "/health", { headers = headers() })
    if resp.status ~= 200 then
      error("unleash: GET /health HTTP " .. resp.status .. ": " .. resp.body)
    end
    return json.parse(resp.body)
  end

  -- ===== Projects =====

  c.projects = {}

  function c.projects:list()
    local data = api_get("/api/admin/projects")
    if not data then return {} end
    return data.projects or {}
  end

  function c.projects:get(id)
    return api_get("/api/admin/projects/" .. id)
  end

  function c.projects:create(project)
    return api_post("/api/admin/projects", project)
  end

  function c.projects:update(id, project)
    return api_put("/api/admin/projects/" .. id, project)
  end

  function c.projects:delete(id)
    return api_delete("/api/admin/projects/" .. id)
  end

  -- ===== Environments =====

  c.environments = {}

  function c.environments:list()
    local data = api_get("/api/admin/environments")
    if not data then return {} end
    return data.environments or {}
  end

  function c.environments:enable(project_id, env_name)
    return api_post("/api/admin/projects/" .. project_id .. "/environments", {
      environment = env_name,
    })
  end

  function c.environments:disable(project_id, env_name)
    return api_delete("/api/admin/projects/" .. project_id .. "/environments/" .. env_name)
  end

  -- ===== Features =====

  c.features = {}

  function c.features:list(project_id)
    local data = api_get("/api/admin/projects/" .. project_id .. "/features")
    if not data then return {} end
    return data.features or {}
  end

  function c.features:get(project_id, name)
    return api_get("/api/admin/projects/" .. project_id .. "/features/" .. name)
  end

  function c.features:create(project_id, feature)
    return api_post("/api/admin/projects/" .. project_id .. "/features", feature)
  end

  function c.features:update(project_id, name, feature)
    return api_put("/api/admin/projects/" .. project_id .. "/features/" .. name, feature)
  end

  function c.features:archive(project_id, name)
    return api_delete("/api/admin/projects/" .. project_id .. "/features/" .. name)
  end

  function c.features:toggle_on(project_id, name, env)
    return api_post("/api/admin/projects/" .. project_id .. "/features/" .. name .. "/environments/" .. env .. "/on", {})
  end

  function c.features:toggle_off(project_id, name, env)
    return api_post("/api/admin/projects/" .. project_id .. "/features/" .. name .. "/environments/" .. env .. "/off", {})
  end

  -- ===== Strategies =====

  c.strategies = {}

  function c.strategies:list(project_id, feature_name, env)
    local data = api_get("/api/admin/projects/" .. project_id .. "/features/" .. feature_name .. "/environments/" .. env .. "/strategies")
    if not data then return {} end
    if type(data) == "table" and data[1] then return data end
    return data.strategies or data
  end

  function c.strategies:add(project_id, feature_name, env, strategy)
    return api_post("/api/admin/projects/" .. project_id .. "/features/" .. feature_name .. "/environments/" .. env .. "/strategies", strategy)
  end

  -- ===== API Tokens =====

  c.tokens = {}

  function c.tokens:list()
    local data = api_get("/api/admin/api-tokens")
    if not data then return {} end
    return data.tokens or {}
  end

  function c.tokens:create(token_config)
    return api_post("/api/admin/api-tokens", token_config)
  end

  function c.tokens:delete(secret)
    return api_delete("/api/admin/api-tokens/" .. secret)
  end

  return c
end

function M.wait(url, opts)
  opts = opts or {}
  local timeout = opts.timeout or 60
  local interval = opts.interval or 2
  local max_attempts = math.ceil(timeout / interval)

  for i = 1, max_attempts do
    local ok, resp = pcall(http.get, url .. "/health")
    if ok and resp.status == 200 then
      log.info("Unleash healthy after " .. tostring(i * interval) .. "s")
      return true
    end
    if i == max_attempts then
      error("unleash.wait: not reachable at " .. url .. " after " .. tostring(timeout) .. "s")
    end
    log.info("Waiting for Unleash... (" .. tostring(i) .. "/" .. tostring(max_attempts) .. ")")
    sleep(interval)
  end
end

function M.ensure_project(client, project_id, opts)
  opts = opts or {}
  local existing = client.projects:get(project_id)
  if existing then
    log.info("Project already exists: " .. project_id)
    return existing
  end

  local project = {
    id = project_id,
    name = opts.name or project_id,
  }
  if opts.description then
    project.description = opts.description
  end

  local created = client.projects:create(project)
  log.info("Created project: " .. project_id)
  return created
end

function M.ensure_environment(client, project_id, env_name)
  local ok, err = pcall(client.environments.enable, client.environments, project_id, env_name)
  if ok then
    log.info("Enabled environment " .. env_name .. " on project " .. project_id)
    return true
  end

  if type(err) == "string" and (err:find("409") or err:find("already")) then
    log.info("Environment " .. env_name .. " already enabled on project " .. project_id)
    return true
  end

  error("unleash.ensure_environment: " .. tostring(err))
end

function M.ensure_token(client, opts)
  local token_name = opts.tokenName
  assert.not_nil(token_name, "unleash.ensure_token: opts.tokenName is required")
  assert.not_nil(opts.type, "unleash.ensure_token: opts.type is required")

  local existing = client.tokens:list()
  for _, t in ipairs(existing) do
    local match = t.tokenName == token_name and t.type == opts.type
    if match and opts.environment then
      match = t.environment == opts.environment
    end
    if match then
      log.info("Token already exists for " .. token_name .. " (" .. opts.type .. ")")
      return t
    end
  end

  local token_config = {
    tokenName = token_name,
    type = opts.type,
  }
  if opts.environment then
    token_config.environment = opts.environment
  end
  if opts.projects then
    token_config.projects = opts.projects
  end

  local created = client.tokens:create(token_config)
  log.info("Created token for " .. token_name .. " (" .. opts.type .. ")")
  return created
end

return M
