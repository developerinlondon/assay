--- @module assay.sonarqube
--- @description SonarQube code quality API. Quality gates, issues, hotspots, measures, projects.
--- @category devtools
--- @icon sonarqubeserver
--- @keywords sonarqube, sonar, quality-gate, issues, hotspots, measures, projects, code-quality, coverage, security, bugs, vulnerabilities
--- @quickref c.qualitygate:project_status(project_key) -> {projectStatus}|nil | Quality gate status for a project
--- @quickref c.issues:search(opts?) -> {total, issues} | Search issues
--- @quickref c.hotspots:search(opts?) -> {hotspots, paging} | Search security hotspots
--- @quickref c.measures:component(component, metric_keys) -> {component} | Component measures for metric keys
--- @quickref c.projects:search(opts?) -> {components, paging} | Search projects

local M = {}

local function encode(value)
  local out = tostring(value):gsub("([^A-Za-z0-9%-_.~])", function(ch)
    return string.format("%%%02X", string.byte(ch))
  end)
  return out
end

local function build_query(params)
  local parts = {}
  for k, v in pairs(params) do
    if v ~= nil then
      parts[#parts + 1] = k .. "=" .. encode(v)
    end
  end
  table.sort(parts)
  if #parts == 0 then return "" end
  return "?" .. table.concat(parts, "&")
end

function M.client(url, opts)
  opts = opts or {}
  local base_url = url:gsub("/+$", "")
  local token = opts.token
  local user = opts.user
  local password = opts.password

  local function headers()
    local h = { ["Content-Type"] = "application/json" }
    if token then
      h["Authorization"] = "Bearer " .. token
    elseif user then
      h["Authorization"] = "Basic " .. base64.encode(user .. ":" .. (password or ""))
    end
    return h
  end

  local function api_get(path_str)
    local resp = http.get(base_url .. path_str, { headers = headers() })
    if resp.status == 404 then return nil end
    if resp.status ~= 200 then
      error("sonarqube: GET " .. path_str .. " HTTP " .. resp.status .. ": " .. resp.body)
    end
    return json.parse(resp.body)
  end

  -- ===== Client =====

  local c = {}

  -- ===== Quality Gates =====

  c.qualitygate = {}

  function c.qualitygate:project_status(project_key)
    return api_get("/api/qualitygates/project_status" .. build_query({ projectKey = project_key }))
  end

  -- ===== Issues =====

  c.issues = {}

  function c.issues:search(search_opts)
    search_opts = search_opts or {}
    return api_get("/api/issues/search" .. build_query({
      componentKeys = search_opts.component_keys,
      types = search_opts.types,
      severities = search_opts.severities,
      statuses = search_opts.statuses,
      resolved = search_opts.resolved,
      ps = search_opts.page_size,
      p = search_opts.page,
    }))
  end

  -- ===== Hotspots =====

  c.hotspots = {}

  function c.hotspots:search(search_opts)
    search_opts = search_opts or {}
    return api_get("/api/hotspots/search" .. build_query({
      projectKey = search_opts.project_key,
      status = search_opts.status,
      resolution = search_opts.resolution,
      ps = search_opts.page_size,
      p = search_opts.page,
    }))
  end

  -- ===== Measures =====

  c.measures = {}

  function c.measures:component(component, metric_keys)
    local keys = metric_keys
    if type(metric_keys) == "table" then keys = table.concat(metric_keys, ",") end
    return api_get("/api/measures/component" .. build_query({
      component = component,
      metricKeys = keys,
    }))
  end

  -- ===== Projects =====

  c.projects = {}

  function c.projects:search(search_opts)
    search_opts = search_opts or {}
    return api_get("/api/projects/search" .. build_query({
      q = search_opts.query,
      qualifiers = search_opts.qualifiers,
      ps = search_opts.page_size,
      p = search_opts.page,
    }))
  end

  return c
end

return M
