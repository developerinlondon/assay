--- @module assay.unleash
--- @description Unleash feature flag management. Projects, features, environments, strategies, API tokens.
--- @keywords unleash, feature-flags, toggles, projects, environments, strategies
--- @quickref c:health() -> {health} | Check Unleash health
--- @quickref c:projects() -> [project] | List projects
--- @quickref c:project(id) -> project|nil | Get project by ID
--- @quickref c:create_project(project) -> project | Create a project
--- @quickref c:update_project(id, project) -> project | Update a project
--- @quickref c:delete_project(id) -> nil | Delete a project
--- @quickref c:environments() -> [environment] | List environments
--- @quickref c:enable_environment(project_id, env_name) -> nil | Enable environment on project
--- @quickref c:disable_environment(project_id, env_name) -> nil | Disable environment on project
--- @quickref c:features(project_id) -> [feature] | List features in project
--- @quickref c:feature(project_id, name) -> feature|nil | Get feature by name
--- @quickref c:create_feature(project_id, feature) -> feature | Create a feature
--- @quickref c:update_feature(project_id, name, feature) -> feature | Update a feature
--- @quickref c:archive_feature(project_id, name) -> nil | Archive a feature
--- @quickref c:toggle_on(project_id, name, env) -> nil | Enable feature in environment
--- @quickref c:toggle_off(project_id, name, env) -> nil | Disable feature in environment
--- @quickref c:strategies(project_id, feature_name, env) -> [strategy] | List feature strategies
--- @quickref c:add_strategy(project_id, feature_name, env, strategy) -> strategy | Add strategy to feature
--- @quickref c:tokens() -> [token] | List API tokens
--- @quickref c:create_token(token_config) -> token | Create API token
--- @quickref c:delete_token(secret) -> nil | Delete API token
--- @quickref M.wait(url, opts?) -> true | Wait for Unleash to become healthy
--- @quickref M.ensure_project(client, project_id, opts?) -> project | Ensure project exists
--- @quickref M.ensure_environment(client, project_id, env_name) -> true | Ensure environment enabled
--- @quickref M.ensure_token(client, opts) -> token | Ensure API token exists

local M = {}

function M.client(url, opts)
  opts = opts or {}
  local c = {
    url = url:gsub("/+$", ""),
    token = opts.token,
  }

  local function headers(self)
    local h = { ["Content-Type"] = "application/json" }
    if self.token then h["Authorization"] = self.token end
    return h
  end

  local function api_get(self, path_str)
    local resp = http.get(self.url .. path_str, { headers = headers(self) })
    if resp.status == 404 then return nil end
    if resp.status ~= 200 then
      error("unleash: GET " .. path_str .. " HTTP " .. resp.status .. ": " .. resp.body)
    end
    return json.parse(resp.body)
  end

  local function api_post(self, path_str, payload)
    local resp = http.post(self.url .. path_str, payload, { headers = headers(self) })
    if resp.status ~= 200 and resp.status ~= 201 then
      error("unleash: POST " .. path_str .. " HTTP " .. resp.status .. ": " .. resp.body)
    end
    if resp.body and #resp.body > 0 then
      return json.parse(resp.body)
    end
    return nil
  end

  local function api_put(self, path_str, payload)
    local resp = http.put(self.url .. path_str, payload, { headers = headers(self) })
    if resp.status ~= 200 then
      error("unleash: PUT " .. path_str .. " HTTP " .. resp.status .. ": " .. resp.body)
    end
    if resp.body and #resp.body > 0 then
      return json.parse(resp.body)
    end
    return nil
  end

  local function api_delete(self, path_str)
    local resp = http.delete(self.url .. path_str, { headers = headers(self) })
    if resp.status ~= 200 then
      error("unleash: DELETE " .. path_str .. " HTTP " .. resp.status .. ": " .. resp.body)
    end
    if resp.body and #resp.body > 0 then
      return json.parse(resp.body)
    end
    return nil
  end

  -- Health

  function c:health()
    local resp = http.get(self.url .. "/health", { headers = headers(self) })
    if resp.status ~= 200 then
      error("unleash: GET /health HTTP " .. resp.status .. ": " .. resp.body)
    end
    return json.parse(resp.body)
  end

  -- Projects

  function c:projects()
    local data = api_get(self, "/api/admin/projects")
    if not data then return {} end
    return data.projects or {}
  end

  function c:project(id)
    return api_get(self, "/api/admin/projects/" .. id)
  end

  function c:create_project(project)
    return api_post(self, "/api/admin/projects", project)
  end

  function c:update_project(id, project)
    return api_put(self, "/api/admin/projects/" .. id, project)
  end

  function c:delete_project(id)
    return api_delete(self, "/api/admin/projects/" .. id)
  end

  -- Environments

  function c:environments()
    local data = api_get(self, "/api/admin/environments")
    if not data then return {} end
    return data.environments or {}
  end

  function c:enable_environment(project_id, env_name)
    return api_post(self, "/api/admin/projects/" .. project_id .. "/environments", {
      environment = env_name,
    })
  end

  function c:disable_environment(project_id, env_name)
    return api_delete(self, "/api/admin/projects/" .. project_id .. "/environments/" .. env_name)
  end

  -- Features

  function c:features(project_id)
    local data = api_get(self, "/api/admin/projects/" .. project_id .. "/features")
    if not data then return {} end
    return data.features or {}
  end

  function c:feature(project_id, name)
    return api_get(self, "/api/admin/projects/" .. project_id .. "/features/" .. name)
  end

  function c:create_feature(project_id, feature)
    return api_post(self, "/api/admin/projects/" .. project_id .. "/features", feature)
  end

  function c:update_feature(project_id, name, feature)
    return api_put(self, "/api/admin/projects/" .. project_id .. "/features/" .. name, feature)
  end

  function c:archive_feature(project_id, name)
    return api_delete(self, "/api/admin/projects/" .. project_id .. "/features/" .. name)
  end

  function c:toggle_on(project_id, name, env)
    return api_post(self, "/api/admin/projects/" .. project_id .. "/features/" .. name .. "/environments/" .. env .. "/on", {})
  end

  function c:toggle_off(project_id, name, env)
    return api_post(self, "/api/admin/projects/" .. project_id .. "/features/" .. name .. "/environments/" .. env .. "/off", {})
  end

  -- Strategies

  function c:strategies(project_id, feature_name, env)
    local data = api_get(self, "/api/admin/projects/" .. project_id .. "/features/" .. feature_name .. "/environments/" .. env .. "/strategies")
    if not data then return {} end
    if type(data) == "table" and data[1] then return data end
    return data.strategies or data
  end

  function c:add_strategy(project_id, feature_name, env, strategy)
    return api_post(self, "/api/admin/projects/" .. project_id .. "/features/" .. feature_name .. "/environments/" .. env .. "/strategies", strategy)
  end

  -- API Tokens

  function c:tokens()
    local data = api_get(self, "/api/admin/api-tokens")
    if not data then return {} end
    return data.tokens or {}
  end

  function c:create_token(token_config)
    return api_post(self, "/api/admin/api-tokens", token_config)
  end

  function c:delete_token(secret)
    return api_delete(self, "/api/admin/api-tokens/" .. secret)
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
  local existing = client:project(project_id)
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

  local created = client:create_project(project)
  log.info("Created project: " .. project_id)
  return created
end

function M.ensure_environment(client, project_id, env_name)
  local ok, err = pcall(client.enable_environment, client, project_id, env_name)
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
  assert.not_nil(opts.username, "unleash.ensure_token: opts.username is required")
  assert.not_nil(opts.type, "unleash.ensure_token: opts.type is required")

  local existing = client:tokens()
  for _, t in ipairs(existing) do
    local match = t.username == opts.username and t.type == opts.type
    if match and opts.environment then
      match = t.environment == opts.environment
    end
    if match then
      log.info("Token already exists for " .. opts.username .. " (" .. opts.type .. ")")
      return t
    end
  end

  local token_config = {
    username = opts.username,
    type = opts.type,
  }
  if opts.environment then
    token_config.environment = opts.environment
  end
  if opts.projects then
    token_config.projects = opts.projects
  end

  local created = client:create_token(token_config)
  log.info("Created token for " .. opts.username .. " (" .. opts.type .. ")")
  return created
end

return M
