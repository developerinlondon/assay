---
category: AI Agents & Workflow
tagline: Huly transactor REST API — document queries, transactions, fulltext search, and tracker helpers
---

## assay.huly

Huly client for a self-hosted or cloud workspace: read any document class, write through
transactions, search fulltext, and drive the tracker (projects, issues, milestones, components).

Huly has no per-resource REST endpoints. Everything is one of two calls — a class-parameterised
query, or a transaction document — so the module is shaped around those rather than around nouns.

```text
read   GET  /find-all/{workspace}?class=&query=&options=  -> {dataType, total, value:[...]}
write  POST /tx/{workspace}   {_class: core:class:Tx*Doc, objectClass, objectSpace, ...}
```

### Client

```lua
local huly = require("assay.huly")
local c = huly.client({
  token = env.get("HULY_TOKEN"),
  workspace = "b07d7630-3393-44e8-803d-4366baaeb80b",
  base_url = "https://huly.example.com",
})
```

`huly.client(opts)` accepts:

- `token` — a workspace JWT. Falls back to `HULY_TOKEN`. Travels as `Authorization: Bearer`.
- `workspace` — the workspace **uuid**, not its slug. Falls back to `HULY_WORKSPACE`. Every path
  ends in it, so a blank one errors on first use rather than building `/find-all/`.
- `base_url` — falls back to `HULY_URL`, then `https://huly.app`. `/_transactor` is appended unless
  already present.
- `account` — the social id to stamp on transactions. Falls back to `HULY_ACCOUNT`, then to the
  token's `primarySocialId`, fetched on first write.

Requests ask for `Accept-Encoding: identity`. Huly's own client asks for `snappy, gzip`; assay can
decode neither, and a compressed response arrives as bytes the JSON parser rejects.

### Reading

- `c:account()` -> `account` — the identity behind the token, with its social ids
- `c:model(full?)` -> `[tx]` — every model transaction, which is where class and attribute
  definitions live. Useful for discovering class ids on an unfamiliar deployment.
- `c:find_all(class, query?, options?)` -> `[doc]`
- `c:find_one(class, query?, options?)` -> `doc`|nil
- `c:count(class, query?)` -> `number`
- `c:search(query, opts?)` -> `{docs, total}` — fulltext; `opts.classes`, `opts.spaces`,
  `opts.limit`

`query` is a Mongo-flavoured document filter (`{ space = "p1", priority = { ["$in"] = {1, 2} } }`).
`options` carries `limit`, `sort`, `projection`, `total`, `lookup`.

Two server behaviours are papered over, matching what Huly's own client does:

- Results arrive wrapped as `{dataType = "TotalArray", total, lookupMap, value = [...]}`; the module
  returns the rows.
- The transactor omits `_class` on class-scoped reads, and strips attributes the query already pins
  to a scalar — query on `identifier` and the returned document has no `identifier`. Both are put
  back, so `doc.identifier` reads as expected. Values the server did send are never overwritten, and
  operator terms (`{["$in"] = ...}`) are not scalars and are not copied.

`total` is `-1` unless the request asks for it, which is why `count` exists.

### Writing

- `c:tx(tx)` -> `result` — post a raw transaction
- `c:create_doc(class, space, attrs, id?)` -> `id` — returns the new document's id; the transactor
  answers a bare `[]`
- `c:update_doc(class, space, id, ops, retrieve?)` -> `result` — `ops` assigns plain fields and
  honours `$inc` / `$push` / `$pull`; with `retrieve` the answer is `{object = <new doc>}`
- `c:remove_doc(class, space, id)` -> `true`
- `c:ensure_person(social_type, social_value, first, last)` -> `person` — idempotent upsert
- `huly.new_id()` -> a 24-hex id in Huly's format (unix seconds, randomness, counter). The
  transactor's `isId` accepts exactly 24 hex characters, so the widths are not free.

### Tracker helpers

- `huly.projects(c)` -> `[project]`
- `huly.resolve_project(c, key?)` -> `project` — matched on `identifier` first, then `name`, else
  the only one. Errors rather than guessing between several.
- `huly.statuses(c)` -> `[status]` — the issue statuses (`Backlog`, `Todo`, `In Progress`, `Done`,
  `Canceled` on a stock tracker)
- `huly.issues(c, project, opts?)` -> `[issue]` — `opts` merges into the find options
- `huly.find_issue_by_title(c, project, title)` -> `issue`|nil
- `huly.create_issue(c, project, spec)` -> `issue`
- `huly.ensure_issue(c, project, spec)` -> `issue` — creates unless the title is taken
- `huly.set_issue_status(c, issue, status)` -> `true`
- `huly.delete_issue(c, issue)` -> `true`
- `huly.components(c, project)` / `huly.create_component(c, project, spec)`
- `huly.milestones(c, project)` / `huly.create_milestone(c, project, spec)`

Issue numbering lives on the project, not the issue. `create_issue` atomically increments the
project's `sequence` and derives both `number` and the `PREFIX-N` identifier from the result; an
issue written without them is invisible in the UI. It also fills the fields the transactor requires
but the caller rarely cares about — `rank`, `kind`, and the `attachedTo` / `attachedToClass` /
`collection` triple that marks a top-level issue.

Class ids are exposed as constants (`huly.ISSUE_CLASS`, `huly.PROJECT_CLASS`, `huly.STATUS_CLASS`,
`huly.MILESTONE_CLASS`, `huly.COMPONENT_CLASS`, `huly.NO_PARENT`, `huly.TX_SPACE`).

### Example — file a triage issue and move it

```lua
local huly = require("assay.huly")
local c = huly.client({ workspace = env.get("HULY_WORKSPACE") })

local project = huly.resolve_project(c, "TSK")
local issue = huly.ensure_issue(c, project, {
  title = "Nightly backup verification failed",
  description = "rustic check reported a damaged pack",
  priority = 1,
})

huly.set_issue_status(c, issue, "tracker:status:InProgress")
log.info(issue.identifier .. " is now in progress")
```

### Example — read anything by class

```lua
local huly = require("assay.huly")
local c = huly.client({})

-- Discover what a deployment actually has, then query it.
for _, tx in ipairs(c:model()) do
  if tx.objectClass == "core:class:Class" then log.info(tx.objectId) end
end

local overdue = c:find_all("tracker:class:Issue", {
  space = "tracker:project:DefaultProject",
  dueDate = { ["$lt"] = os.time() * 1000 },
}, { sort = { dueDate = 1 }, limit = 20 })
log.info(#overdue .. " overdue issues")
```
