## assay.gitlab

GitLab REST API v4 client. Projects, repositories, commits, merge requests, pipelines, jobs, issues,
releases, groups, container registry, webhooks, environments, and deploy tokens.

Supports both private access token (`PRIVATE-TOKEN` header) and OAuth2 bearer authentication.

```lua
local gitlab = require("assay.gitlab")
local c = gitlab.client("https://gitlab.example.com", { token = "glpat-xxxx" })
```

### c.projects

- `c.projects:list(opts?)` -> `[project]` — List projects. Options: `search`, `order_by`, `sort`,
  `per_page`, `page`
- `c.projects:get(id)` -> `project|nil` — Get project by numeric ID or `"namespace/name"` path

### c.files

- `c.files:get(project, path, opts?)` -> `table|nil` — Get file metadata (base64-encoded content).
  Options: `ref` (default: `"main"`)
- `c.files:raw(project, path, opts?)` -> `string|nil` — Get raw file content as string. Options:
  `ref` (default: `"main"`)
- `c.files:create(project, path, opts)` -> `table` — Create file. Options: `branch`, `content`,
  `commit_message`
- `c.files:update(project, path, opts)` -> `table` — Update file. Options: `branch`, `content`,
  `commit_message`
- `c.files:delete(project, path, opts)` -> `nil` — Delete file. Options: `branch`, `commit_message`

### c.repository

- `c.repository:tree(project, opts?)` -> `[entry]` — List repository tree. Options: `path`, `ref`,
  `recursive`, `per_page`
- `c.repository:compare(project, from, to)` -> `{commits, diffs}` — Compare branches, tags, or
  commits

### c.commits

- `c.commits:list(project, opts?)` -> `[commit]` — List commits. Options: `ref_name`, `since`,
  `until`, `path`, `per_page`
- `c.commits:get(project, sha)` -> `commit|nil` — Get single commit by SHA
- `c.commits:create(project, opts)` -> `commit` — Atomic multi-file commit. Options: `branch`,
  `commit_message`, `actions` (array of `{action, file_path, content}`)
- `c.commits:cherry_pick(project, sha, opts)` -> `commit` — Cherry-pick commit. Options: `branch`

### c.branches

- `c.branches:list(project, opts?)` -> `[branch]` — List branches. Options: `search`, `per_page`
- `c.branches:get(project, name)` -> `branch|nil` — Get branch by name
- `c.branches:create(project, opts)` -> `branch` — Create branch. Options: `branch`, `ref`
- `c.branches:delete(project, name)` -> `nil` — Delete branch

### c.tags

- `c.tags:list(project, opts?)` -> `[tag]` — List tags. Options: `search`, `order_by`, `sort`
- `c.tags:get(project, name)` -> `tag|nil` — Get tag by name
- `c.tags:create(project, opts)` -> `tag` — Create tag. Options: `tag_name`, `ref`, `message`
- `c.tags:delete(project, name)` -> `nil` — Delete tag

### c.merge_requests

- `c.merge_requests:list(project, opts?)` -> `[mr]` — List MRs. Options: `state`, `order_by`,
  `sort`, `labels`, `per_page`
- `c.merge_requests:get(project, iid)` -> `mr|nil` — Get MR by IID
- `c.merge_requests:create(project, opts)` -> `mr` — Create MR. Options: `source_branch`,
  `target_branch`, `title`, `description`
- `c.merge_requests:update(project, iid, opts)` -> `mr` — Update MR. Options: `title`,
  `description`, `state_event`, `labels`
- `c.merge_requests:merge(project, iid, opts?)` -> `mr` — Accept (merge) MR. Options: `squash`,
  `merge_commit_message`, `should_remove_source_branch`
- `c.merge_requests:approve(project, iid)` -> `table` — Approve MR
- `c.merge_requests:changes(project, iid)` -> `mr` — Get MR with diff changes
- `c.merge_requests:notes(project, iid, opts?)` -> `[note]` — List MR comments
- `c.merge_requests:create_note(project, iid, body)` -> `note` — Add comment to MR

### c.pipelines

- `c.pipelines:list(project, opts?)` -> `[pipeline]` — List pipelines. Options: `ref`, `status`,
  `per_page`
- `c.pipelines:get(project, id)` -> `pipeline|nil` — Get pipeline by ID
- `c.pipelines:create(project, opts)` -> `pipeline` — Trigger pipeline. Options: `ref`, `variables`
- `c.pipelines:cancel(project, id)` -> `pipeline` — Cancel running pipeline
- `c.pipelines:retry(project, id)` -> `pipeline` — Retry failed pipeline
- `c.pipelines:delete(project, id)` -> `nil` — Delete pipeline
- `c.pipelines:jobs(project, pipeline_id, opts?)` -> `[job]` — List jobs for a pipeline

### c.jobs

- `c.jobs:list(project, opts?)` -> `[job]` — List all project jobs. Options: `scope` (array of
  statuses)
- `c.jobs:get(project, id)` -> `job|nil` — Get job by ID
- `c.jobs:retry(project, id)` -> `job` — Retry a job
- `c.jobs:cancel(project, id)` -> `job` — Cancel a job
- `c.jobs:log(project, id)` -> `string|nil` — Get job trace/log output as raw text

### c.releases

- `c.releases:list(project, opts?)` -> `[release]` — List releases. Options: `per_page`
- `c.releases:get(project, tag_name)` -> `release|nil` — Get release by tag name
- `c.releases:create(project, opts)` -> `release` — Create release. Options: `tag_name`, `name`,
  `description`
- `c.releases:update(project, tag_name, opts)` -> `release` — Update release
- `c.releases:delete(project, tag_name)` -> `nil` — Delete release

### c.issues

- `c.issues:list(project, opts?)` -> `[issue]` — List issues. Pass `nil` as project for global
  issues. Options: `state`, `labels`, `search`, `per_page`
- `c.issues:get(project, iid)` -> `issue|nil` — Get issue by IID
- `c.issues:create(project, opts)` -> `issue` — Create issue. Options: `title`, `description`,
  `labels`, `assignee_ids`
- `c.issues:update(project, iid, opts)` -> `issue` — Update issue. Options: `title`, `description`,
  `state_event`, `labels`
- `c.issues:notes(project, iid, opts?)` -> `[note]` — List issue comments
- `c.issues:create_note(project, iid, body)` -> `note` — Add comment to issue

### c.groups

- `c.groups:list(opts?)` -> `[group]` — List groups. Options: `search`, `per_page`
- `c.groups:get(id)` -> `group|nil` — Get group by numeric ID or `"path"` name
- `c.groups:projects(id, opts?)` -> `[project]` — List projects in a group

### c.registry

- `c.registry:repositories(project)` -> `[repo]` — List container registry repositories
- `c.registry:tags(project, repo_id)` -> `[tag]` — List tags for a registry repository
- `c.registry:tag(project, repo_id, tag_name)` -> `tag|nil` — Get single registry tag (with digest,
  size)
- `c.registry:delete_tag(project, repo_id, tag_name)` -> `nil` — Delete a registry tag

### c.hooks

- `c.hooks:list(project)` -> `[hook]` — List project hooks
- `c.hooks:get(project, id)` -> `hook|nil` — Get hook by ID
- `c.hooks:create(project, opts)` -> `hook` — Create hook. Options: `url`, `push_events`,
  `merge_requests_events`, etc.
- `c.hooks:update(project, id, opts)` -> `hook` — Update hook
- `c.hooks:delete(project, id)` -> `nil` — Delete hook

### c.users

- `c.users:current()` -> `user` — Get authenticated user
- `c.users:list(opts?)` -> `[user]` — Search users. Options: `username`, `search`, `per_page`

### c.environments

- `c.environments:list(project, opts?)` -> `[env]` — List environments. Options: `search`
- `c.environments:get(project, id)` -> `env|nil` — Get environment by ID

### c.deploy_tokens

- `c.deploy_tokens:list(project)` -> `[token]` — List deploy tokens
- `c.deploy_tokens:create(project, opts)` -> `token` — Create token. Options: `name`, `scopes`,
  `expires_at`
- `c.deploy_tokens:delete(project, id)` -> `nil` — Delete token

### Example: Atomic Multi-File Commit

```lua
local gitlab = require("assay.gitlab")
local c = gitlab.client("https://gitlab.example.com", { token = env.get("GITLAB_TOKEN") })

local result = c.commits:create(42, {
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
c.branches:create(42, { branch = "feat/update", ref = "main" })

-- Commit changes
c.commits:create(42, {
    branch = "feat/update",
    commit_message = "Update dependencies",
    actions = {
        { action = "update", file_path = "package.json", content = '{"version": "2.0.0"}' },
    },
})

-- Open MR
local mr = c.merge_requests:create(42, {
    source_branch = "feat/update",
    target_branch = "main",
    title = "Update dependencies to v2.0",
})

-- Approve and merge
c.merge_requests:approve(42, mr.iid)
c.merge_requests:merge(42, mr.iid, { squash = true, should_remove_source_branch = true })
```
