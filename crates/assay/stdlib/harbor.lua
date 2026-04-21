--- @module assay.harbor
--- @description Harbor container registry. Projects, repositories, artifacts, vulnerability scanning.
--- @keywords harbor, registry, artifacts, vulnerabilities, scanning, containers, project, repository, artifact, tag, scan, vulnerability, replication, image, docker, container-registry, oci
--- @quickref c.system:health() -> {status, components} | Check Harbor health
--- @quickref c.system:info() -> {harbor_version, ...} | Get system information
--- @quickref c.system:statistics() -> {private_project_count, ...} | Get registry statistics
--- @quickref c.system:is_healthy() -> bool | Check if all components are healthy
--- @quickref c.projects:list(opts?) -> [project] | List projects
--- @quickref c.projects:get(name_or_id) -> project | Get project by name or ID
--- @quickref c.repositories:list(project_name, opts?) -> [repo] | List repositories in project
--- @quickref c.repositories:get(project_name, repo_name) -> repo | Get repository
--- @quickref c.artifacts:list(project_name, repo_name, opts?) -> [artifact] | List artifacts
--- @quickref c.artifacts:get(project_name, repo_name, reference) -> artifact | Get artifact by reference
--- @quickref c.artifacts:tags(project_name, repo_name, reference) -> [tag] | List artifact tags
--- @quickref c.artifacts:exists(project_name, repo_name, tag) -> bool | Check if image tag exists
--- @quickref c.artifacts:latest(project_name, repo_name) -> artifact|nil | Get latest artifact
--- @quickref c.scan:trigger(project_name, repo_name, reference) -> true | Trigger vulnerability scan
--- @quickref c.scan:vulnerabilities(project, repo, ref) -> {total, critical, ...}|nil | Get vulnerabilities
--- @quickref c.replication:policies() -> [policy] | List replication policies
--- @quickref c.replication:executions(opts?) -> [execution] | List replication executions

local M = {}

function M.client(url, opts)
  opts = opts or {}
  local base_url = url:gsub("/+$", "")
  local api_key = opts.api_key
  local username = opts.username
  local password = opts.password

  -- Shared HTTP helpers (plain closures capturing upvalues)

  local function headers()
    local h = { ["Content-Type"] = "application/json" }
    if api_key then
      h["Authorization"] = "Bearer " .. api_key
    elseif username and password then
      h["Authorization"] = "Basic " .. base64.encode(username .. ":" .. password)
    end
    return h
  end

  local function api_get(path_str)
    local resp = http.get(base_url .. path_str, { headers = headers() })
    if resp.status ~= 200 then
      error("harbor: GET " .. path_str .. " HTTP " .. resp.status .. ": " .. resp.body)
    end
    return json.parse(resp.body)
  end

  local function api_post(path_str, payload)
    local resp = http.post(base_url .. path_str, payload, { headers = headers() })
    return resp
  end

  local function build_qs(params)
    if #params == 0 then return "" end
    return "?" .. table.concat(params, "&")
  end

  -- ===== Client =====

  local c = {}

  -- ===== System =====

  c.system = {}

  function c.system:health()
    return api_get("/api/v2.0/health")
  end

  function c.system:info()
    return api_get("/api/v2.0/systeminfo")
  end

  function c.system:statistics()
    return api_get("/api/v2.0/statistics")
  end

  function c.system:is_healthy()
    local h = c.system:health()
    if not h.components then return false end
    for i = 1, #h.components do
      if h.components[i].status ~= "healthy" then
        return false
      end
    end
    return true
  end

  -- ===== Projects =====

  c.projects = {}

  function c.projects:list(list_opts)
    list_opts = list_opts or {}
    local params = {}
    if list_opts.name then params[#params + 1] = "name=" .. list_opts.name end
    if list_opts.public ~= nil then params[#params + 1] = "public=" .. tostring(list_opts.public) end
    if list_opts.page then params[#params + 1] = "page=" .. list_opts.page end
    if list_opts.page_size then params[#params + 1] = "page_size=" .. list_opts.page_size end
    return api_get("/api/v2.0/projects" .. build_qs(params))
  end

  function c.projects:get(name_or_id)
    return api_get("/api/v2.0/projects/" .. name_or_id)
  end

  -- ===== Repositories =====

  c.repositories = {}

  function c.repositories:list(project_name, list_opts)
    list_opts = list_opts or {}
    local params = {}
    if list_opts.page then params[#params + 1] = "page=" .. list_opts.page end
    if list_opts.page_size then params[#params + 1] = "page_size=" .. list_opts.page_size end
    if list_opts.q then params[#params + 1] = "q=" .. list_opts.q end
    return api_get("/api/v2.0/projects/" .. project_name .. "/repositories" .. build_qs(params))
  end

  function c.repositories:get(project_name, repo_name)
    return api_get("/api/v2.0/projects/" .. project_name .. "/repositories/" .. repo_name)
  end

  -- ===== Artifacts =====

  c.artifacts = {}

  function c.artifacts:list(project_name, repo_name, list_opts)
    list_opts = list_opts or {}
    local params = {}
    if list_opts.page then params[#params + 1] = "page=" .. list_opts.page end
    if list_opts.page_size then params[#params + 1] = "page_size=" .. list_opts.page_size end
    if list_opts.with_tag ~= nil then params[#params + 1] = "with_tag=" .. tostring(list_opts.with_tag) end
    if list_opts.with_scan_overview ~= nil then params[#params + 1] = "with_scan_overview=" .. tostring(list_opts.with_scan_overview) end
    return api_get("/api/v2.0/projects/" .. project_name .. "/repositories/" .. repo_name .. "/artifacts" .. build_qs(params))
  end

  function c.artifacts:get(project_name, repo_name, reference)
    return api_get("/api/v2.0/projects/" .. project_name .. "/repositories/" .. repo_name .. "/artifacts/" .. reference)
  end

  function c.artifacts:tags(project_name, repo_name, reference)
    return api_get("/api/v2.0/projects/" .. project_name .. "/repositories/" .. repo_name .. "/artifacts/" .. reference .. "/tags")
  end

  function c.artifacts:exists(project_name, repo_name, tag)
    local resp = http.get(
      base_url .. "/api/v2.0/projects/" .. project_name .. "/repositories/" .. repo_name .. "/artifacts/" .. tag,
      { headers = headers() }
    )
    return resp.status == 200
  end

  function c.artifacts:latest(project_name, repo_name)
    local list = c.artifacts:list(project_name, repo_name, { page_size = 1 })
    if #list == 0 then return nil end
    return list[1]
  end

  -- ===== Scan =====

  c.scan = {}

  function c.scan:trigger(project_name, repo_name, reference)
    local resp = api_post("/api/v2.0/projects/" .. project_name .. "/repositories/" .. repo_name .. "/artifacts/" .. reference .. "/scan", {})
    if resp.status ~= 202 then
      error("harbor: POST scan HTTP " .. resp.status .. ": " .. resp.body)
    end
    return true
  end

  function c.scan:vulnerabilities(project_name, repo_name, reference)
    local resp = http.get(
      base_url .. "/api/v2.0/projects/" .. project_name .. "/repositories/" .. repo_name .. "/artifacts/" .. reference .. "?with_scan_overview=true",
      { headers = headers() }
    )
    if resp.status ~= 200 then
      error("harbor: GET artifact vulnerabilities HTTP " .. resp.status .. ": " .. resp.body)
    end
    local data = json.parse(resp.body)
    if not data.scan_overview then return nil end
    local report = nil
    for mime_type, val in pairs(data.scan_overview) do
      report = val
      break
    end
    if not report or not report.summary then return nil end
    local sum = report.summary
    return {
      total = sum.total or 0,
      fixable = sum.fixable or 0,
      critical = (sum.summary and sum.summary.Critical) or 0,
      high = (sum.summary and sum.summary.High) or 0,
      medium = (sum.summary and sum.summary.Medium) or 0,
      low = (sum.summary and sum.summary.Low) or 0,
      negligible = (sum.summary and sum.summary.Negligible) or 0,
    }
  end

  -- ===== Replication =====

  c.replication = {}

  function c.replication:policies()
    return api_get("/api/v2.0/replication/policies")
  end

  function c.replication:executions(exec_opts)
    exec_opts = exec_opts or {}
    local params = {}
    if exec_opts.policy_id then params[#params + 1] = "policy_id=" .. exec_opts.policy_id end
    return api_get("/api/v2.0/replication/executions" .. build_qs(params))
  end

  return c
end

return M
