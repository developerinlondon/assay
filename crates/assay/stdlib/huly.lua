--- @module assay.huly
--- @description Huly transactor REST API — document queries, transactions, fulltext search, and tracker helpers for projects, issues, milestones, and components on a self-hosted or cloud Huly workspace.
--- @keywords huly, hulylabs, huly.io, tracker, issue, project, milestone, component, task, transactor, document, transaction, tx, findall, workspace, self-hosted, project-management, knowledge-base, hcengineering
--- @quickref c:account() -> account | Identity behind the token
--- @quickref c:model(full?) -> [tx] | Model transactions describing every class
--- @quickref c:find_all(class, query?, options?) -> [doc] | Documents of a class
--- @quickref c:find_one(class, query?, options?) -> doc|nil | First matching document
--- @quickref c:count(class, query?) -> number | Total matches, without the rows
--- @quickref c:tx(tx) -> result | Post a raw transaction
--- @quickref c:create_doc(class, space, attrs, id?) -> id | Create a document
--- @quickref c:update_doc(class, space, id, ops, retrieve?) -> result | Update a document
--- @quickref c:remove_doc(class, space, id) -> true | Remove a document
--- @quickref c:search(query, opts?) -> {docs, total} | Fulltext search
--- @quickref c:ensure_person(social_type, social_value, first, last) -> person | Idempotent person upsert
--- @quickref M.new_id() -> string | A 24-hex document id in Huly's format
--- @quickref M.projects(c) -> [project] | Tracker projects in the workspace
--- @quickref M.resolve_project(c, identifier?) -> project | The only project, or the one matching an identifier or name
--- @quickref M.issues(c, project, opts?) -> [issue] | Issues in a tracker project
--- @quickref M.find_issue_by_title(c, project, title) -> issue|nil | Exact-title issue lookup
--- @quickref M.create_issue(c, project, spec) -> issue | Create an issue, numbering it from the project sequence
--- @quickref M.ensure_issue(c, project, spec) -> issue | Create an issue unless the title exists
--- @quickref M.set_issue_status(c, issue, status) -> true | Move an issue to another status
--- @quickref M.statuses(c) -> [status] | Issue statuses available to the tracker

local M = {}

local V1 = "/api/v1"

-- Model ids the transactor pins by convention rather than serving as documents.
M.TX_SPACE = "core:space:Tx"
M.NO_PARENT = "tracker:ids:NoParent"
M.ISSUE_CLASS = "tracker:class:Issue"
M.PROJECT_CLASS = "tracker:class:Project"
M.MILESTONE_CLASS = "tracker:class:Milestone"
M.COMPONENT_CLASS = "tracker:class:Component"
M.STATUS_CLASS = "tracker:class:IssueStatus"
M.ISSUE_TASK_TYPE = "tracker:taskTypes:Issue"

-- LexoRank's midpoint. Huly ranks issues with this when there is no neighbour
-- to sit between, and the transactor rejects a document with an empty rank.
local MIDDLE_RANK = "0|hzzzzz:"

local function hex(n, width)
  return string.format("%0" .. width .. "x", n)
end

local id_counter = 0

--- A document id in Huly's format: 8 hex of unix seconds, 10 hex of randomness,
--- 6 hex of a process counter. `isId` on the server side accepts exactly 24 hex
--- characters, so the widths are not free.
function M.new_id()
  id_counter = (id_counter + 1) % 0x1000000
  local rand = crypto.hash(crypto.random(32), "sha256"):sub(1, 10)
  return hex(os.time(), 8) .. rand .. hex(id_counter, 6)
end

function M.client(opts)
  opts = opts or {}
  local token = opts.token or env.get("HULY_TOKEN")
  local workspace = opts.workspace or env.get("HULY_WORKSPACE")
  local account = opts.account or env.get("HULY_ACCOUNT")
  local base_url = (opts.base_url or env.get("HULY_URL") or "https://huly.app"):gsub("/+$", "")
  local endpoint = base_url:match("/_transactor$") and base_url or (base_url .. "/_transactor")

  -- Huly's own client asks for `snappy, gzip`. assay builds reqwest without the
  -- gzip feature and has no snappy decoder, so anything but identity comes back
  -- as binary the JSON parser cannot read.
  local function headers()
    local h = {
      ["Content-Type"] = "application/json",
      ["Accept"] = "application/json",
      ["Accept-Encoding"] = "identity",
    }
    if token then h["Authorization"] = "Bearer " .. token end
    return h
  end

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

  local function ws()
    if not workspace or workspace == "" then
      error("huly: no workspace uuid; pass workspace= or set HULY_WORKSPACE")
    end
    return urlencode(workspace)
  end

  local function decode(resp)
    if resp.body and resp.body ~= "" then
      local ok, parsed = pcall(json.parse, resp.body)
      if ok then return parsed end
    end
    return nil
  end

  -- Failures arrive as {message, error}; surface the inner error, which names
  -- the operation, rather than the generic outer message.
  local function fail(verb, route, resp)
    local detail = resp.body or ""
    local parsed = decode(resp)
    if type(parsed) == "table" and parsed.error then detail = tostring(parsed.error) end
    error("huly: " .. verb .. " " .. route .. " HTTP " .. resp.status .. ": " .. detail)
  end

  local function api_get(route, query_params)
    local resp = http.get(endpoint .. V1 .. route .. build_query(query_params), { headers = headers() })
    if resp.status ~= 200 then fail("GET", route, resp) end
    return decode(resp)
  end

  local function api_post(route, payload)
    local resp = http.post(endpoint .. V1 .. route, payload or {}, { headers = headers() })
    if resp.status ~= 200 and resp.status ~= 201 then fail("POST", route, resp) end
    return decode(resp)
  end

  local c = {}

  -- ===== Identity and model =====

  function c:account()
    return api_get("/account/" .. ws())
  end

  --- Every model transaction. `full` includes the transactions the UI needs and
  --- roughly doubles the payload; the class and attribute definitions are in
  --- both.
  function c:model(full)
    return api_get("/load-model/" .. ws(), full and { full = "true" } or nil)
  end

  -- ===== Queries =====

  --- Collections come back wrapped as
  --- `{dataType = "TotalArray", total, lookupMap, value = [...]}`.
  local function unwrap(payload)
    if type(payload) ~= "table" then return {}, -1 end
    if payload.dataType == "TotalArray" or payload.value ~= nil then
      return payload.value or {}, payload.total or -1
    end
    return payload, -1
  end

  --- The transactor drops attributes the query already pins to a scalar, and
  --- leaves `_class` off documents fetched by class. Huly's own client patches
  --- both back before returning, and callers reading `doc.identifier` after
  --- querying on it depend on that.
  local function restore(docs, class, query)
    for _, doc in ipairs(docs) do
      if type(doc) == "table" then
        if doc._class == nil then doc._class = class end
        for k, v in pairs(query or {}) do
          local t = type(v)
          if (t == "string" or t == "number" or t == "boolean") and doc[k] == nil then
            doc[k] = v
          end
        end
      end
    end
    return docs
  end

  local function find(class, query, options)
    if not class or class == "" then error("huly: find requires a class id") end
    local params = { class = class }
    if query and next(query) ~= nil then params.query = json.encode(query) end
    if options and next(options) ~= nil then params.options = json.encode(options) end
    return unwrap(api_get("/find-all/" .. ws(), params))
  end

  function c:find_all(class, query, options)
    local docs = find(class, query, options)
    return restore(docs, class, query)
  end

  function c:find_one(class, query, options)
    local opts = {}
    for k, v in pairs(options or {}) do opts[k] = v end
    opts.limit = 1
    return self:find_all(class, query, opts)[1]
  end

  --- The row count without the rows. `total` is -1 unless it is asked for, so
  --- this always asks.
  function c:count(class, query)
    local _, total = find(class, query, { limit = 1, total = true })
    return total
  end

  function c:search(query, opts)
    opts = opts or {}
    local params = { query = query }
    if opts.classes then params.classes = json.encode(opts.classes) end
    if opts.spaces then params.spaces = json.encode(opts.spaces) end
    if opts.limit then params.limit = tostring(opts.limit) end
    local payload = api_get("/search-fulltext/" .. ws(), params)
    return { docs = payload and payload.docs or {}, total = payload and payload.total or 0 }
  end

  -- ===== Transactions =====

  --- The account that authors transactions. Huly stamps `modifiedBy` with a
  --- social id, not the person uuid, and the token's primary social id is the
  --- one the UI attributes changes to.
  function c:actor()
    if not account then
      local acc = self:account()
      account = acc and (acc.primarySocialId or (acc.socialIds or {})[1])
      if not account then error("huly: cannot resolve an actor social id from the token") end
    end
    return account
  end

  function c:tx(tx)
    return api_post("/tx/" .. ws(), tx)
  end

  --- Returns the new document's id so the caller can read it back or attach to
  --- it; the transactor answers a bare `[]` on success.
  function c:create_doc(class, space, attrs, object_id)
    local id = object_id or M.new_id()
    self:tx({
      _id = M.new_id(),
      _class = "core:class:TxCreateDoc",
      space = M.TX_SPACE,
      objectId = id,
      objectClass = class,
      objectSpace = space,
      modifiedOn = math.floor(os.time() * 1000),
      modifiedBy = self:actor(),
      createdBy = self:actor(),
      attributes = attrs or {},
    })
    return id
  end

  --- `ops` is a document update: plain fields assign, `$inc`/`$push`/`$pull`
  --- mutate. With `retrieve` the transactor answers `{object = <new doc>}`,
  --- which is how a sequence can be incremented and read atomically.
  function c:update_doc(class, space, object_id, ops, retrieve)
    return self:tx({
      _id = M.new_id(),
      _class = "core:class:TxUpdateDoc",
      space = M.TX_SPACE,
      objectId = object_id,
      objectClass = class,
      objectSpace = space,
      modifiedOn = math.floor(os.time() * 1000),
      modifiedBy = self:actor(),
      operations = ops or {},
      retrieve = retrieve or false,
    })
  end

  function c:remove_doc(class, space, object_id)
    self:tx({
      _id = M.new_id(),
      _class = "core:class:TxRemoveDoc",
      space = M.TX_SPACE,
      objectId = object_id,
      objectClass = class,
      objectSpace = space,
      modifiedOn = math.floor(os.time() * 1000),
      modifiedBy = self:actor(),
    })
    return true
  end

  -- ===== People =====

  function c:ensure_person(social_type, social_value, first_name, last_name)
    return api_post("/ensure-person/" .. ws(), {
      socialType = social_type,
      socialValue = social_value,
      firstName = first_name,
      lastName = last_name,
    })
  end

  c.workspace = workspace
  c.base_url = base_url
  c.endpoint = endpoint
  c.authenticated = token ~= nil

  return c
end

-- ===== Tracker helpers =====

--- Every tracker project. Projects are spaces, so they carry `identifier` (the
--- issue prefix) alongside `name`.
function M.projects(c)
  return c:find_all(M.PROJECT_CLASS, {}, { sort = { name = 1 } })
end

--- The project to act on: matched on identifier first, then name, else the only
--- one. Refuses to guess between several.
function M.resolve_project(c, key)
  local projects = M.projects(c)
  if key then
    for _, p in ipairs(projects) do
      if p.identifier == key then return p end
    end
    for _, p in ipairs(projects) do
      if p.name == key then return p end
    end
    error("huly: no project with identifier or name " .. tostring(key))
  end
  if #projects == 0 then error("huly: token can see no tracker projects") end
  if #projects > 1 then
    error("huly: token sees " .. #projects .. " projects; pass an identifier to disambiguate")
  end
  return projects[1]
end

local function project_id(project)
  return type(project) == "table" and project._id or project
end

function M.statuses(c)
  return c:find_all(M.STATUS_CLASS, { ofAttribute = "tracker:attribute:IssueStatus" })
end

--- Issues in a project. Extra `opts` merge into the find options, so
--- `{limit = 50, sort = {number = -1}}` works.
function M.issues(c, project, opts)
  local options = { sort = { modifiedOn = -1 } }
  for k, v in pairs(opts or {}) do options[k] = v end
  return c:find_all(M.ISSUE_CLASS, { space = project_id(project) }, options)
end

function M.find_issue_by_title(c, project, title)
  return c:find_one(M.ISSUE_CLASS, { space = project_id(project), title = title })
end

--- Create an issue.
---
--- Issue numbering lives on the project, not the issue: the number and the
--- `PREFIX-N` identifier come from an atomic increment of the project's
--- `sequence`, and an issue created without them is invisible in the UI. Anyone
--- writing the transaction by hand has to do the same two steps.
function M.create_issue(c, project, spec)
  if type(spec) ~= "table" or not spec.title then
    error("huly: create_issue requires spec.title")
  end
  local proj = type(project) == "table" and project or M.resolve_project(c, project)
  local bumped = c:update_doc(M.PROJECT_CLASS, proj.space or "core:space:Space", proj._id,
    { ["$inc"] = { sequence = 1 } }, true)
  local number = bumped and bumped.object and bumped.object.sequence
  if not number then error("huly: project " .. tostring(proj._id) .. " did not return a sequence") end

  local attrs = {
    title = spec.title,
    description = spec.description or nil,
    status = spec.status or proj.defaultIssueStatus or "tracker:status:Backlog",
    priority = spec.priority or 0,
    assignee = spec.assignee or nil,
    component = spec.component or nil,
    milestone = spec.milestone or nil,
    dueDate = spec.dueDate or nil,
    estimation = spec.estimation or 0,
    remainingTime = spec.remainingTime or 0,
    reportedTime = 0,
    reports = 0,
    comments = 0,
    subIssues = 0,
    childInfo = json.array({}),
    parents = json.array({}),
    number = number,
    identifier = proj.identifier .. "-" .. number,
    rank = spec.rank or MIDDLE_RANK,
    kind = spec.kind or M.ISSUE_TASK_TYPE,
    attachedTo = spec.parent or M.NO_PARENT,
    attachedToClass = M.ISSUE_CLASS,
    collection = "subIssues",
  }
  local id = c:create_doc(M.ISSUE_CLASS, proj._id, attrs)
  return c:find_one(M.ISSUE_CLASS, { _id = id })
end

--- Idempotent create: returns the existing issue when the title is taken.
function M.ensure_issue(c, project, spec)
  if type(spec) ~= "table" or not spec.title then
    error("huly: ensure_issue requires spec.title")
  end
  local proj = type(project) == "table" and project or M.resolve_project(c, project)
  local found = M.find_issue_by_title(c, proj, spec.title)
  if found then return found end
  return M.create_issue(c, proj, spec)
end

function M.set_issue_status(c, issue, status)
  c:update_doc(M.ISSUE_CLASS, issue.space, issue._id, { status = status })
  return true
end

function M.delete_issue(c, issue)
  return c:remove_doc(M.ISSUE_CLASS, issue.space, issue._id)
end

--- Milestones and components hang off a project space and need no numbering.
function M.create_milestone(c, project, spec)
  local proj = type(project) == "table" and project or M.resolve_project(c, project)
  local id = c:create_doc(M.MILESTONE_CLASS, proj._id, {
    label = spec.label,
    description = spec.description or "",
    status = spec.status or 0,
    targetDate = spec.targetDate,
    comments = 0,
    attachments = 0,
  })
  return c:find_one(M.MILESTONE_CLASS, { _id = id })
end

function M.create_component(c, project, spec)
  local proj = type(project) == "table" and project or M.resolve_project(c, project)
  local id = c:create_doc(M.COMPONENT_CLASS, proj._id, {
    label = spec.label,
    description = spec.description or "",
    lead = spec.lead or nil,
    comments = 0,
    attachments = 0,
  })
  return c:find_one(M.COMPONENT_CLASS, { _id = id })
end

function M.milestones(c, project)
  return c:find_all(M.MILESTONE_CLASS, { space = project_id(project) })
end

function M.components(c, project)
  return c:find_all(M.COMPONENT_CLASS, { space = project_id(project) })
end

return M
