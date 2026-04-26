---
category: AI Agents & Workflow
---

## assay.github

GitHub REST API client. PRs, issues, actions, repositories, GraphQL, releases. No `gh` CLI
dependency. Authentication via `GITHUB_TOKEN` / `GH_TOKEN` env vars or explicit `token` opt.

### Client

```lua
local github = require("assay.github")
local c = github.client({ token = "...", base_url = "https://api.github.com" })
```

### Pull Requests (`c.pulls`)

- `c.pulls:get(repo, number)` → `pr` — Get pull request details.
- `c.pulls:list(repo, opts?)` → `[pr]` — List pull requests. `opts`:
  `{state, sort, direction, per_page}`.
- `c.pulls:reviews(repo, number)` → `[review]` — List PR reviews.
- `c.pulls:merge(repo, number, opts?)` → `result` — Merge a pull request. `opts`:
  `{merge_method, commit_title, commit_message}`.

### Issues (`c.issues`)

- `c.issues:list(repo, opts?)` → `[issue]` — List issues. `opts`:
  `{state, labels, sort, direction, per_page}`.
- `c.issues:get(repo, number)` → `issue` — Get issue details.
- `c.issues:create(repo, title, body, opts?)` → `issue` — Create an issue. `opts`:
  `{labels, assignees, milestone}`.
- `c.issues:create_note(repo, number, body)` → `comment` — Add a comment.

### Repositories (`c.repos`)

- `c.repos:get(repo)` → `repository` — Get repository details.

### Workflow Runs (`c.runs`)

- `c.runs:list(repo, opts?)` → `{workflow_runs}` — List workflow runs.
- `c.runs:get(repo, run_id)` → `run` — Get workflow run details.

### GraphQL

- `c:graphql(query, variables?)` → `data` — Execute a GraphQL query.

### Releases (module-level)

These are top-level functions on the module — no client needed. Token falls back to `GITHUB_TOKEN` /
`GH_TOKEN` env vars. Pass `opts.base_url` to point at GitHub Enterprise.

- `github.latest_release(owner, repo, opts?)` → `release` —
  `GET /repos/{owner}/{repo}/releases/latest`. The returned table has all the JSON fields plus
  `version` (the `tag_name` with a leading `v` stripped).
- `github.find_asset(release, name_pattern)` → `asset | nil` — Lua-pattern match on `asset.name`.
  Returns the first hit.
- `github.fetch_asset_text(asset)` → `string` — Download `asset.browser_download_url` and return the
  body.
- `github.fetch_asset_bytes(asset)` → `string` — Alias of `fetch_asset_text` (Lua strings are
  bytes).
- `github.release_checksum(release, opts)` → `hex` — Find a sibling `<asset>.sha256` (or other
  `opts.digest`) in the release, fetch it, and return the lowercase hex digest. `opts`:
  `{asset_pattern, digest = "sha256"}`.

### Example

```lua
local github = require("assay.github")

local rel    = github.latest_release("rustic-rs", "rustic")
print(rel.version)  -- e.g. "0.10.0"

local sha256 = github.release_checksum(rel, {
  asset_pattern = "x86_64%-unknown%-linux%-musl%.tar%.gz$",
  digest        = "sha256",
})
print(sha256)
```
