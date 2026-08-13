--- @module assay.excalidash
--- @description ExcaliDash REST API — Excalidraw drawings, collections, version-history snapshots, and user/link sharing on a self-hosted ExcaliDash dashboard.
--- @keywords excalidash, excalidraw, drawing, diagram, sketch, whiteboard, canvas, scene, elements, collection, folder, trash, snapshot, version-history, restore, share, link-share, permission, api-key, self-hosted, zimengxiong
--- @quickref c.drawings:list(opts?) -> {drawings, totalCount} | One page of drawing summaries
--- @quickref c.drawings:get(id) -> drawing|nil | A drawing with its scene
--- @quickref c.drawings:create(spec) -> drawing | Create a drawing
--- @quickref c.drawings:update(id, patch) -> drawing | Update name, scene, or collection
--- @quickref c.drawings:delete(id) -> true | Delete a drawing for good
--- @quickref c.drawings:duplicate(id) -> drawing | Copy a drawing as "<name> (Copy)"
--- @quickref c.drawings:shared(opts?) -> {drawings, totalCount} | Drawings other people shared with you
--- @quickref c.collections:list() -> [collection] | Collections you own or were given
--- @quickref c.collections:create(name) -> collection | Create a collection
--- @quickref c.collections:rename(id, name) -> collection | Rename a collection
--- @quickref c.collections:delete(id) -> true | Delete a collection, orphaning its drawings
--- @quickref c.collections:shares(id) -> [share] | Who a collection is shared with
--- @quickref c.collections:share(id, identifier, role) -> share | Share a collection by email or username
--- @quickref c.collections:set_share_role(id, user_id, role) -> true | Change a collection share's role
--- @quickref c.collections:unshare(id, user_id) -> true | Drop a collection share
--- @quickref c.collections:resolve_users(id, q) -> [user] | Search users to share a collection with
--- @quickref c.history:list(drawing_id, opts?) -> {snapshots, totalCount} | Snapshot metadata, newest first
--- @quickref c.history:get(drawing_id, snapshot_id) -> snapshot | One snapshot with its scene
--- @quickref c.history:restore(drawing_id, snapshot_id, version) -> drawing | Roll a drawing back to a snapshot
--- @quickref c.sharing:get(drawing_id) -> {permissions, linkShares} | A drawing's whole sharing state
--- @quickref c.sharing:grant(drawing_id, user_id, permission) -> permission | Share a drawing with a user
--- @quickref c.sharing:revoke(drawing_id, permission_id) -> true | Drop a user share
--- @quickref c.sharing:create_link(drawing_id, spec?) -> share | Open an "anyone with the link" share
--- @quickref c.sharing:revoke_link(drawing_id, share_id) -> true | Revoke a link share
--- @quickref c.sharing:resolve_users(drawing_id, q) -> [user] | Search users to share a drawing with
--- @quickref M.all_drawings(c, opts?) -> [drawing] | Walk every page to the end
--- @quickref M.find_drawing_by_name(c, name, opts?) -> drawing|nil | Exact-name drawing lookup
--- @quickref M.ensure_drawing(c, spec) -> drawing | Create a drawing unless the name exists
--- @quickref M.collections(c) -> [collection] | Collections you own, Trash excluded
--- @quickref M.resolve_collection(c, name?) -> collection | The only collection, or the one matching a name
--- @quickref M.ensure_collection(c, name) -> collection | Create a collection unless the name exists
--- @quickref M.trash(c, drawing_id) -> drawing | Move a drawing to Trash instead of deleting it
--- @quickref M.undo_last_change(c, drawing_id) -> drawing|nil | Restore the snapshot before the last scene write

local M = {}

-- The public alias for a user's trash. On the wire the collection is
-- `trash:<userId>`, but the server maps that id in both directions and only
-- ever accepts and reports the bare word.
M.TRASH = "trash"

M.PERMISSIONS = { view = "view", edit = "edit" }

local CSRF_COOKIE = "excalidash-csrf-client"
-- `-` is a repetition operator in a Lua pattern, so the cookie name has to be
-- escaped before it can be matched against a Set-Cookie header.
local CSRF_COOKIE_PATTERN = CSRF_COOKIE:gsub("%-", "%%-") .. "=([^;]+)"

-- assay's JSON encoder has no null: a Lua table simply has no key for one. The
-- link-share route distinguishes an absent `expiresAt` (use the default TTL)
-- from an explicit null (no expiry at all), so that one body is encoded through
-- a marker and patched to a literal null.
local NULL_MARK = "__excalidash_null_5f3a91__"

local function encode_with_nulls(body)
  local encoded = json.encode(body)
  return (encoded:gsub('"' .. NULL_MARK .. '"', "null"))
end

-- Routes an API key cannot reach, and what they are called in an error. The
-- server's scope gate only recognises /drawings, /drawings/:id, /collections
-- and /collections/:id; everything deeper is refused before the handler runs,
-- so the module says which credential is missing rather than relaying a bare
-- 401 or 403.
local function session_only(label)
  return "excalidash: " .. label .. " is not reachable with an API key; pass token= "
    .. "(a session access token) or set EXCALIDASH_TOKEN"
end

function M.client(opts)
  opts = opts or {}
  local api_key = opts.api_key or env.get("EXCALIDASH_API_KEY")
  local token = opts.token or env.get("EXCALIDASH_TOKEN")
  local base_url = (opts.base_url or env.get("EXCALIDASH_URL") or ""):gsub("/+$", "")
  -- A browser talks to the dashboard origin, where nginx proxies /api/ to the
  -- backend with the prefix stripped. Point base_url at the backend itself and
  -- pass api_path = "" instead.
  local api_path = opts.api_path or env.get("EXCALIDASH_API_PATH") or "/api"
  api_path = api_path:gsub("/+$", "")

  if base_url == "" then
    error("excalidash: no base url; pass base_url= or set EXCALIDASH_URL")
  end

  local csrf = nil

  local function urlencode(str)
    return tostring(str):gsub("([^%w%-%.%_%~])", function(ch)
      return string.format("%%%02X", string.byte(ch))
    end)
  end

  local function build_query(params)
    if not params then return "" end
    local parts = {}
    for k, v in pairs(params) do
      if v ~= nil then parts[#parts + 1] = urlencode(k) .. "=" .. urlencode(v) end
    end
    table.sort(parts)
    return #parts > 0 and "?" .. table.concat(parts, "&") or ""
  end

  local function url(route, query_params)
    return base_url .. api_path .. route .. build_query(query_params)
  end

  local function decode(resp)
    if resp.body and resp.body ~= "" then
      local ok, parsed = pcall(json.parse, resp.body)
      if ok then return parsed end
    end
    return nil
  end

  --- Errors carry `{error, message}`, and sometimes a machine-readable `code`
  --- worth keeping — a stale write answers 409 `VERSION_CONFLICT`.
  local function fail(verb, route, resp)
    local detail = resp.body or ""
    local parsed = decode(resp)
    if type(parsed) == "table" then
      local parts = {}
      if parsed.error then parts[#parts + 1] = tostring(parsed.error) end
      if parsed.code then parts[#parts + 1] = "(" .. tostring(parsed.code) .. ")" end
      if parsed.message then parts[#parts + 1] = tostring(parsed.message) end
      if #parts > 0 then detail = table.concat(parts, " ") end
    end
    error("excalidash: " .. verb .. " " .. route .. " HTTP " .. resp.status .. ": " .. detail)
  end

  --- Which credential answers for a route. An API key is preferred where it
  --- works: it needs no CSRF round trip. `label` names the route in the error
  --- raised when only an API key is on hand and the route needs a session.
  local function credential(needs_session, label)
    if needs_session then
      if not token then error(session_only(label)) end
      return "session"
    end
    if api_key then return "key" end
    if token then return "session" end
    error("excalidash: no credential; pass api_key= or token=, or set "
      .. "EXCALIDASH_API_KEY / EXCALIDASH_TOKEN")
  end

  --- The CSRF handshake a session write needs. `GET /csrf-token` answers the
  --- token and the header to put it in, and sets a client cookie the token is
  --- bound to; presenting one without the other fails validation.
  local function ensure_csrf()
    if csrf then return csrf end
    local resp = http.get(url("/csrf-token"), { headers = { ["Accept"] = "application/json" } })
    if resp.status ~= 200 then fail("GET", "/csrf-token", resp) end
    local parsed = decode(resp) or {}
    local set_cookie = (resp.headers or {})["set-cookie"] or ""
    local cookie = set_cookie:match(CSRF_COOKIE_PATTERN)
    if not parsed.token or not cookie then
      error("excalidash: /csrf-token did not answer both a token and a " .. CSRF_COOKIE .. " cookie")
    end
    csrf = {
      token = parsed.token,
      header = parsed.header or "x-csrf-token",
      cookie = cookie,
    }
    return csrf
  end

  --- An API-key request must carry no Origin and no Referer: that is exactly
  --- what the server checks to tell a script from a browser, and it is what
  --- exempts the request from CSRF. assay's HTTP client sends neither.
  local function headers(mode, mutating)
    local h = { ["Content-Type"] = "application/json", ["Accept"] = "application/json" }
    if mode == "key" then
      h["Authorization"] = "Bearer " .. api_key
      return h
    end
    h["Authorization"] = "Bearer " .. token
    if mutating then
      local cs = ensure_csrf()
      h[cs.header] = cs.token
      h["Cookie"] = CSRF_COOKIE .. "=" .. cs.cookie
    end
    return h
  end

  --- A dashboard origin answers any unknown path with the SPA's HTML and a 200,
  --- so a wrong `api_path` reads as an empty result instead of a failure. Refuse
  --- a body that is not JSON rather than reporting "no drawings".
  local function decode_json_body(verb, route, resp)
    local parsed = decode(resp)
    if parsed == nil and resp.body and resp.body ~= "" then
      error("excalidash: " .. verb .. " " .. route .. " answered HTTP " .. resp.status
        .. " with a non-JSON body; is api_path (" .. (api_path == "" and "<empty>" or api_path)
        .. ") right for this host?")
    end
    return parsed
  end

  local function api_get(route, query_params, needs_session, label)
    local mode = credential(needs_session, label or route)
    local resp = http.get(url(route, query_params), { headers = headers(mode, false) })
    if resp.status == 404 then return nil end
    if resp.status ~= 200 then fail("GET", route, resp) end
    return decode_json_body("GET", route, resp)
  end

  local function api_send(verb, fn, route, payload, needs_session, label)
    local mode = credential(needs_session, label or route)
    local resp = fn(url(route), payload or {}, { headers = headers(mode, true) })
    if resp.status ~= 200 and resp.status ~= 201 and resp.status ~= 204 then
      fail(verb, route, resp)
    end
    return decode_json_body(verb, route, resp)
  end

  local function api_post(route, payload, needs_session, label)
    return api_send("POST", http.post, route, payload, needs_session, label)
  end

  local function api_put(route, payload, needs_session, label)
    return api_send("PUT", http.put, route, payload, needs_session, label)
  end

  local function api_patch(route, payload, needs_session, label)
    return api_send("PATCH", http.patch, route, payload, needs_session, label)
  end

  local function api_delete(route, needs_session, label)
    local mode = credential(needs_session, label or route)
    local resp = http.delete(url(route), { headers = headers(mode, true) })
    if resp.status ~= 200 and resp.status ~= 204 then fail("DELETE", route, resp) end
    return true
  end

  --- List filters, as the server names them. Booleans and numbers all travel as
  --- strings, and an unknown sort field silently falls back to `updatedAt`.
  local function list_params(o)
    o = o or {}
    local params = {}
    if o.search then params.search = o.search end
    if o.collection_id then params.collectionId = o.collection_id end
    if o.include_data ~= nil then params.includeData = tostring(o.include_data) end
    if o.include_preview ~= nil then params.includePreview = tostring(o.include_preview) end
    if o.limit then params.limit = tostring(o.limit) end
    if o.offset then params.offset = tostring(o.offset) end
    if o.sort_field then params.sortField = o.sort_field end
    if o.sort_direction then params.sortDirection = o.sort_direction end
    return params
  end

  --- An empty Lua table encodes as `{}`, and the create schema wants `[]` for
  --- elements. Anything already built by `json.array` passes through.
  local function elements_of(value)
    if value == nil then return json.array({}) end
    if type(value) == "table" and next(value) == nil then return json.array({}) end
    return value
  end

  local c = {}

  -- ===== Drawings =====

  c.drawings = {}

  --- One page of summaries: `{drawings, totalCount}`, plus `limit` and `offset`
  --- echoed back when they were asked for. Summaries carry no scene unless
  --- `include_data` is set, which is why listing a large dashboard stays cheap.
  function c.drawings:list(o)
    return api_get("/drawings", list_params(o))
  end

  function c.drawings:get(id)
    return api_get("/drawings/" .. urlencode(id))
  end

  --- `spec.elements` is the excalidraw element array and `spec.app_state` the
  --- editor state; both default empty. `collection_id` may be `M.TRASH`.
  function c.drawings:create(spec)
    spec = spec or {}
    return api_post("/drawings", {
      name = spec.name,
      collectionId = spec.collection_id,
      elements = elements_of(spec.elements),
      appState = spec.app_state or {},
      files = spec.files,
      preview = spec.preview,
    })
  end

  --- Only the keys present are written. Passing `version` makes the write
  --- conditional: a scene that moved on since it was read answers 409 with
  --- `VERSION_CONFLICT` and the current version, rather than clobbering it.
  --- Every scene write snapshots the previous state first.
  function c.drawings:update(id, patch)
    patch = patch or {}
    local body = {}
    if patch.name ~= nil then body.name = patch.name end
    if patch.collection_id ~= nil then body.collectionId = patch.collection_id end
    if patch.elements ~= nil then body.elements = elements_of(patch.elements) end
    if patch.app_state ~= nil then body.appState = patch.app_state end
    if patch.files ~= nil then body.files = patch.files end
    if patch.preview ~= nil then body.preview = patch.preview end
    if patch.version ~= nil then body.version = patch.version end
    return api_put("/drawings/" .. urlencode(id), body)
  end

  function c.drawings:delete(id)
    return api_delete("/drawings/" .. urlencode(id))
  end

  function c.drawings:duplicate(id)
    return api_post("/drawings/" .. urlencode(id) .. "/duplicate", {}, true, "duplicating a drawing")
  end

  --- Drawings owned by someone else and shared with you. Each carries
  --- `accessLevel`, and `collectionId` is always null: collections belong to
  --- the owner and are not exposed to a viewer.
  function c.drawings:shared(o)
    return api_get("/drawings/shared", list_params(o), true, "the shared-with-me list")
  end

  -- ===== Collections =====

  c.collections = {}

  --- Owned collections first, then ones shared with you (`isOwner = false`,
  --- `sharedRole` set). Trash is always present, reported as id `trash`.
  function c.collections:list()
    return api_get("/collections")
  end

  function c.collections:create(name)
    return api_post("/collections", { name = name })
  end

  function c.collections:rename(id, name)
    return api_put("/collections/" .. urlencode(id), { name = name })
  end

  --- Deleting a collection does not delete its drawings; they are moved out to
  --- no collection at all.
  function c.collections:delete(id)
    return api_delete("/collections/" .. urlencode(id))
  end

  function c.collections:shares(id)
    local payload = api_get("/collections/" .. urlencode(id) .. "/shares", nil, true,
      "collection sharing")
    return payload and payload.shares or {}
  end

  --- `identifier` is matched against email, username, then display name.
  function c.collections:share(id, identifier, role)
    local payload = api_post("/collections/" .. urlencode(id) .. "/shares",
      { identifier = identifier, role = role or M.PERMISSIONS.view }, true, "collection sharing")
    return payload and payload.share
  end

  function c.collections:set_share_role(id, user_id, role)
    api_patch("/collections/" .. urlencode(id) .. "/shares/" .. urlencode(user_id),
      { role = role }, true, "collection sharing")
    return true
  end

  function c.collections:unshare(id, user_id)
    return api_delete("/collections/" .. urlencode(id) .. "/shares/" .. urlencode(user_id),
      true, "collection sharing")
  end

  function c.collections:resolve_users(id, q)
    local payload = api_get("/collections/" .. urlencode(id) .. "/share-resolve", { q = q }, true,
      "collection user lookup")
    return payload and payload.users or {}
  end

  -- ===== Version history =====

  c.history = {}

  --- Snapshot metadata only — id, version, createdAt — newest first. The server
  --- keeps snapshots for two days and sweeps hourly, so history is a short
  --- window, not an archive.
  function c.history:list(drawing_id, o)
    o = o or {}
    local params = {}
    if o.limit then params.limit = tostring(o.limit) end
    if o.offset then params.offset = tostring(o.offset) end
    return api_get("/drawings/" .. urlencode(drawing_id) .. "/history", params, true,
      "version history")
  end

  function c.history:get(drawing_id, snapshot_id)
    return api_get("/drawings/" .. urlencode(drawing_id) .. "/history/" .. urlencode(snapshot_id),
      nil, true, "version history")
  end

  --- Restoring snapshots the current state first, so a restore is itself
  --- reversible, and bumps the drawing's version.
  ---
  --- `version` is the drawing's current version, and the write is guarded by it
  --- exactly as a scene update is: a drawing that moved on since the snapshot
  --- list was read answers 409 rather than discarding the newer state. Servers
  --- from 0.6.0 on require it and answer 400 without one.
  function c.history:restore(drawing_id, snapshot_id, version)
    local body = {}
    if version ~= nil then body.version = version end
    return api_post(
      "/drawings/" .. urlencode(drawing_id) .. "/history/" .. urlencode(snapshot_id) .. "/restore",
      body, true, "version history")
  end

  -- ===== Sharing =====

  c.sharing = {}

  --- `{permissions, linkShares}` — per-user grants and link policies together.
  function c.sharing:get(drawing_id)
    return api_get("/drawings/" .. urlencode(drawing_id) .. "/sharing", nil, true,
      "drawing sharing")
  end

  --- Upserts, so re-granting a user changes their permission rather than
  --- failing. Sharing with yourself is refused.
  function c.sharing:grant(drawing_id, user_id, permission)
    local payload = api_post("/drawings/" .. urlencode(drawing_id) .. "/permissions",
      { granteeUserId = user_id, permission = permission or M.PERMISSIONS.view }, true,
      "drawing sharing")
    return payload and payload.permission
  end

  --- Takes the permission row's id, not the grantee's user id.
  function c.sharing:revoke(drawing_id, permission_id)
    return api_delete(
      "/drawings/" .. urlencode(drawing_id) .. "/permissions/" .. urlencode(permission_id),
      true, "drawing sharing")
  end

  --- Only one link share is active per drawing: creating another revokes the
  --- one before it. `spec.expires_at` is an ISO timestamp at least a minute
  --- out; `false` asks for no expiry, which the server honours for `view` and
  --- overrides with its own ceiling for `edit`.
  function c.sharing:create_link(drawing_id, spec)
    spec = spec or {}
    local body = { permission = spec.permission or M.PERMISSIONS.view }
    if spec.expires_at == false then
      body.expiresAt = NULL_MARK
    elseif spec.expires_at ~= nil then
      body.expiresAt = spec.expires_at
    end
    local payload = api_post("/drawings/" .. urlencode(drawing_id) .. "/link-shares",
      encode_with_nulls(body), true, "drawing sharing")
    return payload and payload.share
  end

  function c.sharing:revoke_link(drawing_id, share_id)
    return api_delete(
      "/drawings/" .. urlencode(drawing_id) .. "/link-shares/" .. urlencode(share_id),
      true, "drawing sharing")
  end

  --- Scoped to a drawing you own, and needs three characters, both of which
  --- narrow how far the directory can be enumerated.
  function c.sharing:resolve_users(drawing_id, q)
    local payload = api_get("/drawings/" .. urlencode(drawing_id) .. "/share-resolve", { q = q },
      true, "drawing user lookup")
    return payload and payload.users or {}
  end

  c.base_url = base_url
  c.api_path = api_path
  c.has_api_key = api_key ~= nil
  c.has_session = token ~= nil

  return c
end

-- ===== Helpers =====

--- Every drawing, following `offset` until the page comes back short. The
--- server caps a page at 200 however large a limit is asked for.
function M.all_drawings(c, opts)
  local o = {}
  for k, v in pairs(opts or {}) do o[k] = v end
  o.limit = o.limit or 200
  o.offset = o.offset or 0

  local all = {}
  while true do
    local page = c.drawings:list(o)
    local rows = page and page.drawings or {}
    for _, d in ipairs(rows) do all[#all + 1] = d end
    if #rows < o.limit then return all end
    o.offset = o.offset + #rows
    if page.totalCount and #all >= page.totalCount then return all end
  end
end

--- Exact-name lookup. The server's `search` filter is a substring match, so it
--- narrows the scan but cannot stand in for the comparison.
function M.find_drawing_by_name(c, name, opts)
  local o = {}
  for k, v in pairs(opts or {}) do o[k] = v end
  o.search = name
  for _, d in ipairs(M.all_drawings(c, o)) do
    if d.name == name then return d end
  end
  return nil
end

--- Idempotent create: returns the existing drawing when the name is taken.
--- Names are not unique to the server, so this is the caller's constraint.
function M.ensure_drawing(c, spec)
  if type(spec) ~= "table" or not spec.name then
    error("excalidash: ensure_drawing requires spec.name")
  end
  local found = M.find_drawing_by_name(c, spec.name, { collection_id = spec.collection_id })
  if found then return found end
  return c.drawings:create(spec)
end

--- Collections owned by the caller, with Trash left out — it exists on every
--- account and is never what a name is being resolved against.
function M.collections(c)
  local out = {}
  for _, col in ipairs(c.collections:list() or {}) do
    if col.id ~= M.TRASH and col.isOwner ~= false then out[#out + 1] = col end
  end
  return out
end

--- The collection to act on: the one matching a name, else the only one.
--- Refuses to guess between several.
function M.resolve_collection(c, name)
  local cols = M.collections(c)
  if name then
    for _, col in ipairs(cols) do
      if col.name == name then return col end
    end
    error("excalidash: no collection named " .. tostring(name))
  end
  if #cols == 0 then error("excalidash: account has no collections") end
  if #cols > 1 then
    error("excalidash: account has " .. #cols .. " collections; pass a name to disambiguate")
  end
  return cols[1]
end

function M.ensure_collection(c, name)
  if not name or name == "" then error("excalidash: ensure_collection requires a name") end
  for _, col in ipairs(M.collections(c)) do
    if col.name == name then return col end
  end
  return c.collections:create(name)
end

--- Move a drawing to Trash. Trash is an ordinary collection, so this is a
--- normal update — and unlike `drawings:delete` it is reversible by moving the
--- drawing back out.
function M.trash(c, drawing_id)
  return c.drawings:update(drawing_id, { collection_id = M.TRASH })
end

--- Restore the newest snapshot, which is the state before the last scene write.
--- Reads the drawing first for the version the guarded restore needs, so an
--- edit landing between the two calls loses the race rather than the edit.
function M.undo_last_change(c, drawing_id)
  local page = c.history:list(drawing_id, { limit = 1 })
  local newest = page and page.snapshots and page.snapshots[1]
  if not newest then return nil end
  local current = c.drawings:get(drawing_id)
  if not current then return nil end
  return c.history:restore(drawing_id, newest.id, current.version)
end

return M
