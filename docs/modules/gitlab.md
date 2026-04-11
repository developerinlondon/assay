## assay.gitlab

GitLab REST API v4 client. Projects, repositories, commits, merge requests, pipelines, jobs,
issues, releases, groups, container registry, webhooks, environments, and deploy tokens.

Supports both private access token (`PRIVATE-TOKEN` header) and OAuth2 bearer authentication.

```lua
local gitlab = require("assay.gitlab")
local c = gitlab.client("https://gitlab.example.com", { token = "glpat-xxxx" })
```

### Projects

- `c:projects(opts?)` -> `[project]` — List projects. Options: `search`, `order_by`, `sort`, `per_page`, `page`
- `c:project(id)` -> `project|nil` — Get project by numeric ID or `"namespace/name"` path

### Repository Files

- `c:file(project, path, opts?)` -> `table|nil` — Get file metadata (base64-encoded content). Options: `ref` (default: `"main"`)
- `c:file_raw(project, path, opts?)` -> `string|nil` — Get raw file content as string. Options: `ref` (default: `"main"`)
- `c:create_file(project, path, opts)` -> `table` — Create file. Options: `branch`, `content`, `commit_message`
- `c:update_file(project, path, opts)` -> `table` — Update file. Options: `branch`, `content`, `commit_message`
- `c:delete_file(project, path, opts)` -> `nil` — Delete file. Options: `branch`, `commit_message`

### Repository

- `c:tree(project, opts?)` -> `[entry]` — List repository tree. Options: `path`, `ref`, `recursive`, `per_page`
- `c:compare(project, from, to)` -> `{commits, diffs}` — Compare branches, tags, or commits

### Commits

- `c:commits(project, opts?)` -> `[commit]` — List commits. Options: `ref_name`, `since`, `until`, `path`, `per_page`
- `c:commit(project, sha)` -> `commit|nil` — Get single commit by SHA
- `c:create_commit(project, opts)` -> `commit` — Atomic multi-file commit. Options: `branch`, `commit_message`, `actions` (array of `{action, file_path, content}`)
- `c:cherry_pick(project, sha, opts)` -> `commit` — Cherry-pick commit. Options: `branch`

### Branches

- `c:branches(project, opts?)` -> `[branch]` — List branches. Options: `search`, `per_page`
- `c:branch(project, name)` -> `branch|nil` — Get branch by name
- `c:create_branch(project, opts)` -> `branch` — Create branch. Options: `branch`, `ref`
- `c:delete_branch(project, name)` -> `nil` — Delete branch

### Tags

- `c:tags(project, opts?)` -> `[tag]` — List tags. Options: `search`, `order_by`, `sort`
- `c:tag(project, name)` -> `tag|nil` — Get tag by name
- `c:create_tag(project, opts)` -> `tag` — Create tag. Options: `tag_name`, `ref`, `message`
- `c:delete_tag(project, name)` -> `nil` — Delete tag

### Merge Requests

- `c:merge_requests(project, opts?)` -> `[mr]` — List MRs. Options: `state`, `order_by`, `sort`, `labels`, `per_page`
- `c:merge_request(project, iid)` -> `mr|nil` — Get MR by IID
- `c:create_merge_request(project, opts)` -> `mr` — Create MR. Options: `source_branch`, `target_branch`, `title`, `description`
- `c:update_merge_request(project, iid, opts)` -> `mr` — Update MR. Options: `title`, `description`, `state_event`, `labels`
- `c:merge(project, iid, opts?)` -> `mr` — Accept (merge) MR. Options: `squash`, `merge_commit_message`, `should_remove_source_branch`
- `c:approve_merge_request(project, iid)` -> `table` — Approve MR
- `c:merge_request_changes(project, iid)` -> `mr` — Get MR with diff changes
- `c:merge_request_notes(project, iid, opts?)` -> `[note]` — List MR comments
- `c:create_merge_request_note(project, iid, body)` -> `note` — Add comment to MR

### Pipelines

- `c:pipelines(project, opts?)` -> `[pipeline]` — List pipelines. Options: `ref`, `status`, `per_page`
- `c:pipeline(project, id)` -> `pipeline|nil` — Get pipeline by ID
- `c:create_pipeline(project, opts)` -> `pipeline` — Trigger pipeline. Options: `ref`, `variables`
- `c:cancel_pipeline(project, id)` -> `pipeline` — Cancel running pipeline
- `c:retry_pipeline(project, id)` -> `pipeline` — Retry failed pipeline
- `c:delete_pipeline(project, id)` -> `nil` — Delete pipeline

### Jobs

- `c:pipeline_jobs(project, pipeline_id, opts?)` -> `[job]` — List jobs for a pipeline
- `c:jobs(project, opts?)` -> `[job]` — List all project jobs. Options: `scope` (array of statuses)
- `c:job(project, id)` -> `job|nil` — Get job by ID
- `c:retry_job(project, id)` -> `job` — Retry a job
- `c:cancel_job(project, id)` -> `job` — Cancel a job
- `c:job_log(project, id)` -> `string|nil` — Get job trace/log output as raw text

### Releases

- `c:releases(project, opts?)` -> `[release]` — List releases. Options: `per_page`
- `c:release(project, tag_name)` -> `release|nil` — Get release by tag name
- `c:create_release(project, opts)` -> `release` — Create release. Options: `tag_name`, `name`, `description`
- `c:update_release(project, tag_name, opts)` -> `release` — Update release
- `c:delete_release(project, tag_name)` -> `nil` — Delete release

### Issues

- `c:issues(project, opts?)` -> `[issue]` — List issues. Pass `nil` as project for global issues. Options: `state`, `labels`, `search`, `per_page`
- `c:issue(project, iid)` -> `issue|nil` — Get issue by IID
- `c:create_issue(project, opts)` -> `issue` — Create issue. Options: `title`, `description`, `labels`, `assignee_ids`
- `c:update_issue(project, iid, opts)` -> `issue` — Update issue. Options: `title`, `description`, `state_event`, `labels`
- `c:issue_notes(project, iid, opts?)` -> `[note]` — List issue comments
- `c:create_issue_note(project, iid, body)` -> `note` — Add comment to issue

### Groups

- `c:groups(opts?)` -> `[group]` — List groups. Options: `search`, `per_page`
- `c:group(id)` -> `group|nil` — Get group by numeric ID or `"path"` name
- `c:group_projects(id, opts?)` -> `[project]` — List projects in a group

### Container Registry

- `c:registries(project)` -> `[repo]` — List container registry repositories
- `c:registry_tags(project, repo_id)` -> `[tag]` — List tags for a registry repository
- `c:registry_tag(project, repo_id, tag_name)` -> `tag|nil` — Get single registry tag (with digest, size)
- `c:delete_registry_tag(project, repo_id, tag_name)` -> `nil` — Delete a registry tag

### Webhooks (Project Hooks)

- `c:hooks(project)` -> `[hook]` — List project hooks
- `c:hook(project, id)` -> `hook|nil` — Get hook by ID
- `c:create_hook(project, opts)` -> `hook` — Create hook. Options: `url`, `push_events`, `merge_requests_events`, etc.
- `c:update_hook(project, id, opts)` -> `hook` — Update hook
- `c:delete_hook(project, id)` -> `nil` — Delete hook

### Environments

- `c:environments(project, opts?)` -> `[env]` — List environments. Options: `search`
- `c:environment(project, id)` -> `env|nil` — Get environment by ID

### Deploy Tokens

- `c:deploy_tokens(project)` -> `[token]` — List deploy tokens
- `c:create_deploy_token(project, opts)` -> `token` — Create token. Options: `name`, `scopes`, `expires_at`
- `c:delete_deploy_token(project, id)` -> `nil` — Delete token

### Users

- `c:current_user()` -> `user` — Get authenticated user
- `c:users(opts?)` -> `[user]` — Search users. Options: `username`, `search`, `per_page`

### Example: Atomic Multi-File Commit

```lua
local gitlab = require("assay.gitlab")
local c = gitlab.client("https://gitlab.example.com", { token = env.get("GITLAB_TOKEN") })

local result = c:create_commit(42, {
    branch = "main",
    commit_message = "Update config for v2.0",
    actions = {
        { action = "update", file_path = "config/app.yaml", content = "version: 2.0\n" },
        { action = "update", file_path = "config/db.yaml",  content = "pool_size: 20\n" },
    },
})
log.info("Committed: " .. result.short_id)
```

### Example: Create and Merge an MR

```lua
local gitlab = require("assay.gitlab")
local c = gitlab.client("https://gitlab.example.com", { token = env.get("GITLAB_TOKEN") })

-- Create branch
c:create_branch(42, { branch = "feat/update", ref = "main" })

-- Commit changes
c:create_commit(42, {
    branch = "feat/update",
    commit_message = "Update dependencies",
    actions = {
        { action = "update", file_path = "package.json", content = '{"version": "2.0.0"}' },
    },
})

-- Open MR
local mr = c:create_merge_request(42, {
    source_branch = "feat/update",
    target_branch = "main",
    title = "Update dependencies to v2.0",
})

-- Approve and merge
c:approve_merge_request(42, mr.iid)
c:merge(42, mr.iid, { squash = true, should_remove_source_branch = true })
```
