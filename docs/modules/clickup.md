---
category: AI Agents & Workflow
tagline: ClickUp REST API — tasks, lists, spaces, goals, custom fields, time tracking, and Docs
---

## assay.clickup

ClickUp client covering sprint execution (tasks, lists, statuses), quarterly targets (Goals), KPI
carriers (custom fields), time tracking, comments, and Docs. Everything is API v2 except Docs, which
ClickUp only ships on v3.

### Client

```lua
local clickup = require("assay.clickup")
local c = clickup.client({ token = env.get("CLICKUP_TOKEN") })
```

`clickup.client(opts)` accepts:

- `token` — a personal `pk_` token or an OAuth access token. Falls back to `CLICKUP_TOKEN`.
- `base_url` — defaults to `https://api.clickup.com/api`. Override for a proxy or a test double.

The token travels in `Authorization` **without** a `Bearer` prefix; ClickUp rejects the prefix on
personal tokens.

### Workspaces, spaces, folders, lists

API v2 calls a workspace a _team_, and the module keeps the API's vocabulary.

- `c.teams:list()` -> `[team]`
- `c.spaces:list(team_id, opts?)` -> `[space]`
- `c.spaces:get(space_id)` -> `space`|nil
- `c.spaces:create(team_id, space)` -> `space`
- `c.spaces:tags(space_id)` -> `[tag]`
- `c.folders:list(space_id, opts?)` -> `[folder]` — Sprint Folders live here
- `c.folders:get(folder_id)` -> `folder`|nil
- `c.folders:create(space_id, folder)` -> `folder`
- `c.folders:delete(folder_id)` -> `true`
- `c.lists:list(folder_id, opts?)` -> `[list]` — one List per sprint
- `c.lists:folderless(space_id, opts?)` -> `[list]`
- `c.lists:get(list_id)` -> `list`|nil — `.statuses` carries the space's real status names
- `c.lists:create(folder_id, list)` -> `list`
- `c.lists:create_folderless(space_id, list)` -> `list`
- `c.lists:update(list_id, patch)` -> `list`
- `c.lists:delete(list_id)` -> `true`
- `c.lists:members(list_id)` -> `[member]`

### Tasks

- `c.tasks:page(list_id, opts?)` -> `{tasks, last_page}` — one page, 100 tasks maximum
- `c.tasks:list(list_id, opts?)` -> `[task]` — first page only
- `c.tasks:get(task_id, opts?)` -> `task`|nil
- `c.tasks:create(list_id, task)` -> `task`
- `c.tasks:update(task_id, patch)` -> `task`
- `c.tasks:delete(task_id)` -> `true`
- `c.tasks:filtered(team_id, opts?)` -> `{tasks, last_page}` — workspace-wide query
- `c.tasks:members(task_id)` -> `[member]`

Task pagination is zero-based `page`, and the envelope ends the walk with `last_page = true`.
List-valued filters (`statuses`, `assignees`, `tags`) are passed as Lua arrays and encode as
repeated `key[]=` parameters.

```lua
local open = c.tasks:list(list_id, { statuses = { "to do", "in progress" }, page = 0 })
```

### Comments

- `c.comments:list(task_id, opts?)` -> `[comment]`
- `c.comments:create(task_id, body, extra?)` -> `comment` — `body` is a rich builder, a string, or a
  full payload table; `extra` merges in request options such as `notify_all`
- `c.comments:update(comment_id, patch)` -> `comment`
- `c.comments:delete(comment_id)` -> `true`

ClickUp renders comments as Quill rich text. A string goes out on `comment_text` and is displayed
**verbatim**, so markdown arrives with its asterisks and pipes intact and a plain `@Name` tags
nobody. Use `clickup.rich()` for anything beyond a one-line note.

```lua
local bharat = clickup.resolve_member(c, team.id, "nsmtech.development@gmail.com")

local body = clickup.rich()
  :bold("Docs cutover: done and live."):br()
  :mention(bharat):text(" — the revision is yours."):br()
  :text("Live at "):link("docs.agentkit.sbs", "https://docs.agentkit.sbs/"):bullet()
  :text("Source: "):code("docs/hextra/content/**"):bullet()
  :text("Review it against the acceptance criteria"):number()

c.comments:create(task_id, body, { notify_all = true })
```

Builder methods, each returning the builder so calls chain:

- Inline runs — `:text(s)`, `:bold(s)`, `:italic(s)`, `:code(s)`, `:link(s, url)`
- Mentions — `:mention(user)`, taking the record `resolve_member` returns or a bare user id
- Line terminators — `:br()`, `:bullet()`, `:number()`, `:heading(level?)`

The terminator formats the line it closes, because a Quill delta carries line-level attributes on
the newline op rather than on the text before it. There is **no table type** — flatten tabular
content into labelled bullets.

`clickup.resolve_member(c, team_id, needle)` -> `user` matches a username or email exactly, then
falls back to a username substring, and raises rather than guess when several members match. A
mention notifies on the numeric id; the `@Name` text is only a label, so a name that was never
resolved against the roster silently reaches no one.

`clickup.comment_payload(body)` -> `table` exposes the same normalisation for callers assembling a
request by hand.

### Goals

Goals carry quarterly targets. Listing is scoped to a workspace; every other operation addresses the
goal directly.

- `c.goals:list(team_id, opts?)` -> `[goal]`
- `c.goals:get(goal_id)` -> `goal`|nil
- `c.goals:create(team_id, goal)` -> `goal`
- `c.goals:update(goal_id, patch)` -> `goal`
- `c.goals:delete(goal_id)` -> `true`

### Custom fields

Custom fields are the queryable home for KPIs.

- `c.fields:list(list_id)` -> `[field]`
- `c.fields:space(space_id)` -> `[field]`
- `c.fields:team(team_id)` -> `[field]`
- `c.fields:set(task_id, field_id, value, opts?)` -> `true`
- `c.fields:remove(task_id, field_id)` -> `true`

### Time tracking

- `c.time:entries(team_id, opts?)` -> `[entry]`
- `c.time:running(team_id, opts?)` -> `entry`|nil
- `c.time:create(team_id, entry)` -> `entry`
- `c.time:stop(team_id)` -> `entry` — stops the authenticated user's running timer

### Docs (API v3)

Docs are the only resource on v3, which addresses the workspace explicitly rather than calling it a
team.

- `c.docs:search(workspace_id, opts?)` -> `{docs, ...}`
- `c.docs:get(workspace_id, doc_id)` -> `doc`|nil
- `c.docs:create(workspace_id, doc)` -> `doc`
- `c.docs:pages(workspace_id, doc_id, opts?)` -> `[page]`
- `c.docs:create_page(workspace_id, doc_id, page)` -> `page`
- `c.docs:edit_page(workspace_id, doc_id, page_id, patch)` -> `page`

### Helpers

- `clickup.all_tasks(c, list_id, opts?)` -> `[task]` — follows every page until `last_page`.
  `opts.max_pages` (default 100) bounds the walk; any other key is forwarded as a filter.
- `clickup.find_task_by_name(c, list_id, name, opts?)` -> `task`|nil — exact-name match.
- `clickup.ensure_task(c, list_id, spec, opts?)` -> `task` — returns the existing task when the name
  is already present, so repeated runs do not fan out duplicates. Requires `spec.name`.
- `clickup.resolve_team(c, name?)` -> `team` — the only visible workspace, or the one matching
  `name`. Errors when the token sees several and no name is given.

```lua
local team = clickup.resolve_team(c)
local task = clickup.ensure_task(c, list_id, { name = "Fix token refresh race" })
c.tasks:update(task.id, { status = "in progress" })
c.comments:create(task.id, "Root cause: … Fix: …")
```

### Rate limits

The REST API allows 100 requests per minute per token on the Free, Unlimited, and Business plans.
ClickUp's hosted MCP server is a separate, far tighter budget — 50 calls per 24 hours on Free and
300 on paid tiers — so automation belongs on the REST API this module speaks.
