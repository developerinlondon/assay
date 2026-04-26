--- @module assay.github
--- @description GitHub REST API client. PRs, issues, actions, repositories, GraphQL, releases. No gh CLI dependency.
--- @keywords github, pr, pull-request, issue, actions, runs, graphql, repository, merge, review, comment, release, asset, checksum
--- @quickref c.pulls:get(repo, number) -> pr | Get pull request details
--- @quickref c.pulls:list(repo, opts?) -> [pr] | List pull requests
--- @quickref c.pulls:reviews(repo, number) -> [review] | List PR reviews
--- @quickref c.pulls:merge(repo, number, opts?) -> result | Merge a pull request
--- @quickref c.issues:list(repo, opts?) -> [issue] | List issues
--- @quickref c.issues:get(repo, number) -> issue | Get issue details
--- @quickref c.issues:create(repo, title, body, opts?) -> issue | Create an issue
--- @quickref c.issues:create_note(repo, number, body) -> comment | Add issue comment
--- @quickref c.repos:get(repo) -> repository | Get repository details
--- @quickref c.runs:list(repo, opts?) -> {workflow_runs} | List workflow runs
--- @quickref c.runs:get(repo, run_id) -> run | Get workflow run details
--- @quickref c:graphql(query, variables?) -> data | Execute GraphQL query
--- @quickref github.latest_release(owner, repo, opts?) -> release | Get latest release
--- @quickref github.find_asset(release, name_pattern) -> asset | Match asset by Lua pattern
--- @quickref github.fetch_asset_text(asset) -> string | Download asset body
--- @quickref github.fetch_asset_bytes(asset) -> string | Download asset body (alias)
--- @quickref github.release_checksum(release, opts) -> hex | Look up sibling .sha256 digest

local M = {}

function M.client(opts)
  opts = opts or {}
  local token = opts.token or env.get("GITHUB_TOKEN")
  local base_url = (opts.base_url or "https://api.github.com"):gsub("/+$", "")

  -- Shared HTTP helpers (captured by all sub-object methods as upvalues)

  local function headers()
    local h = {
      ["Content-Type"] = "application/json",
      ["Accept"] = "application/vnd.github+json",
    }
    if token then
      h["Authorization"] = "Bearer " .. token
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

  local function api_get(path_str)
    local resp = http.get(base_url .. path_str, { headers = headers() })
    if resp.status == 404 then return nil end
    if resp.status ~= 200 then
      error("github: GET " .. path_str .. " HTTP " .. resp.status .. ": " .. resp.body)
    end
    return json.parse(resp.body)
  end

  local function api_post(path_str, payload)
    local resp = http.post(base_url .. path_str, payload, { headers = headers() })
    if resp.status ~= 200 and resp.status ~= 201 then
      error("github: POST " .. path_str .. " HTTP " .. resp.status .. ": " .. resp.body)
    end
    return json.parse(resp.body)
  end

  local function api_put(path_str, payload)
    local resp = http.put(base_url .. path_str, payload or {}, { headers = headers() })
    if resp.status ~= 200 and resp.status ~= 204 then
      error("github: PUT " .. path_str .. " HTTP " .. resp.status .. ": " .. resp.body)
    end
    if resp.body and resp.body ~= "" then
      return json.parse(resp.body)
    end
    return true
  end

  -- ===== Client =====

  local c = {}

  -- ===== Pull Requests =====

  c.pulls = {}

  function c.pulls:get(repo, number)
    local owner, name = parse_repo(repo)
    return api_get("/repos/" .. owner .. "/" .. name .. "/pulls/" .. number)
  end

  function c.pulls:list(repo, pr_opts)
    pr_opts = pr_opts or {}
    local owner, name = parse_repo(repo)
    local params = {}
    if pr_opts.state then params[#params + 1] = "state=" .. pr_opts.state end
    if pr_opts.sort then params[#params + 1] = "sort=" .. pr_opts.sort end
    if pr_opts.direction then params[#params + 1] = "direction=" .. pr_opts.direction end
    if pr_opts.per_page then params[#params + 1] = "per_page=" .. pr_opts.per_page end
    local qs = ""
    if #params > 0 then qs = "?" .. table.concat(params, "&") end
    return api_get("/repos/" .. owner .. "/" .. name .. "/pulls" .. qs)
  end

  function c.pulls:reviews(repo, number)
    local owner, name = parse_repo(repo)
    return api_get("/repos/" .. owner .. "/" .. name .. "/pulls/" .. number .. "/reviews")
  end

  function c.pulls:merge(repo, number, merge_opts)
    merge_opts = merge_opts or {}
    local owner, name = parse_repo(repo)
    local payload = {}
    if merge_opts.merge_method then payload.merge_method = merge_opts.merge_method end
    if merge_opts.commit_title then payload.commit_title = merge_opts.commit_title end
    if merge_opts.commit_message then payload.commit_message = merge_opts.commit_message end
    return api_put("/repos/" .. owner .. "/" .. name .. "/pulls/" .. number .. "/merge", payload)
  end

  -- ===== Issues =====

  c.issues = {}

  function c.issues:list(repo, issue_opts)
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
    return api_get("/repos/" .. owner .. "/" .. name .. "/issues" .. qs)
  end

  function c.issues:get(repo, number)
    local owner, name = parse_repo(repo)
    return api_get("/repos/" .. owner .. "/" .. name .. "/issues/" .. number)
  end

  function c.issues:create(repo, title, body, create_opts)
    create_opts = create_opts or {}
    local owner, name = parse_repo(repo)
    local payload = {
      title = title,
      body = body,
    }
    if create_opts.labels then payload.labels = create_opts.labels end
    if create_opts.assignees then payload.assignees = create_opts.assignees end
    if create_opts.milestone then payload.milestone = create_opts.milestone end
    return api_post("/repos/" .. owner .. "/" .. name .. "/issues", payload)
  end

  function c.issues:create_note(repo, number, body)
    local owner, name = parse_repo(repo)
    return api_post("/repos/" .. owner .. "/" .. name .. "/issues/" .. number .. "/comments", {
      body = body,
    })
  end

  -- ===== Repositories =====

  c.repos = {}

  function c.repos:get(repo)
    local owner, name = parse_repo(repo)
    return api_get("/repos/" .. owner .. "/" .. name)
  end

  -- ===== Workflow Runs =====

  c.runs = {}

  function c.runs:list(repo, runs_opts)
    runs_opts = runs_opts or {}
    local owner, name = parse_repo(repo)
    local params = {}
    if runs_opts.status then params[#params + 1] = "status=" .. runs_opts.status end
    if runs_opts.branch then params[#params + 1] = "branch=" .. runs_opts.branch end
    if runs_opts.per_page then params[#params + 1] = "per_page=" .. runs_opts.per_page end
    local qs = ""
    if #params > 0 then qs = "?" .. table.concat(params, "&") end
    return api_get("/repos/" .. owner .. "/" .. name .. "/actions/runs" .. qs)
  end

  function c.runs:get(repo, run_id)
    local owner, name = parse_repo(repo)
    return api_get("/repos/" .. owner .. "/" .. name .. "/actions/runs/" .. run_id)
  end

  -- ===== GraphQL (top-level, not resource-scoped) =====

  function c:graphql(query, variables)
    local payload = { query = query }
    if variables then payload.variables = variables end
    return api_post("/graphql", payload)
  end

  return c
end

-- ===== Releases (module-level helpers) =====

local function release_headers(token)
  local h = {
    ["Accept"] = "application/vnd.github+json",
  }
  if token then
    h["Authorization"] = "Bearer " .. token
  end
  return h
end

local function release_token(opts)
  return opts.token or env.get("GITHUB_TOKEN") or env.get("GH_TOKEN")
end

local function release_base_url(opts)
  local base = opts.base_url or "https://api.github.com"
  return (base:gsub("/+$", ""))
end

--- Get the latest release for a repository.
--- @param owner string Repo owner
--- @param repo string Repo name
--- @param opts table? `{ token = "...", base_url = "..." }`
--- @return table release JSON release with extra `version` field (tag_name with leading "v" stripped)
function M.latest_release(owner, repo, opts)
  opts = opts or {}
  local base_url = release_base_url(opts)
  local url = base_url .. "/repos/" .. owner .. "/" .. repo .. "/releases/latest"
  local resp = http.get(url, { headers = release_headers(release_token(opts)) })
  if resp.status ~= 200 then
    error("github.latest_release: GET " .. url .. " HTTP " .. resp.status .. ": " .. resp.body)
  end
  local rel = json.parse(resp.body)
  if rel.tag_name then
    rel.version = (rel.tag_name:gsub("^v", ""))
  end
  return rel
end

--- Find the first asset whose name matches a Lua pattern.
--- @param release table Release returned by `latest_release`.
--- @param name_pattern string Lua pattern matched against `asset.name`.
--- @return table|nil asset
function M.find_asset(release, name_pattern)
  if not release or not release.assets then return nil end
  for _, asset in ipairs(release.assets) do
    if asset.name and asset.name:match(name_pattern) then
      return asset
    end
  end
  return nil
end

--- Download an asset body as text. Lua strings are byte buffers, so this is
--- safe for binary data too — `fetch_asset_bytes` is provided as an alias
--- for clarity at the call site.
--- @param asset table Asset table containing `browser_download_url`.
--- @return string body
function M.fetch_asset_text(asset)
  if not asset or not asset.browser_download_url then
    error("github.fetch_asset_text: asset missing browser_download_url")
  end
  local resp = http.get(asset.browser_download_url)
  if resp.status ~= 200 then
    error(
      "github.fetch_asset_text: GET "
        .. asset.browser_download_url
        .. " HTTP "
        .. resp.status
    )
  end
  return resp.body
end

--- Alias of `fetch_asset_text`. Lua strings are bytes; both are equivalent.
function M.fetch_asset_bytes(asset)
  return M.fetch_asset_text(asset)
end

--- Look up a checksum recorded in a sibling `<asset>.<digest>` release file.
--- Common GitHub-release convention: `tool.tar.gz` ships next to
--- `tool.tar.gz.sha256`, where the latter holds `<hex>  tool.tar.gz`.
--- @param release table Release table.
--- @param opts table `{ asset_pattern = "...", digest = "sha256" }`.
--- @return string hex Lowercase hex digest.
function M.release_checksum(release, opts)
  opts = opts or {}
  local asset_pattern = opts.asset_pattern
    or error("github.release_checksum: opts.asset_pattern required")
  local digest = opts.digest or "sha256"

  local primary = M.find_asset(release, asset_pattern)
  if not primary then
    error("github.release_checksum: no asset matching pattern: " .. asset_pattern)
  end

  local checksum_name = primary.name .. "." .. digest
  local checksum_asset
  for _, asset in ipairs(release.assets or {}) do
    if asset.name == checksum_name then
      checksum_asset = asset
      break
    end
  end
  if not checksum_asset then
    error("github.release_checksum: no sibling asset named: " .. checksum_name)
  end

  local body = M.fetch_asset_text(checksum_asset)
  local hex = body:match("^(%x+)")
  if not hex then
    error("github.release_checksum: could not extract hex digest from: " .. checksum_name)
  end
  return hex:lower()
end

return M
