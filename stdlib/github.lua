--- @module assay.github
--- @description GitHub REST API client. PRs, issues, actions, repositories, GraphQL. No gh CLI dependency.
--- @keywords github, pr, pull-request, issue, actions, runs, graphql, repository, merge, review, comment
--- @quickref c:pr_view(repo, number) -> pr | Get pull request details
--- @quickref c:pr_list(repo, opts?) -> [pr] | List pull requests
--- @quickref c:pr_reviews(repo, number) -> [review] | List PR reviews
--- @quickref c:pr_merge(repo, number, opts?) -> result | Merge a pull request
--- @quickref c:issue_list(repo, opts?) -> [issue] | List issues
--- @quickref c:issue_get(repo, number) -> issue | Get issue details
--- @quickref c:issue_create(repo, title, body, opts?) -> issue | Create an issue
--- @quickref c:issue_comment(repo, number, body) -> comment | Add issue comment
--- @quickref c:repo_get(repo) -> repository | Get repository details
--- @quickref c:runs_list(repo, opts?) -> {workflow_runs} | List workflow runs
--- @quickref c:run_get(repo, run_id) -> run | Get workflow run details
--- @quickref c:graphql(query, variables?) -> data | Execute GraphQL query

local M = {}

function M.client(opts)
  opts = opts or {}
  local token = opts.token or env.get("GITHUB_TOKEN")
  local base_url = opts.base_url or "https://api.github.com"

  local c = {
    base_url = base_url:gsub("/+$", ""),
    token = token,
  }

  local function headers(self)
    local h = {
      ["Content-Type"] = "application/json",
      ["Accept"] = "application/vnd.github+json",
    }
    if self.token then
      h["Authorization"] = "Bearer " .. self.token
    end
    return h
  end

  local function parse_repo(repo)
    local owner, name = repo:match("^([^/]+)/(.+)$")
    if not owner then
      error("github: invalid repo format, expected 'owner/repo': " .. repo)
    end
    return owner, name
  end

  local function api_get(self, path_str)
    local resp = http.get(self.base_url .. path_str, { headers = headers(self) })
    if resp.status == 404 then return nil end
    if resp.status ~= 200 then
      error("github: GET " .. path_str .. " HTTP " .. resp.status .. ": " .. resp.body)
    end
    return json.parse(resp.body)
  end

  local function api_post(self, path_str, payload)
    local resp = http.post(self.base_url .. path_str, payload, { headers = headers(self) })
    if resp.status ~= 200 and resp.status ~= 201 then
      error("github: POST " .. path_str .. " HTTP " .. resp.status .. ": " .. resp.body)
    end
    return json.parse(resp.body)
  end

  local function api_put(self, path_str, payload)
    local resp = http.put(self.base_url .. path_str, payload or {}, { headers = headers(self) })
    if resp.status ~= 200 and resp.status ~= 204 then
      error("github: PUT " .. path_str .. " HTTP " .. resp.status .. ": " .. resp.body)
    end
    if resp.body and resp.body ~= "" then
      return json.parse(resp.body)
    end
    return true
  end

  function c:pr_view(repo, number)
    local owner, name = parse_repo(repo)
    return api_get(self, "/repos/" .. owner .. "/" .. name .. "/pulls/" .. number)
  end

  function c:pr_list(repo, pr_opts)
    pr_opts = pr_opts or {}
    local owner, name = parse_repo(repo)
    local params = {}
    if pr_opts.state then params[#params + 1] = "state=" .. pr_opts.state end
    if pr_opts.sort then params[#params + 1] = "sort=" .. pr_opts.sort end
    if pr_opts.direction then params[#params + 1] = "direction=" .. pr_opts.direction end
    if pr_opts.per_page then params[#params + 1] = "per_page=" .. pr_opts.per_page end
    local qs = ""
    if #params > 0 then qs = "?" .. table.concat(params, "&") end
    return api_get(self, "/repos/" .. owner .. "/" .. name .. "/pulls" .. qs)
  end

  function c:pr_reviews(repo, number)
    local owner, name = parse_repo(repo)
    return api_get(self, "/repos/" .. owner .. "/" .. name .. "/pulls/" .. number .. "/reviews")
  end

  function c:pr_merge(repo, number, merge_opts)
    merge_opts = merge_opts or {}
    local owner, name = parse_repo(repo)
    local payload = {}
    if merge_opts.merge_method then payload.merge_method = merge_opts.merge_method end
    if merge_opts.commit_title then payload.commit_title = merge_opts.commit_title end
    if merge_opts.commit_message then payload.commit_message = merge_opts.commit_message end
    return api_put(self, "/repos/" .. owner .. "/" .. name .. "/pulls/" .. number .. "/merge", payload)
  end

  function c:issue_list(repo, issue_opts)
    issue_opts = issue_opts or {}
    local owner, name = parse_repo(repo)
    local params = {}
    if issue_opts.state then params[#params + 1] = "state=" .. issue_opts.state end
    if issue_opts.labels then params[#params + 1] = "labels=" .. issue_opts.labels end
    if issue_opts.sort then params[#params + 1] = "sort=" .. issue_opts.sort end
    if issue_opts.direction then params[#params + 1] = "direction=" .. issue_opts.direction end
    if issue_opts.per_page then params[#params + 1] = "per_page=" .. issue_opts.per_page end
    local qs = ""
    if #params > 0 then qs = "?" .. table.concat(params, "&") end
    return api_get(self, "/repos/" .. owner .. "/" .. name .. "/issues" .. qs)
  end

  function c:issue_get(repo, number)
    local owner, name = parse_repo(repo)
    return api_get(self, "/repos/" .. owner .. "/" .. name .. "/issues/" .. number)
  end

  function c:issue_create(repo, title, body, create_opts)
    create_opts = create_opts or {}
    local owner, name = parse_repo(repo)
    local payload = {
      title = title,
      body = body,
    }
    if create_opts.labels then payload.labels = create_opts.labels end
    if create_opts.assignees then payload.assignees = create_opts.assignees end
    if create_opts.milestone then payload.milestone = create_opts.milestone end
    return api_post(self, "/repos/" .. owner .. "/" .. name .. "/issues", payload)
  end

  function c:issue_comment(repo, number, body)
    local owner, name = parse_repo(repo)
    return api_post(self, "/repos/" .. owner .. "/" .. name .. "/issues/" .. number .. "/comments", {
      body = body,
    })
  end

  function c:repo_get(repo)
    local owner, name = parse_repo(repo)
    return api_get(self, "/repos/" .. owner .. "/" .. name)
  end

  function c:runs_list(repo, runs_opts)
    runs_opts = runs_opts or {}
    local owner, name = parse_repo(repo)
    local params = {}
    if runs_opts.status then params[#params + 1] = "status=" .. runs_opts.status end
    if runs_opts.branch then params[#params + 1] = "branch=" .. runs_opts.branch end
    if runs_opts.per_page then params[#params + 1] = "per_page=" .. runs_opts.per_page end
    local qs = ""
    if #params > 0 then qs = "?" .. table.concat(params, "&") end
    return api_get(self, "/repos/" .. owner .. "/" .. name .. "/actions/runs" .. qs)
  end

  function c:run_get(repo, run_id)
    local owner, name = parse_repo(repo)
    return api_get(self, "/repos/" .. owner .. "/" .. name .. "/actions/runs/" .. run_id)
  end

  function c:graphql(query, variables)
    local payload = { query = query }
    if variables then payload.variables = variables end
    return api_post(self, "/graphql", payload)
  end

  return c
end

return M
