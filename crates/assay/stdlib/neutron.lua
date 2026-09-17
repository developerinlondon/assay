--- @module assay.neutron
--- @description Neutron self-hosted agent platform — full admin API: agents (personas, tool policies, guardrails, baked assay modules), secrets, git-host connections, workspace/guide resources, roles, instance settings, API tokens, usage. One client per instance; manage a fleet by creating several. Also the CRM surface (campaigns, people, inbox, held sends, suppressions, mailbox fleet, domains, integrations, budgets, deals, claims, reports) and the work tracker (projects, items, plans, sprints).
--- @category saas
--- @keywords neutron, agent, agents, admin, fleet, secrets, connections, workspaces, guides, roles, tokens, persona, tool-policy, crm, campaign, campaigns, inbox, sends, suppressions, deals, claims, work, projects, items, plans, sprints
--- @env NEUTRON_URL, NEUTRON_TOKEN, NEUTRON_EXTRA_HEADERS, CF_ACCESS_CLIENT_ID, CF_ACCESS_CLIENT_SECRET
--- @quickref c.agents:list() -> {agents, default_agent, defaults, brand} | Named agents + core-agent config
--- @quickref c.agents:create(display_name, config?) -> agent | Create a named agent
--- @quickref c.agents:update(id, config) -> agent | Replace a named agent's override set
--- @quickref c.agents:delete(id) -> true | Delete a named agent
--- @quickref c.agents:core() -> {stored, defaults} | The core agent's stored config
--- @quickref c.agents:update_core(config) -> {stored} | Replace the core agent's override set
--- @quickref c.secrets:list() -> [secret] | Redacted secrets (has_value + masked preview)
--- @quickref c.secrets:set(name, opts?) -> [secret] | Upsert value/note/agent scope
--- @quickref c.secrets:value(name) -> string | Admin read-back of a secret value
--- @quickref c.secrets:delete(name) -> [secret] | Delete a secret
--- @quickref c.connections:list() -> [connection] | Git-host connections (redacted)
--- @quickref c.connections:set(name, opts) -> [connection] | Upsert kind/base_url/token/agent scope
--- @quickref c.connections:delete(name) -> [connection] | Delete a connection
--- @quickref c.resources:list() -> [resource] | Workspaces + guides with grantable ids
--- @quickref c.resources:create(body) -> resource | New workspace or guide
--- @quickref c.resources:update(id, body) -> resource | Replace a resource
--- @quickref c.resources:delete(id) -> true | Delete a resource
--- @quickref c.roles:list() -> [role] | Approver roles with members
--- @quickref c.roles:create(name) -> role | New approver role
--- @quickref c.roles:member(id, email, action) -> {members} | action = "add"|"remove"
--- @quickref c.roles:delete(id) -> true | Delete a role
--- @quickref c.tokens:list() -> [token] | API token metadata (name, prefix, created)
--- @quickref c.tokens:mint(name) -> {name, token} | New bearer token (value shown ONCE)
--- @quickref c.tokens:revoke(name) -> [token] | Revoke a token
--- @quickref c.settings:get() -> {theme, approvals} | Instance-wide settings
--- @quickref c.settings:update(body) -> table | Update theme and/or approvals
--- @quickref c.channels:get() -> table | Channel config (secrets redacted)
--- @quickref c.channels:update(body) -> table | Update channel config
--- @quickref c.users:list() -> [user] | Users the instance has seen
--- @quickref c.usage:get(opts?) -> table | Usage rollup (opts.from / opts.to ISO)
--- @quickref c.assay_catalog() -> [module] | The instance's assay module catalog
--- @quickref neutron.client(url?, opts?) -> c | opts.headers = extra headers on every request
--- @quickref c:request(method, path, body?) -> body, status | Raw call; non-2xx returns the error body
--- @quickref c.crm.campaigns:list() -> body, status | All campaigns with counts
--- @quickref c.crm.campaigns:get(id) -> body, status | One campaign
--- @quickref c.crm.campaigns:overview(id) -> body, status | Campaign headline numbers
--- @quickref c.crm.campaigns:pipeline(id) -> body, status | People per pipeline state
--- @quickref c.crm.campaigns:board(id) -> body, status | Campaign board view
--- @quickref c.crm.campaigns:companies(id) -> body, status | Discovered companies
--- @quickref c.crm.campaigns:setup(id) -> body, status | Setup/readiness checklist
--- @quickref c.crm.campaigns:create(body) -> body, status | {name, id?, locale?, kind?, brief?, identity_pool?}
--- @quickref c.crm.campaigns:update(id, body) -> body, status | PATCH {name?, brief?, locale?}
--- @quickref c.crm.campaigns:delete(id) -> body, status | Delete a campaign
--- @quickref c.crm.campaigns:activate(id) -> body, status | Start sending
--- @quickref c.crm.campaigns:pause(id) -> body, status | Stop sending
--- @quickref c.crm.campaigns:archive(id) -> body, status | Archive a campaign
--- @quickref c.crm.campaigns:unarchive(id) -> body, status | Restore an archived campaign
--- @quickref c.crm.campaigns:enrol(id, body?) -> body, status | {person_ids?} — absent means all eligible
--- @quickref c.crm.campaigns:discover(id) -> body, status | Start a company-discovery run
--- @quickref c.crm.campaigns:discover_stop(id) -> body, status | Stop the running discovery
--- @quickref c.crm.campaigns:sequence_put(id, body) -> body, status | {steps=[{subject, body_text, offset_days?}]} — replaces the whole sequence
--- @quickref c.crm.campaigns:setup_put(id, body) -> body, status | {brief?, locale?, timezone?, send_window?, review_policy?, budget?, ...}
--- @quickref c.crm.campaigns:sequencer(id, body) -> body, status | {sequencer=neutron|salesforge|smartlead, sequencer_ref?}
--- @quickref c.crm.campaigns:experiments(id) -> body, status | A/B experiments on a campaign
--- @quickref c.crm.campaigns:experiment_create(id, body) -> body, status | {name, arms=[{name, subject?, text?}], holdout_fraction?}
--- @quickref c.crm.campaigns:learning(id) -> body, status | What the campaign has learned
--- @quickref c.crm.campaigns:terminals(id) -> body, status | Terminal-state breakdown
--- @quickref c.crm.experiments:close(id) -> body, status | Close a running experiment
--- @quickref c.crm.people:list(query?) -> body, status | query {page, limit, campaign, verdict, state, q}
--- @quickref c.crm.people:get(id) -> body, status | One person
--- @quickref c.crm.people:touches(id) -> body, status | Every logged touch for a person
--- @quickref c.crm.people:state(id, body) -> body, status | {to=<PersonState>, reason}
--- @quickref c.crm.people:touch(id, body) -> body, status | {channel, label, direction?} — hand-log a touch
--- @quickref c.crm.people:verify(body) -> body, status | {campaign, person_ids?} — run email verification
--- @quickref c.crm.people:reopen(id) -> body, status | Clear one person's verification verdict
--- @quickref c.crm.people:reopen_many(body) -> body, status | {campaign, person_ids} (1..50)
--- @quickref c.crm.people:import(body, opts?) -> body, status | {campaign, csv, filename?, mapping?, mapping_check?}; DRY RUN unless opts.dry_run=false
--- @quickref c.crm.people:imports(query) -> body, status | {campaign} required — that campaign's import batches
--- @quickref c.crm.people:import_batch(batch) -> body, status | One import batch
--- @quickref c.crm.people:import_delete(batch) -> body, status | Roll back an import batch
--- @quickref c.crm.people:import_check(sha256) -> body, status | Has this CSV been imported
--- @quickref c.crm.inbox:list(query?) -> body, status | query {label, campaign, identity, unanswered, cursor, limit}
--- @quickref c.crm.inbox:thread(id) -> body, status | One conversation thread
--- @quickref c.crm.inbox:draft_get(id) -> body, status | One reply draft
--- @quickref c.crm.inbox:draft_create(body) -> body, status | {person_id, body_text, campaign?, identity?, subject?}
--- @quickref c.crm.inbox:draft_update(id, body) -> body, status | PATCH {subject?, body_text?, body_html?, claim_ids?}
--- @quickref c.crm.inbox:draft_delete(id) -> body, status | Discard a draft
--- @quickref c.crm.inbox:draft_send(id) -> body, status | Send an approved draft
--- @quickref c.crm.inbox:touch_draft(touch_id, body?) -> body, status | {label?} — compose a reply to one inbound turn
--- @quickref c.crm.inbox:touch_label(touch_id, body) -> body, status | {label=<ReplyLabel>} — correct a reply's reading
--- @quickref c.crm.sends:pending(query?) -> body, status | query {campaign} — the held queue
--- @quickref c.crm.sends:approve(id) -> body, status | Release one held send
--- @quickref c.crm.sends:reject(id) -> body, status | Drop one held send
--- @quickref c.crm.replies:classify(email_id, body) -> body, status | {kind="positive"|"declined"|"other"}
--- @quickref c.crm.suppressions:list() -> body, status | Do-not-contact list
--- @quickref c.crm.suppressions:add(body) -> body, status | {value, scope_kind?="domain"|"address", note?}
--- @quickref c.crm.suppressions:import(body) -> body, status | {csv, note?}
--- @quickref c.crm.suppressions:lift(scope, value) -> body, status | Remove one suppression
--- @quickref c.crm.fleet:get() -> body, status | Mailbox fleet state
--- @quickref c.crm.fleet:summary() -> body, status | Fleet health rollup
--- @quickref c.crm.fleet:mailboxes() -> body, status | Every mailbox
--- @quickref c.crm.fleet:fleet_mailboxes(id) -> body, status | Mailboxes of one fleet
--- @quickref c.crm.fleet:fleets_get() -> body, status | Fleet ids and labels
--- @quickref c.crm.fleet:fleets_put(body) -> body, status | {fleets={id=label}} — rename fleets
--- @quickref c.crm.domains:list() -> body, status | Sending domains with health
--- @quickref c.crm.domains:add(body) -> body, status | {domain, provider?, fleet?}
--- @quickref c.crm.domains:pause(domain, body?) -> body, status | {reason?} — stop sending from a domain
--- @quickref c.crm.domains:resume(domain) -> body, status | Resume a paused domain
--- @quickref c.crm.domains:recheck(domain, body?) -> body, status | {dkim_selector?} — re-run DNS/auth checks
--- @quickref c.crm.domains:sync() -> body, status | Re-sync domains from providers
--- @quickref c.crm.integrations:list() -> body, status | Providers with redacted secrets
--- @quickref c.crm.integrations:put(provider, body) -> body, status | {secrets?, settings?} — empty secret clears
--- @quickref c.crm.integrations:test(provider) -> body, status | Probe provider credentials
--- @quickref c.crm.integrations:account_create(provider, body) -> body, status | {label?, fleet_label?} — add an account
--- @quickref c.crm.integrations:account_put(provider, account, body) -> body, status | Per-account {secrets?, settings?}
--- @quickref c.crm.integrations:account_update(provider, account, body) -> body, status | {label?, fleet_label?} — rename only, never keys
--- @quickref c.crm.integrations:account_delete(provider, account) -> body, status | Remove an account and its secrets (not the first)
--- @quickref c.crm.integrations:account_webhook_secret(provider, account) -> body, status | Rotate one account's webhook secret
--- @quickref c.crm.integrations:account_secret_delete(provider, account, key) -> body, status | Drop one stored per-account secret
--- @quickref c.crm.integrations:account_test(provider, account) -> body, status | Probe one account
--- @quickref c.crm.integrations:webhook(provider) -> body, status | Register the provider webhook
--- @quickref c.crm.integrations:account_webhook(provider, account) -> body, status | Register a per-account webhook
--- @quickref c.crm.integrations:webhook_secret(provider) -> body, status | Rotate the webhook signing secret
--- @quickref c.crm.integrations:secret_delete(provider, key) -> body, status | Drop one stored secret
--- @quickref c.crm.integrations:sequences(provider) -> body, status | Sequences the provider exposes
--- @quickref c.crm.integrations:lead_data_order_put(body) -> body, status | {order=[provider]} — paid-data waterfall
--- @quickref c.crm.integrations:lead_data_figures() -> body, status | Spend and hit rate per lead-data provider
--- @quickref c.crm.budgets:get() -> body, status | Spend ceilings
--- @quickref c.crm.budgets:put(body) -> body, status | {scope_id, auto_approve_cents, monthly_cap_cents, scope_kind?} — keyed on scope_kind+scope_id, default "campaign"
--- @quickref c.crm.costs:get() -> body, status | Cost rollup (the route takes no filters)
--- @quickref c.crm.deals:list(query?) -> body, status | query {campaign, state}
--- @quickref c.crm.deals:advance(id, body) -> body, status | {state, value_cents?, commission_cents?, owner?}
--- @quickref c.crm.claims:list(query?) -> body, status | query {citable="1", locale} — citable needs a locale
--- @quickref c.crm.claims:create(body) -> body, status | {claim, proof_source, locale?}
--- @quickref c.crm.claims:update(id, body) -> body, status | PATCH {claim?, proof_source?} — un-approves
--- @quickref c.crm.claims:approve(id, body) -> body, status | {review_by="YYYY-MM-DD"} — required, the date it stops being citable
--- @quickref c.crm.claims:retire(id) -> body, status | Retire a claim
--- @quickref c.crm.claims:restore(id) -> body, status | Restore a retired claim
--- @quickref c.crm.identities:list() -> body, status | Send-as identities
--- @quickref c.crm.identities:create(body) -> body, status | {actor, address, display_name, domain?, signature?}
--- @quickref c.crm.identities:update(body) -> body, status | PATCH {actor, address, active?, display_name?, signature?}
--- @quickref c.crm.reports:conversion(query?) -> body, status | query {since="YYYY-MM-DD"} — conversion report
--- @quickref c.crm.reports:corrections(query?) -> body, status | query {since} — label corrections report
--- @quickref c.crm.reports:deliverability(query?) -> body, status | query {since} — deliverability report
--- @quickref c.crm.reports:rot() -> body, status | Data-rot report (the route takes no filters)
--- @quickref c.crm.reports:weekly_note(query?) -> body, status | query {week="YYYY-MM-DD"} — Monday-anchored
--- @quickref c.crm:overview() -> body, status | Whole-CRM headline numbers
--- @quickref c.crm:agent() -> body, status | The CRM agent's configuration
--- @quickref c.crm.proposals:list() -> body, status | Outstanding suppression proposals
--- @quickref c.crm.proposals:accept(id) -> body, status | Accept a proposal (409 once settled)
--- @quickref c.crm.proposals:decline(id) -> body, status | Decline a proposal (409 once settled)
--- @quickref c.crm:sequencer_events(query?) -> body, status | query {campaign, kind, limit}
--- @quickref c.crm:sequencer_event_reapply(id) -> body, status | Re-apply one sequencer event
--- @quickref c.work.projects:list() -> body, status | Work projects
--- @quickref c.work.projects:board(id, query?) -> body, status | query {sprint, epic, label, assignee, done="1", cancelled="1"}
--- @quickref c.work.projects:meta(id, body) -> body, status | PATCH {key} — set the reference prefix
--- @quickref c.work.projects:labels(id) -> body, status | Project labels
--- @quickref c.work.projects:label_add(id, body) -> body, status | {name, color?, description?}
--- @quickref c.work.projects:stages(id) -> body, status | Board stages
--- @quickref c.work.projects:stages_put(id, body) -> body, status | {stages=[{key, label, status, position}]} replaces all
--- @quickref c.work.projects:sprints(id) -> body, status | Sprints of a project
--- @quickref c.work.projects:sprint_add(id, body) -> body, status | {name, startsOn, endsOn, goal?, state?}
--- @quickref c.work.projects:plans(id) -> body, status | Plans of a project
--- @quickref c.work.projects:plan_add(id, body) -> body, status | {title, summary?, links?, phases?}
--- @quickref c.work.projects:report(id) -> body, status | Throughput and status report
--- @quickref c.work.projects:timeline(id) -> body, status | Milestone timeline
--- @quickref c.work.projects:adopt_code_refs(id, body?) -> body, status | {dry_run?} — bulk relink forge refs
--- @quickref c.work.items:list(query?) -> body, status | query {kind, status, project_id, parent_id ("root" = top level), repository_id, sprint_id, assignee, labels, due_before, q}
--- @quickref c.work.items:get(id) -> body, status | One work item
--- @quickref c.work.items:create(body) -> body, status | {kind, title, description?, status?, priority?, labels?, ...}
--- @quickref c.work.items:update(id, body) -> body, status | PATCH; needs {expected_revision}
--- @quickref c.work.items:move(id, body, opts?) -> body, status | {to_status, expected_revision, before_id?, after_id?, sprint_id?}; opts.unassign_sprint sends sprint_id null
--- @quickref c.work.items:claim(id, body) -> body, status | {expected_revision} — take the item
--- @quickref c.work.items:comment(id, body) -> body, status | {body, mentions?}
--- @quickref c.work.items:relate(id, body) -> body, status | {targetId, kind=blocks|relates|duplicates|precedes}
--- @quickref c.work.items:unrelate(id, target_id, kind) -> body, status | Drop one relation
--- @quickref c.work.items:snapshot(id) -> body, status | Snapshot the item
--- @quickref c.work.items:revisions(id) -> body, status | Revision history
--- @quickref c.work.items:code_link(id, body) -> body, status | {provider="gitlab"|"github", url}
--- @quickref c.work.items:code_unlink(id) -> body, status | Drop the forge link
--- @quickref c.work.plans:get(id) -> body, status | One plan
--- @quickref c.work.plans:update(id, body) -> body, status | PATCH; needs {expected_revision}
--- @quickref c.work.plans:submit(id) -> body, status | Submit a draft plan for decision
--- @quickref c.work.plans:decide(id, body) -> body, status | {accept} — anything but true rejects
--- @quickref c.work.plans:signoff(id) -> body, status | Sign off a plan
--- @quickref c.work.plans:withdraw_signoff(id) -> body, status | Withdraw a sign-off
--- @quickref c.work.plans:complete(id) -> body, status | Mark a plan complete
--- @quickref c.work.sprints:update(id, body) -> body, status | PATCH {name?, startsOn?, endsOn?, goal?, state?}
--- @quickref c.work.sprints:close(id, body?) -> body, status | {carry_to?} — "backlog" (default), "next", or a sprint id
--- @quickref c.work.labels:update(id, body) -> body, status | PATCH {name?, color?, description?}
--- @quickref c.work.labels:delete(id) -> body, status | Delete a label
--- @quickref c.work:ready(query?) -> body, status | query {project} — items ready to pick up
--- @quickref c.work:repositories() -> body, status | Linked code repositories
--- @quickref c.work:import_clickup(body) -> body, status | {team_id, token?, org_id?, dry_run?}

local urlmod = require("assay.url")

local CRM = "/api/admin/crm"
local WORK = "/api/work"

local function check_header_table(t, source)
  for k in pairs(t) do
    if type(k) ~= "string" then
      error("neutron: " .. source .. " must map header names to values")
    end
  end
  return t
end

--- opts.headers wins; otherwise NEUTRON_EXTRA_HEADERS, a JSON object.
local function resolve_extra_headers(from_opts)
  if from_opts ~= nil then
    if type(from_opts) ~= "table" then
      error("neutron: opts.headers must be a table of header -> value")
    end
    return check_header_table(from_opts, "opts.headers")
  end
  local raw = env.get("NEUTRON_EXTRA_HEADERS")
  if raw == nil or raw == "" then return nil end
  local ok, parsed = pcall(json.parse, raw)
  if not ok or type(parsed) ~= "table" then
    error("neutron: NEUTRON_EXTRA_HEADERS is not a JSON object of header -> value")
  end
  return check_header_table(parsed, "NEUTRON_EXTRA_HEADERS")
end

local M = {}

--- Create a client for one Neutron instance.
--- url defaults to env NEUTRON_URL; opts.token to env NEUTRON_TOKEN (a bearer
--- minted in the instance's Settings → API, or its NEUTRON_BOOTSTRAP_TOKEN on
--- first boot). Instances behind Cloudflare Access also need a service token —
--- opts.cf_client_id / opts.cf_client_secret (env CF_ACCESS_CLIENT_ID/SECRET).
function M.client(url, opts)
  opts = opts or {}
  local base_url = (url or env.get("NEUTRON_URL") or ""):gsub("/+$", "")
  if base_url == "" then
    error("neutron: no url — pass one or set NEUTRON_URL")
  end
  local token = opts.token or env.get("NEUTRON_TOKEN")
  if not token then
    error("neutron: no token — pass opts.token or set NEUTRON_TOKEN")
  end
  local cf_id = opts.cf_client_id or env.get("CF_ACCESS_CLIENT_ID")
  local cf_secret = opts.cf_client_secret or env.get("CF_ACCESS_CLIENT_SECRET")
  local extra = resolve_extra_headers(opts.headers)

  local function headers()
    local h = {
      Authorization = "Bearer " .. token,
      ["Content-Type"] = "application/json",
    }
    if cf_id and cf_secret then
      h["CF-Access-Client-Id"] = cf_id
      h["CF-Access-Client-Secret"] = cf_secret
    end
    if extra then
      for k, v in pairs(extra) do
        -- The bearer is the client's identity; an extra header may sit beside
        -- it (an internal-principal pair, a trace id) but never replace it.
        if k:lower() ~= "authorization" then h[k] = tostring(v) end
      end
    end
    return h
  end

  local function send(method, path_str, payload)
    local url_full = base_url .. path_str
    local h = headers()
    if method == "GET" then return http.get(url_full, { headers = h }) end
    if method == "POST" then return http.post(url_full, payload or {}, { headers = h }) end
    if method == "PUT" then return http.put(url_full, payload or {}, { headers = h }) end
    if method == "PATCH" then return http.patch(url_full, payload or {}, { headers = h }) end
    if method == "DELETE" then return http.delete(url_full, { headers = h }) end
    error("neutron: unsupported method " .. tostring(method))
  end

  local function seg(v) return urlmod.encode(tostring(v)) end

  local function qs(path_str, query)
    if not query then return path_str end
    local parts = {}
    for k, v in pairs(query) do
      if v ~= nil then
        parts[#parts + 1] = urlmod.encode(tostring(k)) .. "=" .. urlmod.encode(tostring(v))
      end
    end
    table.sort(parts)
    return #parts > 0 and (path_str .. "?" .. table.concat(parts, "&")) or path_str
  end

  --- JSON in, JSON out, status as a second return. A non-2xx does not throw:
  --- the API's own error body comes back so a caller can report it verbatim.
  --- Only a transport failure raises.
  local function rq(method, path_str, payload)
    local resp = send(method, path_str, payload)
    if resp.body == nil or resp.body == "" then return nil, resp.status end
    local ok, parsed = pcall(json.parse, resp.body)
    if ok then return parsed, resp.status end
    return resp.body, resp.status
  end

  local function request(method, path_str, payload)
    local resp = send(method, path_str, payload)
    if resp.status >= 400 then
      error("neutron: " .. method .. " " .. path_str .. " HTTP " .. resp.status .. ": " .. resp.body)
    end
    if resp.body == nil or resp.body == "" then return true end
    return json.parse(resp.body)
  end

  local c = {}

  function c:request(method, path_str, payload) return rq(method, path_str, payload) end

  -- ===== Agents =====
  -- Config fields (all optional; whole-collection fields REPLACE — read
  -- current values first and merge): identity, mode, tool_policy,
  -- approver_roles, approver_users, capabilities, default_model,
  -- allow_user_switch, agentkit{enabled,police}, resources[ids],
  -- approval_timeout_minutes, inherit_persona, assay_modules, admin_only.

  c.agents = {}

  function c.agents:list()
    return request("GET", "/api/admin/agents")
  end

  function c.agents:create(display_name, config)
    local body = config or {}
    body.display_name = display_name
    return request("POST", "/api/admin/agents", body)
  end

  function c.agents:update(id, config)
    return request("PUT", "/api/admin/agents/" .. id, config)
  end

  function c.agents:delete(id)
    return request("DELETE", "/api/admin/agents/" .. id)
  end

  function c.agents:core()
    return request("GET", "/api/admin/settings/agent")
  end

  function c.agents:update_core(config)
    return request("PUT", "/api/admin/settings/agent", config)
  end

  -- ===== Secrets =====

  c.secrets = {}

  function c.secrets:list()
    return request("GET", "/api/admin/secrets").secrets
  end

  --- opts: { value?, note?, agents? } — omit value to keep the stored one.
  function c.secrets:set(name, opts_)
    return request("PUT", "/api/admin/secrets/" .. name, opts_ or {}).secrets
  end

  function c.secrets:value(name)
    return request("GET", "/api/admin/secrets/" .. name .. "/value").value
  end

  function c.secrets:delete(name)
    return request("DELETE", "/api/admin/secrets/" .. name).secrets
  end

  -- ===== Connections (git-host bot identities) =====

  c.connections = {}

  function c.connections:list()
    return request("GET", "/api/admin/connections").connections
  end

  --- opts: { kind (gitlab|github, required), base_url?, token?, agents? } —
  --- agents scopes the connection to specific agents (their bot identity).
  function c.connections:set(name, opts_)
    return request("PUT", "/api/admin/connections/" .. name, opts_).connections
  end

  function c.connections:delete(name)
    return request("DELETE", "/api/admin/connections/" .. name).connections
  end

  -- ===== Resources (workspaces + guides) =====

  c.resources = {}

  function c.resources:list()
    return request("GET", "/api/admin/resources").resources
  end

  --- workspace: {name, type="workspace", access="ro"|"rw",
  ---   config={repos={{url=..., host="gitlab"|"github", default_branch=...}}}, guide=""}
  --- guide: {name, type="guide", access="ro", config={summary=...}, guide=markdown}
  function c.resources:create(body)
    return request("POST", "/api/admin/resources", body)
  end

  function c.resources:update(id, body)
    return request("PUT", "/api/admin/resources/" .. id, body)
  end

  function c.resources:delete(id)
    return request("DELETE", "/api/admin/resources/" .. id)
  end

  -- ===== Roles =====

  c.roles = {}

  function c.roles:list()
    return request("GET", "/api/admin/roles").roles
  end

  function c.roles:create(name)
    return request("POST", "/api/admin/roles", { name = name })
  end

  function c.roles:member(id, email, action)
    return request("PUT", "/api/admin/roles/" .. id .. "/members", { email = email, action = action })
  end

  function c.roles:delete(id)
    return request("DELETE", "/api/admin/roles/" .. id)
  end

  -- ===== API tokens =====

  c.tokens = {}

  function c.tokens:list()
    return request("GET", "/api/admin/tokens").tokens
  end

  function c.tokens:mint(name)
    return request("POST", "/api/admin/tokens", { name = name })
  end

  function c.tokens:revoke(name)
    return request("DELETE", "/api/admin/tokens/" .. name).tokens
  end

  -- ===== Instance settings / channels / users / usage =====

  c.settings = {}

  function c.settings:get()
    return request("GET", "/api/settings")
  end

  function c.settings:update(body)
    return request("PUT", "/api/admin/settings", body)
  end

  c.channels = {}

  function c.channels:get()
    return request("GET", "/api/admin/settings/channels").channels
  end

  function c.channels:update(body)
    return request("PUT", "/api/admin/settings/channels", body).channels
  end

  c.users = {}

  function c.users:list()
    return request("GET", "/api/admin/users").users
  end

  c.usage = {}

  --- opts: { from?, to? } (ISO timestamps)
  function c.usage:get(opts_)
    local q = {}
    if opts_ and opts_.from then q[#q + 1] = "from=" .. opts_.from end
    if opts_ and opts_.to then q[#q + 1] = "to=" .. opts_.to end
    local suffix = #q > 0 and ("?" .. table.concat(q, "&")) or ""
    return request("GET", "/api/admin/usage" .. suffix)
  end

  function c.assay_catalog()
    return request("GET", "/api/admin/assay/catalog").modules
  end

  -- ===== CRM =====
  -- Every c.crm.* and c.work.* helper returns (body, status) and does not throw
  -- on a non-2xx — the API's own error body comes back as the first return.

  local CAMP = CRM .. "/campaigns/"
  local PEOPLE = CRM .. "/people"
  local INBOX = CRM .. "/inbox"
  local INTEG = CRM .. "/integrations/"

  c.crm = {
    campaigns = {},
    experiments = {},
    people = {},
    inbox = {},
    sends = {},
    replies = {},
    suppressions = {},
    fleet = {},
    domains = {},
    integrations = {},
    budgets = {},
    costs = {},
    deals = {},
    claims = {},
    identities = {},
    proposals = {},
    reports = {},
  }

  local cg = c.crm.campaigns

  function cg:list() return rq("GET", CRM .. "/campaigns") end
  function cg:get(id) return rq("GET", CAMP .. seg(id)) end
  function cg:overview(id) return rq("GET", CAMP .. seg(id) .. "/overview") end
  function cg:pipeline(id) return rq("GET", CAMP .. seg(id) .. "/pipeline") end
  function cg:board(id) return rq("GET", CAMP .. seg(id) .. "/board") end
  function cg:companies(id) return rq("GET", CAMP .. seg(id) .. "/companies") end
  function cg:setup(id) return rq("GET", CAMP .. seg(id) .. "/setup") end
  function cg:create(body) return rq("POST", CRM .. "/campaigns", body) end
  function cg:update(id, body) return rq("PATCH", CAMP .. seg(id), body) end
  function cg:delete(id) return rq("DELETE", CAMP .. seg(id)) end
  function cg:activate(id) return rq("POST", CAMP .. seg(id) .. "/activate") end
  function cg:pause(id) return rq("POST", CAMP .. seg(id) .. "/pause") end
  function cg:archive(id) return rq("POST", CAMP .. seg(id) .. "/archive") end
  function cg:unarchive(id) return rq("POST", CAMP .. seg(id) .. "/unarchive") end
  function cg:enrol(id, body) return rq("POST", CAMP .. seg(id) .. "/enrol", body) end
  function cg:discover(id) return rq("POST", CAMP .. seg(id) .. "/discover") end
  function cg:discover_stop(id) return rq("POST", CAMP .. seg(id) .. "/discover/stop") end
  function cg:sequence_put(id, body) return rq("PUT", CAMP .. seg(id) .. "/sequence", body) end
  function cg:setup_put(id, body) return rq("PUT", CAMP .. seg(id) .. "/setup", body) end
  function cg:sequencer(id, body) return rq("POST", CAMP .. seg(id) .. "/sequencer", body) end
  function cg:experiments(id) return rq("GET", CAMP .. seg(id) .. "/experiments") end
  function cg:learning(id) return rq("GET", CAMP .. seg(id) .. "/learning") end
  function cg:terminals(id) return rq("GET", CAMP .. seg(id) .. "/terminals") end

  function cg:experiment_create(id, body)
    return rq("POST", CAMP .. seg(id) .. "/experiments", body)
  end

  function c.crm.experiments:close(id)
    return rq("POST", CRM .. "/experiments/" .. seg(id) .. "/close")
  end

  local pe = c.crm.people

  function pe:list(query) return rq("GET", qs(PEOPLE, query)) end
  function pe:get(id) return rq("GET", PEOPLE .. "/" .. seg(id)) end
  function pe:touches(id) return rq("GET", PEOPLE .. "/" .. seg(id) .. "/touches") end
  function pe:state(id, body) return rq("POST", PEOPLE .. "/" .. seg(id) .. "/state", body) end
  function pe:touch(id, body) return rq("POST", PEOPLE .. "/" .. seg(id) .. "/touches", body) end
  function pe:verify(body) return rq("POST", PEOPLE .. "/verify", body) end
  function pe:reopen(id) return rq("POST", PEOPLE .. "/" .. seg(id) .. "/reopen") end
  function pe:reopen_many(body) return rq("POST", PEOPLE .. "/reopen", body) end
  --- The importer writes only when told to: opts.dry_run defaults to true, so
  --- a caller that forgets it gets the plan back rather than a written batch.
  function pe:import(body, opts_)
    local dry = true
    if opts_ and opts_.dry_run ~= nil then dry = opts_.dry_run and true or false end
    return rq("POST", PEOPLE .. "/import" .. (dry and "?dry_run=1" or ""), body)
  end

  function pe:imports(query) return rq("GET", qs(PEOPLE .. "/imports", query)) end
  function pe:import_batch(batch) return rq("GET", PEOPLE .. "/import/" .. seg(batch)) end
  function pe:import_delete(batch) return rq("DELETE", PEOPLE .. "/import/" .. seg(batch)) end
  function pe:import_check(sha256)
    return rq("GET", PEOPLE .. "/import/check/" .. seg(sha256))
  end

  local ib = c.crm.inbox

  function ib:list(query) return rq("GET", qs(INBOX, query)) end
  function ib:thread(id) return rq("GET", INBOX .. "/threads/" .. seg(id)) end
  function ib:draft_get(id) return rq("GET", INBOX .. "/drafts/" .. seg(id)) end
  function ib:draft_create(body) return rq("POST", INBOX .. "/drafts", body) end
  function ib:draft_update(id, body) return rq("PATCH", INBOX .. "/drafts/" .. seg(id), body) end
  function ib:draft_delete(id) return rq("DELETE", INBOX .. "/drafts/" .. seg(id)) end
  function ib:draft_send(id) return rq("POST", INBOX .. "/drafts/" .. seg(id) .. "/send") end
  function ib:touch_draft(touch_id, body)
    return rq("POST", INBOX .. "/touches/" .. seg(touch_id) .. "/draft", body)
  end
  function ib:touch_label(touch_id, body)
    return rq("POST", INBOX .. "/touches/" .. seg(touch_id) .. "/label", body)
  end

  local sd = c.crm.sends

  function sd:pending(query) return rq("GET", qs(CRM .. "/sends/pending", query)) end
  function sd:approve(id) return rq("POST", CRM .. "/sends/" .. seg(id) .. "/approve") end
  function sd:reject(id) return rq("POST", CRM .. "/sends/" .. seg(id) .. "/reject") end

  function c.crm.replies:classify(email_id, body)
    return rq("POST", CRM .. "/replies/" .. seg(email_id) .. "/classify", body)
  end

  local su = c.crm.suppressions

  function su:list() return rq("GET", CRM .. "/suppressions") end
  function su:add(body) return rq("POST", CRM .. "/suppressions", body) end
  function su:import(body) return rq("POST", CRM .. "/suppressions/import", body) end
  function su:lift(scope, value)
    return rq("DELETE", CRM .. "/suppressions/" .. seg(scope) .. "/" .. seg(value))
  end

  local fl = c.crm.fleet

  function fl:get() return rq("GET", CRM .. "/fleet") end
  function fl:summary() return rq("GET", CRM .. "/fleet/summary") end
  function fl:mailboxes() return rq("GET", CRM .. "/fleet/mailboxes") end
  function fl:fleet_mailboxes(id) return rq("GET", CRM .. "/fleet/" .. seg(id) .. "/mailboxes") end
  function fl:fleets_get() return rq("GET", CRM .. "/fleets") end
  function fl:fleets_put(body) return rq("PUT", CRM .. "/fleets", body) end

  local dm = c.crm.domains

  function dm:list() return rq("GET", CRM .. "/domains") end
  function dm:add(body) return rq("POST", CRM .. "/domains", body) end
  function dm:pause(domain, body)
    return rq("POST", CRM .. "/domains/" .. seg(domain) .. "/pause", body)
  end
  function dm:resume(domain) return rq("POST", CRM .. "/domains/" .. seg(domain) .. "/resume") end
  function dm:recheck(domain, body)
    return rq("POST", CRM .. "/domains/" .. seg(domain) .. "/recheck", body)
  end
  function dm:sync() return rq("POST", CRM .. "/domains/sync") end

  local ig = c.crm.integrations

  function ig:list() return rq("GET", CRM .. "/integrations") end
  function ig:put(provider, body) return rq("PUT", INTEG .. seg(provider), body) end
  function ig:test(provider) return rq("POST", INTEG .. seg(provider) .. "/test") end
  function ig:webhook(provider) return rq("POST", INTEG .. seg(provider) .. "/webhook") end
  function ig:sequences(provider) return rq("GET", INTEG .. seg(provider) .. "/sequences") end
  function ig:lead_data_figures() return rq("GET", INTEG .. "lead-data/figures") end
  function ig:lead_data_order_put(body) return rq("PUT", INTEG .. "lead-data/order", body) end

  function ig:account_put(provider, account, body)
    return rq("PUT", INTEG .. seg(provider) .. "/accounts/" .. seg(account), body)
  end

  function ig:account_create(provider, body)
    return rq("POST", INTEG .. seg(provider) .. "/accounts", body)
  end

  function ig:account_update(provider, account, body)
    return rq("PATCH", INTEG .. seg(provider) .. "/accounts/" .. seg(account), body)
  end

  function ig:account_delete(provider, account)
    return rq("DELETE", INTEG .. seg(provider) .. "/accounts/" .. seg(account))
  end

  function ig:account_webhook_secret(provider, account)
    return rq("POST", INTEG .. seg(provider) .. "/accounts/" .. seg(account) .. "/webhook-secret")
  end

  function ig:account_secret_delete(provider, account, key)
    local tail = "/accounts/" .. seg(account) .. "/secrets/" .. seg(key)
    return rq("DELETE", INTEG .. seg(provider) .. tail)
  end

  function ig:account_test(provider, account)
    return rq("POST", INTEG .. seg(provider) .. "/accounts/" .. seg(account) .. "/test")
  end

  function ig:account_webhook(provider, account)
    return rq("POST", INTEG .. seg(provider) .. "/accounts/" .. seg(account) .. "/webhook")
  end

  function ig:webhook_secret(provider)
    return rq("POST", INTEG .. seg(provider) .. "/webhook-secret")
  end

  function ig:secret_delete(provider, key)
    return rq("DELETE", INTEG .. seg(provider) .. "/secrets/" .. seg(key))
  end

  function c.crm.budgets:get() return rq("GET", CRM .. "/budgets") end
  function c.crm.budgets:put(body) return rq("PUT", CRM .. "/budgets", body) end
  function c.crm.costs:get() return rq("GET", CRM .. "/costs") end

  function c.crm.deals:list(query) return rq("GET", qs(CRM .. "/deals", query)) end
  function c.crm.deals:advance(id, body)
    return rq("POST", CRM .. "/deals/" .. seg(id) .. "/advance", body)
  end

  local cl = c.crm.claims

  function cl:list(query) return rq("GET", qs(CRM .. "/claims", query)) end
  function cl:create(body) return rq("POST", CRM .. "/claims", body) end
  function cl:update(id, body) return rq("PATCH", CRM .. "/claims/" .. seg(id), body) end
  function cl:approve(id, body)
    return rq("POST", CRM .. "/claims/" .. seg(id) .. "/approve", body)
  end

  function cl:retire(id) return rq("POST", CRM .. "/claims/" .. seg(id) .. "/retire") end
  function cl:restore(id) return rq("POST", CRM .. "/claims/" .. seg(id) .. "/restore") end

  local idn = c.crm.identities

  function idn:list() return rq("GET", CRM .. "/identities") end
  function idn:create(body) return rq("POST", CRM .. "/identities", body) end
  function idn:update(body) return rq("PATCH", CRM .. "/identities", body) end

  local pr = c.crm.proposals

  function pr:list() return rq("GET", CRM .. "/proposals") end
  function pr:accept(id) return rq("POST", CRM .. "/proposals/" .. seg(id) .. "/accept") end
  function pr:decline(id) return rq("POST", CRM .. "/proposals/" .. seg(id) .. "/decline") end

  local rp = c.crm.reports

  function rp:conversion(query) return rq("GET", qs(CRM .. "/reports/conversion", query)) end
  function rp:corrections(query) return rq("GET", qs(CRM .. "/reports/corrections", query)) end
  function rp:deliverability(query)
    return rq("GET", qs(CRM .. "/reports/deliverability", query))
  end
  function rp:rot() return rq("GET", CRM .. "/reports/rot") end
  function rp:weekly_note(query) return rq("GET", qs(CRM .. "/reports/weekly-note", query)) end

  function c.crm:overview() return rq("GET", CRM .. "/overview") end
  function c.crm:agent() return rq("GET", CRM .. "/agent") end

  function c.crm:sequencer_events(query)
    return rq("GET", qs(CRM .. "/sequencer-events", query))
  end

  function c.crm:sequencer_event_reapply(id)
    return rq("POST", CRM .. "/sequencer-events/" .. seg(id) .. "/reapply")
  end

  -- ===== Work (projects, items, plans, sprints) =====

  local PROJ = WORK .. "/projects/"
  local ITEM = WORK .. "/items/"
  local PLAN = WORK .. "/plans/"

  c.work = { projects = {}, items = {}, plans = {}, sprints = {}, labels = {} }

  local pj = c.work.projects

  function pj:list() return rq("GET", WORK .. "/projects") end
  function pj:board(id, query) return rq("GET", qs(PROJ .. seg(id) .. "/board", query)) end
  function pj:meta(id, body) return rq("PATCH", PROJ .. seg(id) .. "/meta", body) end
  function pj:labels(id) return rq("GET", PROJ .. seg(id) .. "/labels") end
  function pj:label_add(id, body) return rq("POST", PROJ .. seg(id) .. "/labels", body) end
  function pj:stages(id) return rq("GET", PROJ .. seg(id) .. "/stages") end
  function pj:stages_put(id, body) return rq("PUT", PROJ .. seg(id) .. "/stages", body) end
  function pj:sprints(id) return rq("GET", PROJ .. seg(id) .. "/sprints") end
  function pj:sprint_add(id, body) return rq("POST", PROJ .. seg(id) .. "/sprints", body) end
  function pj:plans(id) return rq("GET", PROJ .. seg(id) .. "/plans") end
  function pj:plan_add(id, body) return rq("POST", PROJ .. seg(id) .. "/plans", body) end
  function pj:report(id) return rq("GET", PROJ .. seg(id) .. "/report") end
  function pj:timeline(id) return rq("GET", PROJ .. seg(id) .. "/timeline") end

  function pj:adopt_code_refs(id, body)
    return rq("POST", PROJ .. seg(id) .. "/code-host-refs/adopt", body)
  end

  local it = c.work.items

  function it:list(query) return rq("GET", qs(WORK .. "/items", query)) end
  function it:get(id) return rq("GET", ITEM .. seg(id)) end
  function it:create(body) return rq("POST", WORK .. "/items", body) end
  function it:update(id, body) return rq("PATCH", ITEM .. seg(id), body) end
  --- A Lua table cannot hold nil, so `sprint_id = nil` is an absent key and the
  --- item keeps the sprint it had. opts.unassign_sprint sends a body whose
  --- sprint_id really is JSON null, which is what takes it out of one.
  function it:move(id, body, opts_)
    if not (opts_ and opts_.unassign_sprint) then
      return rq("POST", ITEM .. seg(id) .. "/move", body)
    end
    local copy = {}
    for k, v in pairs(body or {}) do copy[k] = v end
    -- Both spellings go: the route reads sprint_id ?? sprintId, so a leftover
    -- camelCase key would win over the null and quietly keep the sprint.
    copy.sprint_id, copy.sprintId = nil, nil
    local encoded = json.encode(json.object(copy))
    local head = encoded:sub(1, #encoded - 1)
    local sep = #head > 1 and "," or ""
    return rq("POST", ITEM .. seg(id) .. "/move", head .. sep .. '"sprint_id":null}')
  end
  function it:claim(id, body) return rq("POST", ITEM .. seg(id) .. "/claim", body) end
  function it:comment(id, body) return rq("POST", ITEM .. seg(id) .. "/comments", body) end
  function it:relate(id, body) return rq("POST", ITEM .. seg(id) .. "/relations", body) end
  function it:snapshot(id) return rq("POST", ITEM .. seg(id) .. "/snapshots") end
  function it:revisions(id) return rq("GET", ITEM .. seg(id) .. "/revisions") end
  function it:code_link(id, body) return rq("POST", ITEM .. seg(id) .. "/code-host-link", body) end
  function it:code_unlink(id) return rq("DELETE", ITEM .. seg(id) .. "/code-host-link") end

  function it:unrelate(id, target_id, kind)
    local path_str = ITEM .. seg(id) .. "/relations/" .. seg(target_id) .. "/" .. seg(kind)
    return rq("DELETE", path_str)
  end

  local pl = c.work.plans

  function pl:get(id) return rq("GET", PLAN .. seg(id)) end
  function pl:update(id, body) return rq("PATCH", PLAN .. seg(id), body) end
  function pl:submit(id) return rq("POST", PLAN .. seg(id) .. "/submit") end
  function pl:decide(id, body) return rq("POST", PLAN .. seg(id) .. "/decide", body) end
  function pl:signoff(id) return rq("POST", PLAN .. seg(id) .. "/signoff") end
  function pl:withdraw_signoff(id) return rq("POST", PLAN .. seg(id) .. "/withdraw-signoff") end
  function pl:complete(id) return rq("POST", PLAN .. seg(id) .. "/complete") end

  local sp = c.work.sprints

  function sp:update(id, body) return rq("PATCH", WORK .. "/sprints/" .. seg(id), body) end
  function sp:close(id, body)
    return rq("POST", WORK .. "/sprints/" .. seg(id) .. "/close", body)
  end

  local lb = c.work.labels

  function lb:update(id, body) return rq("PATCH", WORK .. "/labels/" .. seg(id), body) end
  function lb:delete(id) return rq("DELETE", WORK .. "/labels/" .. seg(id)) end

  function c.work:ready(query) return rq("GET", qs(WORK .. "/ready", query)) end
  function c.work:repositories() return rq("GET", WORK .. "/repositories") end
  function c.work:import_clickup(body) return rq("POST", WORK .. "/import/clickup", body) end

  return c
end

return M
