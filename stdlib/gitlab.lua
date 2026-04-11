--- @module assay.gitlab
--- @description GitLab REST API v4 — projects, repositories, commits, merge requests, pipelines, issues, releases, container registry.
--- @keywords gitlab, git, ci, cd, merge-request, pipeline, repository, commit, registry, release
--- @quickref c:project(id) -> table|nil | Get project details
--- @quickref c:file_raw(project, path, opts?) -> string|nil | Read raw file content from repository
--- @quickref c:create_commit(project, opts) -> table | Atomic multi-file commit
--- @quickref c:merge_requests(project, opts?) -> [table] | List merge requests
--- @quickref c:create_merge_request(project, opts) -> table | Create merge request
--- @quickref c:merge(project, iid, opts?) -> table | Accept (merge) a merge request
--- @quickref c:pipelines(project, opts?) -> [table] | List CI/CD pipelines
--- @quickref c:create_pipeline(project, opts) -> table | Trigger a new pipeline
--- @quickref c:branches(project, opts?) -> [table] | List branches
--- @quickref c:tags(project, opts?) -> [table] | List tags
--- @quickref c:releases(project, opts?) -> [table] | List releases
--- @quickref c:issues(project, opts?) -> [table] | List issues

local M = {}

function M.client(url, opts)
  opts = opts or {}
  local c = {
    url = url:gsub("/+$", ""),
    token = opts.token,
    oauth_token = opts.oauth_token,
  }

  -- Auth: PRIVATE-TOKEN header (personal/project access token) or OAuth2 Bearer
  local function headers(self)
    local h = { ["Content-Type"] = "application/json" }
    if self.token then
      h["PRIVATE-TOKEN"] = self.token
    elseif self.oauth_token then
      h["Authorization"] = "Bearer " .. self.oauth_token
    end
    return h
  end

  local function urlencode(str)
    return tostring(str):gsub("([^%w%-%.%_%~])", function(ch)
      return string.format("%%%02X", string.byte(ch))
    end)
  end

  -- Encode project ID: numeric IDs pass through, namespace/name paths get URL-encoded
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

  local function api_get(self, path_str, query_params)
    local resp = http.get(self.url .. "/api/v4" .. path_str .. build_query(query_params),
      { headers = headers(self) })
    if resp.status == 404 then return nil end
    if resp.status ~= 200 then
      error("gitlab: GET " .. path_str .. " HTTP " .. resp.status .. ": " .. (resp.body or ""))
    end
    return json.parse(resp.body)
  end

  local function api_get_raw(self, path_str, query_params)
    local h = headers(self)
    h["Content-Type"] = nil
    local resp = http.get(self.url .. "/api/v4" .. path_str .. build_query(query_params),
      { headers = h })
    if resp.status == 404 then return nil end
    if resp.status ~= 200 then
      error("gitlab: GET " .. path_str .. " HTTP " .. resp.status .. ": " .. (resp.body or ""))
    end
    return resp.body
  end

  local function api_post(self, path_str, payload)
    local resp = http.post(self.url .. "/api/v4" .. path_str, payload or {}, { headers = headers(self) })
    if resp.status ~= 200 and resp.status ~= 201 then
      error("gitlab: POST " .. path_str .. " HTTP " .. resp.status .. ": " .. (resp.body or ""))
    end
    return json.parse(resp.body)
  end

  local function api_put(self, path_str, payload)
    local resp = http.put(self.url .. "/api/v4" .. path_str, payload or {}, { headers = headers(self) })
    if resp.status ~= 200 then
      error("gitlab: PUT " .. path_str .. " HTTP " .. resp.status .. ": " .. (resp.body or ""))
    end
    return json.parse(resp.body)
  end

  local function api_delete(self, path_str)
    local h = headers(self)
    local resp = http.delete(self.url .. "/api/v4" .. path_str, { headers = h })
    if resp.status ~= 200 and resp.status ~= 204 then
      error("gitlab: DELETE " .. path_str .. " HTTP " .. resp.status .. ": " .. (resp.body or ""))
    end
    if resp.body and resp.body ~= "" then
      local ok, parsed = pcall(json.parse, resp.body)
      if ok then return parsed end
    end
    return nil
  end

  -- ===== Projects =====

  function c:projects(query_opts)
    return api_get(self, "/projects", query_opts)
  end

  function c:project(id)
    return api_get(self, "/projects/" .. encode_project(id))
  end

  -- ===== Repository Files =====

  function c:file(project, file_path, file_opts)
    file_opts = file_opts or {}
    local params = { ref = file_opts.ref or "main" }
    return api_get(self, "/projects/" .. encode_project(project)
      .. "/repository/files/" .. urlencode(file_path), params)
  end

  function c:file_raw(project, file_path, file_opts)
    file_opts = file_opts or {}
    local params = { ref = file_opts.ref or "main" }
    return api_get_raw(self, "/projects/" .. encode_project(project)
      .. "/repository/files/" .. urlencode(file_path) .. "/raw", params)
  end

  function c:create_file(project, file_path, file_opts)
    return api_post(self, "/projects/" .. encode_project(project)
      .. "/repository/files/" .. urlencode(file_path), file_opts)
  end

  function c:update_file(project, file_path, file_opts)
    return api_put(self, "/projects/" .. encode_project(project)
      .. "/repository/files/" .. urlencode(file_path), file_opts)
  end

  function c:delete_file(project, file_path, file_opts)
    file_opts = file_opts or {}
    local h = headers(self)
    local resp = http.delete(self.url .. "/api/v4/projects/" .. encode_project(project)
      .. "/repository/files/" .. urlencode(file_path), {
      headers = h,
      body = json.encode(file_opts),
    })
    if resp.status ~= 204 then
      error("gitlab: DELETE file " .. file_path .. " HTTP " .. resp.status .. ": " .. (resp.body or ""))
    end
    return nil
  end

  -- ===== Repository Tree =====

  function c:tree(project, tree_opts)
    return api_get(self, "/projects/" .. encode_project(project) .. "/repository/tree", tree_opts)
  end

  function c:compare(project, from, to)
    return api_get(self, "/projects/" .. encode_project(project) .. "/repository/compare",
      { from = from, to = to })
  end

  -- ===== Commits =====

  function c:commits(project, commit_opts)
    return api_get(self, "/projects/" .. encode_project(project) .. "/repository/commits", commit_opts)
  end

  function c:commit(project, sha)
    return api_get(self, "/projects/" .. encode_project(project) .. "/repository/commits/" .. sha)
  end

  function c:create_commit(project, commit_opts)
    return api_post(self, "/projects/" .. encode_project(project) .. "/repository/commits", commit_opts)
  end

  function c:cherry_pick(project, sha, cherry_opts)
    return api_post(self, "/projects/" .. encode_project(project)
      .. "/repository/commits/" .. sha .. "/cherry_pick", cherry_opts)
  end

  -- ===== Branches =====

  function c:branches(project, branch_opts)
    return api_get(self, "/projects/" .. encode_project(project) .. "/repository/branches", branch_opts)
  end

  function c:branch(project, name)
    return api_get(self, "/projects/" .. encode_project(project)
      .. "/repository/branches/" .. urlencode(name))
  end

  function c:create_branch(project, branch_opts)
    return api_post(self, "/projects/" .. encode_project(project) .. "/repository/branches", branch_opts)
  end

  function c:delete_branch(project, name)
    return api_delete(self, "/projects/" .. encode_project(project)
      .. "/repository/branches/" .. urlencode(name))
  end

  -- ===== Tags =====

  function c:tags(project, tag_opts)
    return api_get(self, "/projects/" .. encode_project(project) .. "/repository/tags", tag_opts)
  end

  function c:tag(project, name)
    return api_get(self, "/projects/" .. encode_project(project)
      .. "/repository/tags/" .. urlencode(name))
  end

  function c:create_tag(project, tag_opts)
    return api_post(self, "/projects/" .. encode_project(project) .. "/repository/tags", tag_opts)
  end

  function c:delete_tag(project, name)
    return api_delete(self, "/projects/" .. encode_project(project)
      .. "/repository/tags/" .. urlencode(name))
  end

  -- ===== Merge Requests =====

  function c:merge_requests(project, mr_opts)
    return api_get(self, "/projects/" .. encode_project(project) .. "/merge_requests", mr_opts)
  end

  function c:merge_request(project, iid)
    return api_get(self, "/projects/" .. encode_project(project) .. "/merge_requests/" .. iid)
  end

  function c:create_merge_request(project, mr_opts)
    return api_post(self, "/projects/" .. encode_project(project) .. "/merge_requests", mr_opts)
  end

  function c:update_merge_request(project, iid, mr_opts)
    return api_put(self, "/projects/" .. encode_project(project) .. "/merge_requests/" .. iid, mr_opts)
  end

  function c:merge(project, iid, merge_opts)
    return api_put(self, "/projects/" .. encode_project(project)
      .. "/merge_requests/" .. iid .. "/merge", merge_opts)
  end

  function c:approve_merge_request(project, iid)
    return api_post(self, "/projects/" .. encode_project(project) .. "/merge_requests/" .. iid .. "/approve")
  end

  function c:merge_request_changes(project, iid)
    return api_get(self, "/projects/" .. encode_project(project) .. "/merge_requests/" .. iid .. "/changes")
  end

  function c:merge_request_notes(project, iid, note_opts)
    return api_get(self, "/projects/" .. encode_project(project)
      .. "/merge_requests/" .. iid .. "/notes", note_opts)
  end

  function c:create_merge_request_note(project, iid, body)
    return api_post(self, "/projects/" .. encode_project(project)
      .. "/merge_requests/" .. iid .. "/notes", { body = body })
  end

  -- ===== Pipelines =====

  function c:pipelines(project, pipe_opts)
    return api_get(self, "/projects/" .. encode_project(project) .. "/pipelines", pipe_opts)
  end

  function c:pipeline(project, id)
    return api_get(self, "/projects/" .. encode_project(project) .. "/pipelines/" .. id)
  end

  function c:create_pipeline(project, pipe_opts)
    return api_post(self, "/projects/" .. encode_project(project) .. "/pipeline", pipe_opts)
  end

  function c:cancel_pipeline(project, id)
    return api_post(self, "/projects/" .. encode_project(project) .. "/pipelines/" .. id .. "/cancel")
  end

  function c:retry_pipeline(project, id)
    return api_post(self, "/projects/" .. encode_project(project) .. "/pipelines/" .. id .. "/retry")
  end

  function c:delete_pipeline(project, id)
    return api_delete(self, "/projects/" .. encode_project(project) .. "/pipelines/" .. id)
  end

  -- ===== Jobs =====

  function c:pipeline_jobs(project, pipeline_id, job_opts)
    return api_get(self, "/projects/" .. encode_project(project)
      .. "/pipelines/" .. pipeline_id .. "/jobs", job_opts)
  end

  function c:jobs(project, job_opts)
    return api_get(self, "/projects/" .. encode_project(project) .. "/jobs", job_opts)
  end

  function c:job(project, id)
    return api_get(self, "/projects/" .. encode_project(project) .. "/jobs/" .. id)
  end

  function c:retry_job(project, id)
    return api_post(self, "/projects/" .. encode_project(project) .. "/jobs/" .. id .. "/retry")
  end

  function c:cancel_job(project, id)
    return api_post(self, "/projects/" .. encode_project(project) .. "/jobs/" .. id .. "/cancel")
  end

  function c:job_log(project, id)
    return api_get_raw(self, "/projects/" .. encode_project(project) .. "/jobs/" .. id .. "/trace")
  end

  -- ===== Releases =====

  function c:releases(project, release_opts)
    return api_get(self, "/projects/" .. encode_project(project) .. "/releases", release_opts)
  end

  function c:release(project, tag_name)
    return api_get(self, "/projects/" .. encode_project(project)
      .. "/releases/" .. urlencode(tag_name))
  end

  function c:create_release(project, release_opts)
    return api_post(self, "/projects/" .. encode_project(project) .. "/releases", release_opts)
  end

  function c:update_release(project, tag_name, release_opts)
    return api_put(self, "/projects/" .. encode_project(project)
      .. "/releases/" .. urlencode(tag_name), release_opts)
  end

  function c:delete_release(project, tag_name)
    return api_delete(self, "/projects/" .. encode_project(project)
      .. "/releases/" .. urlencode(tag_name))
  end

  -- ===== Issues =====

  function c:issues(project, issue_opts)
    if project then
      return api_get(self, "/projects/" .. encode_project(project) .. "/issues", issue_opts)
    end
    return api_get(self, "/issues", issue_opts)
  end

  function c:issue(project, iid)
    return api_get(self, "/projects/" .. encode_project(project) .. "/issues/" .. iid)
  end

  function c:create_issue(project, issue_opts)
    return api_post(self, "/projects/" .. encode_project(project) .. "/issues", issue_opts)
  end

  function c:update_issue(project, iid, issue_opts)
    return api_put(self, "/projects/" .. encode_project(project) .. "/issues/" .. iid, issue_opts)
  end

  function c:issue_notes(project, iid, note_opts)
    return api_get(self, "/projects/" .. encode_project(project)
      .. "/issues/" .. iid .. "/notes", note_opts)
  end

  function c:create_issue_note(project, iid, body)
    return api_post(self, "/projects/" .. encode_project(project)
      .. "/issues/" .. iid .. "/notes", { body = body })
  end

  -- ===== Groups =====

  function c:groups(group_opts)
    return api_get(self, "/groups", group_opts)
  end

  function c:group(id)
    return api_get(self, "/groups/" .. encode_project(id))
  end

  function c:group_projects(id, group_opts)
    return api_get(self, "/groups/" .. encode_project(id) .. "/projects", group_opts)
  end

  -- ===== Container Registry =====

  function c:registries(project)
    return api_get(self, "/projects/" .. encode_project(project) .. "/registry/repositories")
  end

  function c:registry_tags(project, repo_id)
    return api_get(self, "/projects/" .. encode_project(project)
      .. "/registry/repositories/" .. repo_id .. "/tags")
  end

  function c:registry_tag(project, repo_id, tag_name)
    return api_get(self, "/projects/" .. encode_project(project)
      .. "/registry/repositories/" .. repo_id .. "/tags/" .. urlencode(tag_name))
  end

  function c:delete_registry_tag(project, repo_id, tag_name)
    return api_delete(self, "/projects/" .. encode_project(project)
      .. "/registry/repositories/" .. repo_id .. "/tags/" .. urlencode(tag_name))
  end

  -- ===== Webhooks (Project Hooks) =====

  function c:hooks(project)
    return api_get(self, "/projects/" .. encode_project(project) .. "/hooks")
  end

  function c:hook(project, id)
    return api_get(self, "/projects/" .. encode_project(project) .. "/hooks/" .. id)
  end

  function c:create_hook(project, hook_opts)
    return api_post(self, "/projects/" .. encode_project(project) .. "/hooks", hook_opts)
  end

  function c:update_hook(project, id, hook_opts)
    return api_put(self, "/projects/" .. encode_project(project) .. "/hooks/" .. id, hook_opts)
  end

  function c:delete_hook(project, id)
    return api_delete(self, "/projects/" .. encode_project(project) .. "/hooks/" .. id)
  end

  -- ===== Users =====

  function c:current_user()
    return api_get(self, "/user")
  end

  function c:users(user_opts)
    return api_get(self, "/users", user_opts)
  end

  -- ===== Environments =====

  function c:environments(project, env_opts)
    return api_get(self, "/projects/" .. encode_project(project) .. "/environments", env_opts)
  end

  function c:environment(project, id)
    return api_get(self, "/projects/" .. encode_project(project) .. "/environments/" .. id)
  end

  -- ===== Deploy Tokens =====

  function c:deploy_tokens(project)
    return api_get(self, "/projects/" .. encode_project(project) .. "/deploy_tokens")
  end

  function c:create_deploy_token(project, dt_opts)
    return api_post(self, "/projects/" .. encode_project(project) .. "/deploy_tokens", dt_opts)
  end

  function c:delete_deploy_token(project, id)
    return api_delete(self, "/projects/" .. encode_project(project) .. "/deploy_tokens/" .. id)
  end

  return c
end

return M
