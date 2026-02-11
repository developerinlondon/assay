local M = {}

function M.client(url, opts)
  opts = opts or {}
  local c = {
    url = url:gsub("/+$", ""),
    api_key = opts.api_key,
    username = opts.username,
    password = opts.password,
  }

  local function headers(self)
    local h = { ["Content-Type"] = "application/json" }
    if self.api_key then
      h["Authorization"] = "Bearer " .. self.api_key
    elseif self.username and self.password then
      h["Authorization"] = "Basic " .. base64.encode(self.username .. ":" .. self.password)
    end
    return h
  end

  local function api_get(self, path_str)
    local resp = http.get(self.url .. path_str, { headers = headers(self) })
    if resp.status ~= 200 then
      error("harbor: GET " .. path_str .. " HTTP " .. resp.status .. ": " .. resp.body)
    end
    return json.parse(resp.body)
  end

  local function api_post(self, path_str, payload)
    local resp = http.post(self.url .. path_str, payload, { headers = headers(self) })
    return resp
  end

  local function build_qs(params)
    if #params == 0 then return "" end
    return "?" .. table.concat(params, "&")
  end

  function c:health()
    return api_get(self, "/api/v2.0/health")
  end

  function c:system_info()
    return api_get(self, "/api/v2.0/systeminfo")
  end

  function c:statistics()
    return api_get(self, "/api/v2.0/statistics")
  end

  function c:projects(opts)
    opts = opts or {}
    local params = {}
    if opts.name then params[#params + 1] = "name=" .. opts.name end
    if opts.public ~= nil then params[#params + 1] = "public=" .. tostring(opts.public) end
    if opts.page then params[#params + 1] = "page=" .. opts.page end
    if opts.page_size then params[#params + 1] = "page_size=" .. opts.page_size end
    return api_get(self, "/api/v2.0/projects" .. build_qs(params))
  end

  function c:project(name_or_id)
    return api_get(self, "/api/v2.0/projects/" .. name_or_id)
  end

  function c:repositories(project_name, opts)
    opts = opts or {}
    local params = {}
    if opts.page then params[#params + 1] = "page=" .. opts.page end
    if opts.page_size then params[#params + 1] = "page_size=" .. opts.page_size end
    if opts.q then params[#params + 1] = "q=" .. opts.q end
    return api_get(self, "/api/v2.0/projects/" .. project_name .. "/repositories" .. build_qs(params))
  end

  function c:repository(project_name, repo_name)
    return api_get(self, "/api/v2.0/projects/" .. project_name .. "/repositories/" .. repo_name)
  end

  function c:artifacts(project_name, repo_name, opts)
    opts = opts or {}
    local params = {}
    if opts.page then params[#params + 1] = "page=" .. opts.page end
    if opts.page_size then params[#params + 1] = "page_size=" .. opts.page_size end
    if opts.with_tag ~= nil then params[#params + 1] = "with_tag=" .. tostring(opts.with_tag) end
    if opts.with_scan_overview ~= nil then params[#params + 1] = "with_scan_overview=" .. tostring(opts.with_scan_overview) end
    return api_get(self, "/api/v2.0/projects/" .. project_name .. "/repositories/" .. repo_name .. "/artifacts" .. build_qs(params))
  end

  function c:artifact(project_name, repo_name, reference)
    return api_get(self, "/api/v2.0/projects/" .. project_name .. "/repositories/" .. repo_name .. "/artifacts/" .. reference)
  end

  function c:artifact_tags(project_name, repo_name, reference)
    return api_get(self, "/api/v2.0/projects/" .. project_name .. "/repositories/" .. repo_name .. "/artifacts/" .. reference .. "/tags")
  end

  function c:scan_artifact(project_name, repo_name, reference)
    local resp = api_post(self, "/api/v2.0/projects/" .. project_name .. "/repositories/" .. repo_name .. "/artifacts/" .. reference .. "/scan", {})
    if resp.status ~= 202 then
      error("harbor: POST scan HTTP " .. resp.status .. ": " .. resp.body)
    end
    return true
  end

  function c:artifact_vulnerabilities(project_name, repo_name, reference)
    local resp = http.get(
      self.url .. "/api/v2.0/projects/" .. project_name .. "/repositories/" .. repo_name .. "/artifacts/" .. reference .. "?with_scan_overview=true",
      { headers = headers(self) }
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

  function c:replication_policies()
    return api_get(self, "/api/v2.0/replication/policies")
  end

  function c:replication_executions(opts)
    opts = opts or {}
    local params = {}
    if opts.policy_id then params[#params + 1] = "policy_id=" .. opts.policy_id end
    return api_get(self, "/api/v2.0/replication/executions" .. build_qs(params))
  end

  function c:is_healthy()
    local h = self:health()
    if not h.components then return false end
    for i = 1, #h.components do
      if h.components[i].status ~= "healthy" then
        return false
      end
    end
    return true
  end

  function c:image_exists(project_name, repo_name, tag)
    local resp = http.get(
      self.url .. "/api/v2.0/projects/" .. project_name .. "/repositories/" .. repo_name .. "/artifacts/" .. tag,
      { headers = headers(self) }
    )
    return resp.status == 200
  end

  function c:latest_artifact(project_name, repo_name)
    local list = self:artifacts(project_name, repo_name, { page_size = 1 })
    if #list == 0 then return nil end
    return list[1]
  end

  return c
end

return M
