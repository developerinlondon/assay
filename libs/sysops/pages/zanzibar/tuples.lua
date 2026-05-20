local render = require("pages.render")
local ctx    = require("sysops.ctx")
local form   = require("pages.form")
local auth   = require("sysops.auth")

local M = {}

local function urlenc(s)
  return (tostring(s or "")):gsub("([^%w%-_%.~])", function(c)
    return string.format("%%%02X", string.byte(c))
  end)
end

local function tuple_body(f)
  return {
    object_type  = f.object_type  or "",
    object_id    = f.object_id    or "",
    relation     = f.relation     or "",
    subject_type = f.subject_type or "",
    subject_id   = f.subject_id   or "",
    subject_rel  = f.subject_rel  or "",
  }
end

local function relation_names(ns)
  -- NamespaceSchema rows from the engine carry a `definitions` map
  -- whose keys are the relation / permission names available on the
  -- namespace. Fall back to `relations` if a future API rename ships.
  local defs = (type(ns) == "table") and (ns.definitions or ns.relations) or nil
  local names = {}
  if type(defs) == "table" then
    for k, _ in pairs(defs) do
      if type(k) == "string" then names[#names + 1] = k end
    end
    table.sort(names)
  end
  return names
end

function M.page(req)
  local q        = (req and req.params) or {}
  local sdk_root = auth.new(ctx.engine)
  local sdk      = sdk_root.zanzibar

  local data, err = sdk.tuples()
  local tuples = {}
  local unsupported = false
  local status = err and err.status or 200
  if err and (err.status == 404 or err.status == 405 or err.status == 501) then
    unsupported = true
  elseif data and type(data.items) == "table" then
    tuples = data.items
  elseif data and type(data.tuples) == "table" then
    tuples = data.tuples
  end

  -- Users: id, email lookup for the table label AND the subject-id
  -- dropdown in the write dialog.
  local users_data, _ = sdk_root.users.list({ limit = 500 })
  local user_email_by_id = {}
  local user_options = {}
  if users_data and type(users_data.items) == "table" then
    for _, u in ipairs(users_data.items) do
      if u.id then
        local email = (u.email and u.email ~= "") and u.email or nil
        if email then user_email_by_id[u.id] = email end
        user_options[#user_options + 1] = {
          id    = u.id,
          email = email or u.id,
        }
      end
    end
    table.sort(user_options, function(a, b) return tostring(a.email) < tostring(b.email) end)
  end

  -- Namespaces: name → ordered list of relation/permission names. Drives
  -- the object-type and relation dropdowns in the write dialog.
  local ns_data, _ = sdk.namespaces()
  local namespaces = {}
  local raw_ns = (ns_data and type(ns_data.items) == "table") and ns_data.items
                 or (ns_data and type(ns_data.namespaces) == "table") and ns_data.namespaces
                 or (type(ns_data) == "table" and ns_data[1] ~= nil) and ns_data
                 or {}
  for _, n in ipairs(raw_ns) do
    if n.name then
      namespaces[#namespaces + 1] = {
        name      = n.name,
        relations = relation_names(n),
      }
    end
  end
  table.sort(namespaces, function(a, b) return a.name < b.name end)

  -- Derive object-id suggestions per object_type from the existing
  -- tuple list (powers the object_id datalist). Same for group ids.
  local object_ids_by_type = {}
  local group_ids = {}
  local seen_group = {}
  for _, t in ipairs(tuples) do
    if t.object_type and t.object_id then
      local set = object_ids_by_type[t.object_type] or {}
      set[t.object_id] = true
      object_ids_by_type[t.object_type] = set
    end
    if t.subject_type == "group" and t.subject_id and not seen_group[t.subject_id] then
      seen_group[t.subject_id] = true
      group_ids[#group_ids + 1] = t.subject_id
    end
  end
  local object_ids_table = {}
  for ot, set in pairs(object_ids_by_type) do
    local ids = {}
    for id, _ in pairs(set) do ids[#ids + 1] = id end
    table.sort(ids)
    object_ids_table[ot] = ids
  end
  table.sort(group_ids)

  for _, t in ipairs(tuples) do
    if t.subject_type == "user" then
      t.subject_label = user_email_by_id[t.subject_id] or t.subject_id
    else
      t.subject_label = t.subject_id
    end
  end

  local zb_data_json = json.encode({
    namespaces = namespaces,
    users      = user_options,
    object_ids = object_ids_table,
    group_ids  = group_ids,
  })

  local filter = { limit = (data and data.limit) or 100, offset = (data and data.offset) or 0 }
  return render.render("zanzibar/tuples", {
    nav_active   = "zanzibar:tuples",
    title        = "Tuples · zanzibar · auth",
    page_title   = "Zanzibar tuples",
    tuples       = tuples,
    filter       = filter,
    unsupported  = unsupported,
    error        = (not unsupported) and err or nil,
    status       = status,
    saved        = q.saved == "1" and true or nil,
    deleted      = q.deleted == "1" and true or nil,
    write_err    = q.write_err or nil,
    delete_err   = q.delete_err or nil,
    zb_data_json = zb_data_json,
  }, req)
end

-- Invalidate sysops.authz's per-(sub, tuple) cache whenever a tuple is
-- written or deleted, so the next request through gateway.proxy /
-- require_session sees the new permissions without waiting for the
-- 30s TTL. Best-effort: pcall so a load failure (auth gateway not
-- wired) doesn't kill the mutation path.
local function invalidate_authz_cache()
  local ok, authz = pcall(require, "sysops.authz")
  if ok and authz and authz.invalidate then authz.invalidate() end
end

function M.write(req)
  local f   = form.parse(req)
  local sdk = auth.new(ctx.engine).zanzibar
  local _, err = sdk.write_tuple(tuple_body(f))
  if err then
    return {
      status  = 303,
      headers = {
        Location = "/zanzibar/tuples"
          .. "?write_err=" .. tostring(err.status)
          .. "&form_object_type=" .. urlenc(f.object_type or "")
          .. "&form_object_id=" .. urlenc(f.object_id or "")
          .. "&form_relation=" .. urlenc(f.relation or "")
          .. "&form_subject_type=" .. urlenc(f.subject_type or "")
          .. "&form_subject_id=" .. urlenc(f.subject_id or "")
          .. "&form_subject_rel=" .. urlenc(f.subject_rel or ""),
      },
    }
  end
  invalidate_authz_cache()
  return { status = 303, headers = { Location = "/zanzibar/tuples?saved=1" } }
end

function M.delete(req)
  local f   = form.parse(req)
  local sdk = auth.new(ctx.engine).zanzibar
  local _, err = sdk.delete_tuple(tuple_body(f))
  if err then
    return { status = 303, headers = { Location = "/zanzibar/tuples?delete_err=" .. tostring(err.status) } }
  end
  invalidate_authz_cache()
  return { status = 303, headers = { Location = "/zanzibar/tuples?deleted=1" } }
end

return M
