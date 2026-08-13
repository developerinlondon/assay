---
category: AI Agents & Workflow
tagline: ExcaliDash REST API — Excalidraw drawings, collections, version history, and sharing
---

## assay.excalidash

ExcaliDash client for a self-hosted dashboard: create and edit Excalidraw drawings, organise them
into collections, walk their version history, and share them with users or by link.

The API is plain REST, but the credential decides how much of it you can reach — that is the one
thing worth understanding before writing a script against it.

### Two credentials, two reaches

ExcaliDash authenticates a script one of two ways, and they are not interchangeable.

```text
API key   Authorization: Bearer exd_...     no CSRF, four routes only
session   Authorization: Bearer <jwt>       every route, CSRF handshake on writes
```

An **API key** (`exd_…`, made in the dashboard under Settings) is the credential built for
automation. It carries scopes — `drawings:read`, `drawings:write`, `collections:read`,
`collections:write` — and it is exempt from CSRF, but only on requests that carry no `Origin` and no
`Referer` header. That exemption is how the server tells a script from a browser.

The catch is that the server's scope gate recognises only four route shapes:

| Route                                                   | API key | Session |
| ------------------------------------------------------- | ------- | ------- |
| `GET`/`POST /drawings`                                  | yes     | yes     |
| `GET`/`PUT`/`DELETE /drawings/:id`                      | yes     | yes     |
| `GET`/`POST /collections`                               | yes     | yes     |
| `PUT`/`DELETE /collections/:id`                         | yes     | yes     |
| `/drawings/:id/history/*`                               | no      | yes     |
| `/drawings/:id/sharing`, `/permissions`, `/link-shares` | no      | yes     |
| `/drawings/:id/duplicate`                               | no      | yes     |
| `/drawings/shared`                                      | no      | yes     |
| `/collections/:id/shares/*`                             | no      | yes     |

Anything deeper is refused before the handler runs — 403 on the `requireAuth` routes and a bare 401
on the `optionalAuth` ones, neither of which says why. The module refuses those calls itself, naming
the credential you are missing:

```text
excalidash: version history is not reachable with an API key; pass token=
(a session access token) or set EXCALIDASH_TOKEN
```

A **session token** is the JWT the browser holds after login. It reaches everything, but unsafe
methods then need CSRF: the module fetches `/csrf-token` once on first write, keeps the token and
the `excalidash-csrf-client` cookie it is bound to, and sends both on every write after that. Reads
never pay for the handshake.

Hold both and the module picks per route — the API key wherever it works, the session for the rest.

### Client

```lua
local excalidash = require("assay.excalidash")
local c = excalidash.client({
  api_key = env.get("EXCALIDASH_API_KEY"),
  base_url = "https://draw.example.com",
})
```

`excalidash.client(opts)` accepts:

- `api_key` — an `exd_…` key. Falls back to `EXCALIDASH_API_KEY`.
- `token` — a session access token. Falls back to `EXCALIDASH_TOKEN`.
- `base_url` — the dashboard origin. Falls back to `EXCALIDASH_URL`. Required; a blank one errors at
  construction rather than on first call.
- `api_path` — the prefix the backend sits behind, `/api` by default, which is where the dashboard's
  own nginx proxies it. Falls back to `EXCALIDASH_API_PATH`. Talking to the backend container
  directly needs `api_path = ""`.

A wrong `api_path` is worth getting right: the dashboard answers any unknown path with the SPA's
HTML and a 200, so a read would otherwise report an empty dashboard rather than a mistake. The
module refuses a non-JSON body and says which setting to check.

### Drawings

- `c.drawings:list(opts?)` -> `{drawings, totalCount}` — one page of summaries. `opts` takes
  `search`, `collection_id`, `include_data`, `include_preview`, `limit`, `offset`, `sort_field`
  (`name`|`createdAt`|`updatedAt`) and `sort_direction`. Summaries carry no scene unless
  `include_data` is set, which is what keeps listing a large dashboard cheap. A page caps at 200.
- `c.drawings:get(id)` -> `drawing`|nil — the scene, plus your `accessLevel`
- `c.drawings:create(spec)` -> `drawing` — `spec` takes `name`, `collection_id`, `elements`,
  `app_state`, `files`, `preview`
- `c.drawings:update(id, patch)` -> `drawing` — only the keys you set are written
- `c.drawings:delete(id)` -> `true` — permanent; see `excalidash.trash` for the reversible move
- `c.drawings:duplicate(id)` -> `drawing` — copies as `<name> (Copy)` _(session)_
- `c.drawings:shared(opts?)` -> `{drawings, totalCount}` — drawings others shared with you, each
  with an `accessLevel` _(session)_

Scene writes are versioned. Passing the `version` you read makes the write conditional, and a
drawing that moved on since then is refused rather than clobbered:

```lua
local d = c.drawings:get(id)
local ok, err = pcall(function()
  return c.drawings:update(id, { elements = edited, version = d.version })
end)
-- err names VERSION_CONFLICT; re-read and merge
```

Every scene write snapshots the previous state first, which is where version history comes from.

### Collections

Collections are flat, owner-scoped folders. Trash is one of them, reported as the id `trash`
whatever it is called internally.

- `c.collections:list()` -> `[collection]` — a bare array: owned collections first, then ones shared
  with you (`isOwner = false`, `sharedRole` set)
- `c.collections:create(name)` / `:rename(id, name)` / `:delete(id)`
- `c.collections:shares(id)` -> `[share]` _(session)_
- `c.collections:share(id, identifier, role)` -> `share` — `identifier` matches email, username,
  then display name; `role` is `view` or `edit` _(session)_
- `c.collections:set_share_role(id, user_id, role)` / `:unshare(id, user_id)` _(session)_
- `c.collections:resolve_users(id, q)` -> `[user]` _(session)_

Deleting a collection does not delete its drawings; they are moved out to no collection at all.

### Version history

All session-only. Snapshots are kept for two days and swept hourly, so history is a short window
rather than an archive.

- `c.history:list(drawing_id, opts?)` -> `{snapshots, totalCount}` — metadata only, newest first
- `c.history:get(drawing_id, snapshot_id)` -> `snapshot` — with its scene
- `c.history:restore(drawing_id, snapshot_id, version)` -> `drawing`

Restoring snapshots the current state first, so a restore is itself reversible. `version` is the
drawing's current version and guards the write exactly as a scene update does; servers from 0.6.0 on
require it and answer 400 without one.

### Sharing

All session-only.

- `c.sharing:get(drawing_id)` -> `{permissions, linkShares}`
- `c.sharing:grant(drawing_id, user_id, permission)` -> `permission` — upserts, so re-granting
  changes the level
- `c.sharing:revoke(drawing_id, permission_id)` — takes the permission row's id, not the user's
- `c.sharing:create_link(drawing_id, spec?)` -> `share`
- `c.sharing:revoke_link(drawing_id, share_id)`
- `c.sharing:resolve_users(drawing_id, q)` -> `[user]` — needs three characters, and is scoped to a
  drawing you own

Only one link share is active per drawing: creating another revokes the one before it.
`spec.expires_at` is an ISO timestamp at least a minute out, or `false` for no expiry at all — which
the server honours for `view` and overrides with its own ceiling for `edit`. Omitting it entirely
gets the server's default TTL, which is a different thing from `false`.

### Helpers

- `excalidash.all_drawings(c, opts?)` -> `[drawing]` — walks every page to the end
- `excalidash.find_drawing_by_name(c, name, opts?)` -> `drawing`|nil — exact match. The server's
  `search` filter is a substring match, so it narrows the scan but cannot stand in for the
  comparison
- `excalidash.ensure_drawing(c, spec)` -> `drawing` — creates unless the name is taken
- `excalidash.collections(c)` -> `[collection]` — owned collections, Trash excluded
- `excalidash.resolve_collection(c, name?)` -> `collection` — the one matching a name, else the only
  one; refuses to guess between several
- `excalidash.ensure_collection(c, name)` -> `collection`
- `excalidash.trash(c, drawing_id)` -> `drawing` — moves to Trash instead of deleting
- `excalidash.undo_last_change(c, drawing_id)` -> `drawing`|nil — restores the newest snapshot,
  reading the drawing first for the version the guarded restore needs

### Example

```lua
#!/usr/bin/env assay
local excalidash = require("assay.excalidash")

local c = excalidash.client({ base_url = "https://draw.example.com" })

local folder = excalidash.ensure_collection(c, "Architecture")
local d = excalidash.ensure_drawing(c, {
  name = "Ingress path",
  collection_id = folder.id,
  elements = scene.elements,
  app_state = scene.appState,
})

log.info("drawing " .. d.id .. " is at v" .. d.version)

for _, old in ipairs(excalidash.all_drawings(c, { search = "draft" })) do
  excalidash.trash(c, old.id)
end
```
