--- @module assay.gitlab
--- @description GitLab REST API v4 — projects, repositories, commits, merge requests, pipelines, issues, releases, container registry.
--- @keywords gitlab, git, ci, cd, merge-request, pipeline, repository, commit, registry, release
--- @quickref c.projects:get(id) -> table|nil | Get project details
--- @quickref c.files:raw(project, path, opts?) -> string|nil | Read raw file content
--- @quickref c.commits:create(project, opts) -> table | Atomic multi-file commit
--- @quickref c.merge_requests:list(project, opts?) -> [table] | List merge requests
--- @quickref c.merge_requests:create(project, opts) -> table | Create merge request
--- @quickref c.merge_requests:merge(project, iid, opts?) -> table | Accept a merge request
--- @quickref c.pipelines:list(project, opts?) -> [table] | List CI/CD pipelines
--- @quickref c.pipelines:create(project, opts) -> table | Trigger a new pipeline
--- @quickref c.branches:list(project, opts?) -> [table] | List branches
--- @quickref c.tags:list(project, opts?) -> [table] | List tags
--- @quickref c.releases:list(project, opts?) -> [table] | List releases
--- @quickref c.issues:list(project, opts?) -> [table] | List issues

local M = {}

function M.client(url, opts)
  opts = opts or {}
  local base_url = url:gsub("/+$", "")
  local token = opts.token
  local oauth_token = opts.oauth_token

  -- Shared HTTP helpers (captured by all sub-object methods as upvalues)

  local function headers()
    local h = { ["Content-Type"] = "application/json" }
    if token then
      h["PRIVATE-TOKEN"] = token
    elseif oauth_token then
      h["Authorization"] = "Bearer " .. oauth_token
    end
    return h
  end

  local function urlencode(str)
    return tostring(str):gsub("([^%w%-%.%_%~])", function(ch)
      return string.format("%%%02X", string.byte(ch))
    end)
  end

  local function encode_project(project)
    if type(project) == "number" then return tostring(project) end
    return urlencode(tostring(project))
  end

  local function build_query(params)
    if not params then return "" end
    local parts = {}
    for k, v in pairs(params) do
      if v ~= nil then
        parts[#parts + 1] = urlencode(tostring(k)) .. "=" .. urlencode(tostring(v))
      end
    end
    table.sort(parts)
    return #parts > 0 and "?" .. table.concat(parts, "&") or ""
  end

  local function api_get(path_str, query_params)
    local resp = http.get(base_url .. "/api/v4" .. path_str .. build_query(query_params),
      { headers = headers() })
    if resp.status == 404 then return nil end
    if resp.status ~= 200 then
      error("gitlab: GET " .. path_str .. " HTTP " .. resp.status .. ": " .. (resp.body or ""))
    end
    return json.parse(resp.body)
  end

  local function api_get_raw(path_str, query_params)
    local h = headers()
    h["Content-Type"] = nil
    local resp = http.get(base_url .. "/api/v4" .. path_str .. build_query(query_params),
      { headers = h })
    if resp.status == 404 then return nil end
    if resp.status ~= 200 then
      error("gitlab: GET " .. path_str .. " HTTP " .. resp.status .. ": " .. (resp.body or ""))
    end
    return resp.body
  end

  local function api_post(path_str, payload)
    local resp = http.post(base_url .. "/api/v4" .. path_str, payload or {}, { headers = headers() })
    if resp.status ~= 200 and resp.status ~= 201 then
      error("gitlab: POST " .. path_str .. " HTTP " .. resp.status .. ": " .. (resp.body or ""))
    end
    return json.parse(resp.body)
  end

  local function api_put(path_str, payload)
    local resp = http.put(base_url .. "/api/v4" .. path_str, payload or {}, { headers = headers() })
    if resp.status ~= 200 then
      error("gitlab: PUT " .. path_str .. " HTTP " .. resp.status .. ": " .. (resp.body or ""))
    end
    return json.parse(resp.body)
  end

  local function api_delete(path_str)
    local resp = http.delete(base_url .. "/api/v4" .. path_str, { headers = headers() })
    if resp.status ~= 200 and resp.status ~= 204 then
      error("gitlab: DELETE " .. path_str .. " HTTP " .. resp.status .. ": " .. (resp.body or ""))
    end
    if resp.body and resp.body ~= "" then
      local ok, parsed = pcall(json.parse, resp.body)
      if ok then return parsed end
    end
    return nil
  end

  local function proj(id) return "/projects/" .. encode_project(id) end

  -- ===== Client =====

  local c = {}

  -- ===== Projects =====

  c.projects = {}

  function c.projects:list(query_opts)
    return api_get("/projects", query_opts)
  end

  function c.projects:get(id)
    return api_get(proj(id))
  end

  -- ===== Repository Files =====

  c.files = {}

  function c.files:get(project, file_path, file_opts)
    file_opts = file_opts or {}
    return api_get(proj(project) .. "/repository/files/" .. urlencode(file_path),
      { ref = file_opts.ref or "main" })
  end

  function c.files:raw(project, file_path, file_opts)
    file_opts = file_opts or {}
    return api_get_raw(proj(project) .. "/repository/files/" .. urlencode(file_path) .. "/raw",
      { ref = file_opts.ref or "main" })
  end

  function c.files:create(project, file_path, file_opts)
    return api_post(proj(project) .. "/repository/files/" .. urlencode(file_path), file_opts)
  end

  function c.files:update(project, file_path, file_opts)
    return api_put(proj(project) .. "/repository/files/" .. urlencode(file_path), file_opts)
  end

  function c.files:delete(project, file_path, file_opts)
    file_opts = file_opts or {}
    local h = headers()
    local resp = http.delete(base_url .. "/api/v4" .. proj(project)
      .. "/repository/files/" .. urlencode(file_path), {
      headers = h,
      body = json.encode(file_opts),
    })
    if resp.status ~= 204 then
      error("gitlab: DELETE file " .. file_path .. " HTTP " .. resp.status .. ": " .. (resp.body or ""))
    end
    return nil
  end

  -- ===== Repository =====

  c.repository = {}

  function c.repository:tree(project, tree_opts)
    return api_get(proj(project) .. "/repository/tree", tree_opts)
  end

  function c.repository:compare(project, from, to)
    return api_get(proj(project) .. "/repository/compare", { from = from, to = to })
  end

  -- ===== Commits =====

  c.commits = {}

  function c.commits:list(project, commit_opts)
    return api_get(proj(project) .. "/repository/commits", commit_opts)
  end

  function c.commits:get(project, sha)
    return api_get(proj(project) .. "/repository/commits/" .. sha)
  end

  function c.commits:create(project, commit_opts)
    return api_post(proj(project) .. "/repository/commits", commit_opts)
  end

  function c.commits:cherry_pick(project, sha, cherry_opts)
    return api_post(proj(project) .. "/repository/commits/" .. sha .. "/cherry_pick", cherry_opts)
  end

  -- ===== Branches =====

  c.branches = {}

  function c.branches:list(project, branch_opts)
    return api_get(proj(project) .. "/repository/branches", branch_opts)
  end

  function c.branches:get(project, name)
    return api_get(proj(project) .. "/repository/branches/" .. urlencode(name))
  end

  function c.branches:create(project, branch_opts)
    return api_post(proj(project) .. "/repository/branches", branch_opts)
  end

  function c.branches:delete(project, name)
    return api_delete(proj(project) .. "/repository/branches/" .. urlencode(name))
  end

  -- ===== Tags =====

  c.tags = {}

  function c.tags:list(project, tag_opts)
    return api_get(proj(project) .. "/repository/tags", tag_opts)
  end

  function c.tags:get(project, name)
    return api_get(proj(project) .. "/repository/tags/" .. urlencode(name))
  end

  function c.tags:create(project, tag_opts)
    return api_post(proj(project) .. "/repository/tags", tag_opts)
  end

  function c.tags:delete(project, name)
    return api_delete(proj(project) .. "/repository/tags/" .. urlencode(name))
  end

  -- ===== Merge Requests =====

  c.merge_requests = {}

  function c.merge_requests:list(project, mr_opts)
    return api_get(proj(project) .. "/merge_requests", mr_opts)
  end

  function c.merge_requests:get(project, iid)
    return api_get(proj(project) .. "/merge_requests/" .. iid)
  end

  function c.merge_requests:create(project, mr_opts)
    return api_post(proj(project) .. "/merge_requests", mr_opts)
  end

  function c.merge_requests:update(project, iid, mr_opts)
    return api_put(proj(project) .. "/merge_requests/" .. iid, mr_opts)
  end

  function c.merge_requests:merge(project, iid, merge_opts)
    return api_put(proj(project) .. "/merge_requests/" .. iid .. "/merge", merge_opts)
  end

  function c.merge_requests:approve(project, iid)
    return api_post(proj(project) .. "/merge_requests/" .. iid .. "/approve")
  end

  function c.merge_requests:changes(project, iid)
    return api_get(proj(project) .. "/merge_requests/" .. iid .. "/changes")
  end

  function c.merge_requests:notes(project, iid, note_opts)
    return api_get(proj(project) .. "/merge_requests/" .. iid .. "/notes", note_opts)
  end

  function c.merge_requests:create_note(project, iid, body)
    return api_post(proj(project) .. "/merge_requests/" .. iid .. "/notes", { body = body })
  end

  -- ===== Pipelines =====

  c.pipelines = {}

  function c.pipelines:list(project, pipe_opts)
    return api_get(proj(project) .. "/pipelines", pipe_opts)
  end

  function c.pipelines:get(project, id)
    return api_get(proj(project) .. "/pipelines/" .. id)
  end

  function c.pipelines:create(project, pipe_opts)
    return api_post(proj(project) .. "/pipeline", pipe_opts)
  end

  function c.pipelines:cancel(project, id)
    return api_post(proj(project) .. "/pipelines/" .. id .. "/cancel")
  end

  function c.pipelines:retry(project, id)
    return api_post(proj(project) .. "/pipelines/" .. id .. "/retry")
  end

  function c.pipelines:delete(project, id)
    return api_delete(proj(project) .. "/pipelines/" .. id)
  end

  function c.pipelines:jobs(project, pipeline_id, job_opts)
    return api_get(proj(project) .. "/pipelines/" .. pipeline_id .. "/jobs", job_opts)
  end

  -- ===== Jobs =====

  c.jobs = {}

  function c.jobs:list(project, job_opts)
    return api_get(proj(project) .. "/jobs", job_opts)
  end

  function c.jobs:get(project, id)
    return api_get(proj(project) .. "/jobs/" .. id)
  end

  function c.jobs:retry(project, id)
    return api_post(proj(project) .. "/jobs/" .. id .. "/retry")
  end

  function c.jobs:cancel(project, id)
    return api_post(proj(project) .. "/jobs/" .. id .. "/cancel")
  end

  function c.jobs:log(project, id)
    return api_get_raw(proj(project) .. "/jobs/" .. id .. "/trace")
  end

  -- ===== Releases =====

  c.releases = {}

  function c.releases:list(project, release_opts)
    return api_get(proj(project) .. "/releases", release_opts)
  end

  function c.releases:get(project, tag_name)
    return api_get(proj(project) .. "/releases/" .. urlencode(tag_name))
  end

  function c.releases:create(project, release_opts)
    return api_post(proj(project) .. "/releases", release_opts)
  end

  function c.releases:update(project, tag_name, release_opts)
    return api_put(proj(project) .. "/releases/" .. urlencode(tag_name), release_opts)
  end

  function c.releases:delete(project, tag_name)
    return api_delete(proj(project) .. "/releases/" .. urlencode(tag_name))
  end

  -- ===== Issues =====

  c.issues = {}

  function c.issues:list(project, issue_opts)
    if project then
      return api_get(proj(project) .. "/issues", issue_opts)
    end
    return api_get("/issues", issue_opts)
  end

  function c.issues:get(project, iid)
    return api_get(proj(project) .. "/issues/" .. iid)
  end

  function c.issues:create(project, issue_opts)
    return api_post(proj(project) .. "/issues", issue_opts)
  end

  function c.issues:update(project, iid, issue_opts)
    return api_put(proj(project) .. "/issues/" .. iid, issue_opts)
  end

  function c.issues:notes(project, iid, note_opts)
    return api_get(proj(project) .. "/issues/" .. iid .. "/notes", note_opts)
  end

  function c.issues:create_note(project, iid, body)
    return api_post(proj(project) .. "/issues/" .. iid .. "/notes", { body = body })
  end

  -- ===== Groups =====

  c.groups = {}

  function c.groups:list(group_opts)
    return api_get("/groups", group_opts)
  end

  function c.groups:get(id)
    return api_get("/groups/" .. encode_project(id))
  end

  function c.groups:projects(id, group_opts)
    return api_get("/groups/" .. encode_project(id) .. "/projects", group_opts)
  end

  -- ===== Container Registry =====

  c.registry = {}

  function c.registry:repositories(project)
    return api_get(proj(project) .. "/registry/repositories")
  end

  function c.registry:tags(project, repo_id)
    return api_get(proj(project) .. "/registry/repositories/" .. repo_id .. "/tags")
  end

  function c.registry:tag(project, repo_id, tag_name)
    return api_get(proj(project) .. "/registry/repositories/" .. repo_id
      .. "/tags/" .. urlencode(tag_name))
  end

  function c.registry:delete_tag(project, repo_id, tag_name)
    return api_delete(proj(project) .. "/registry/repositories/" .. repo_id
      .. "/tags/" .. urlencode(tag_name))
  end

  -- ===== Webhooks =====

  c.hooks = {}

  function c.hooks:list(project)
    return api_get(proj(project) .. "/hooks")
  end

  function c.hooks:get(project, id)
    return api_get(proj(project) .. "/hooks/" .. id)
  end

  function c.hooks:create(project, hook_opts)
    return api_post(proj(project) .. "/hooks", hook_opts)
  end

  function c.hooks:update(project, id, hook_opts)
    return api_put(proj(project) .. "/hooks/" .. id, hook_opts)
  end

  function c.hooks:delete(project, id)
    return api_delete(proj(project) .. "/hooks/" .. id)
  end

  -- ===== Users =====

  c.users = {}

  function c.users:current()
    return api_get("/user")
  end

  function c.users:list(user_opts)
    return api_get("/users", user_opts)
  end

  -- ===== Environments =====

  c.environments = {}

  function c.environments:list(project, env_opts)
    return api_get(proj(project) .. "/environments", env_opts)
  end

  function c.environments:get(project, id)
    return api_get(proj(project) .. "/environments/" .. id)
  end

  -- ===== Deploy Tokens =====

  c.deploy_tokens = {}

  function c.deploy_tokens:list(project)
    return api_get(proj(project) .. "/deploy_tokens")
  end

  function c.deploy_tokens:create(project, dt_opts)
    return api_post(proj(project) .. "/deploy_tokens", dt_opts)
  end

  function c.deploy_tokens:delete(project, id)
    return api_delete(proj(project) .. "/deploy_tokens/" .. id)
  end

  return c
end

return M
