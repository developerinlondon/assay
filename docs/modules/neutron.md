---
category: AI & Agents
tagline: Neutron self-hosted agent platform — the admin API, the CRM (campaigns, people, inbox, held sends, fleet, deals, claims) and the work tracker, from Lua
---

## assay.neutron

Neutron self-hosted agent platform — the full admin API from Lua: agents (personas, tool policies,
guardrails, baked assay modules), secrets, git-host connections, workspace/guide resources, roles,
instance settings, API tokens, usage. One client per instance; manage a fleet by creating several.
Client: `neutron.client(url?, opts?)` — `url` defaults to env `NEUTRON_URL`; `opts.token` to env
`NEUTRON_TOKEN` (a bearer minted in the instance's Settings → API, or its `NEUTRON_BOOTSTRAP_TOKEN`
on first boot). Instances behind Cloudflare Access also send a service token: `opts.cf_client_id` /
`opts.cf_client_secret` (env `CF_ACCESS_CLIENT_ID` / `CF_ACCESS_CLIENT_SECRET`).

`opts.headers` adds arbitrary headers to every request — an in-pod procedure running as the
instance's internal principal sends `x-neutron-internal` and `x-neutron-token` beside its bearer
rather than instead of it. When `opts.headers` is absent the client reads `NEUTRON_EXTRA_HEADERS`, a
JSON object of header name to value; anything that is not such an object is an error rather than a
silently unauthenticated call. Neither source can replace the `Authorization` header.

- `c.agents:list()` → `{agents, default_agent, defaults, brand}` — named agents + core-agent config
- `c.agents:create(display_name, config?)` → agent — create a named agent
- `c.agents:update(id, config)` → agent — replace a named agent's override set
- `c.agents:delete(id)` → true — delete a named agent
- `c.agents:core()` → `{stored, defaults}` — the core agent's stored config
- `c.agents:update_core(config)` → `{stored}` — replace the core agent's override set

Agent config fields (all optional; whole-collection fields REPLACE — read current values first and
merge): `identity`, `mode`, `tool_policy`, `approver_roles`, `approver_users`, `capabilities`,
`default_model`, `allow_user_switch`, `agentkit{enabled,police}`, `resources[ids]`,
`approval_timeout_minutes`, `inherit_persona`, `assay_modules`, `admin_only`.

- `c.secrets:list()` → [secret] — redacted (has_value + masked preview)
- `c.secrets:set(name, opts?)` → [secret] — upsert `{value?, note?, agents?}`; omit value to keep
- `c.secrets:value(name)` → string — admin read-back
- `c.secrets:delete(name)` → [secret]
- `c.connections:list()` → [connection] — git-host connections (redacted)
- `c.connections:set(name, opts)` → [connection] — `{kind, base_url?, token?, agents?}`; `agents`
  scopes the connection to specific agents (their bot identity)
- `c.connections:delete(name)` → [connection]
- `c.resources:list()` → [resource] — workspaces + guides with grantable ids
- `c.resources:create(body)` → resource — workspace:
  `{name, type="workspace", access="ro"|"rw", config={repos={{url, host, default_branch}}}}`; guide:
  `{name, type="guide", access="ro", config={summary}, guide=markdown}`
- `c.resources:update(id, body)` → resource
- `c.resources:delete(id)` → true
- `c.roles:list()` → [role] — approver roles with members
- `c.roles:create(name)` → role
- `c.roles:member(id, email, action)` → `{members}` — action `"add"` | `"remove"`
- `c.roles:delete(id)` → true
- `c.tokens:list()` → [token] — API token metadata (name, prefix, created)
- `c.tokens:mint(name)` → `{name, token}` — the value is shown ONCE
- `c.tokens:revoke(name)` → [token]
- `c.settings:get()` → `{theme, approvals}`
- `c.settings:update(body)` → table — `{theme?, approvals?={timeout_minutes}}`
- `c.channels:get()` / `c.channels:update(body)` — channel config (secrets redacted)
- `c.users:list()` → [user]
- `c.usage:get(opts?)` → table — `{from?, to?}` ISO timestamps
- `c.assay_catalog()` → [module] — the instance's assay module catalog

### Raw calls

- `c:request(method, path, body?)` → `body, status` — JSON in, JSON out. Unlike the admin helpers
  above it does **not** raise on a non-2xx: the API's own error body comes back as the first return
  with its status as the second, so a caller can report the failure verbatim. Only a transport
  failure raises. Every `c.crm.*` and `c.work.*` helper below is built on it and shares that
  contract.

### CRM

Campaigns:

- `c.crm.campaigns:list()` / `:get(id)` / `:delete(id)`
- `c.crm.campaigns:overview(id)` / `:pipeline(id)` / `:board(id)` / `:companies(id)` /
  `:terminals(id)` / `:learning(id)` — read views
- `c.crm.campaigns:create(body)` — `{name, id?, locale?, kind?="b2b"|"b2c", brief?, identity_pool?}`
- `c.crm.campaigns:update(id, body)` — PATCH `{name?, brief?, locale?}`
- `c.crm.campaigns:activate(id)` / `:pause(id)` / `:archive(id)` / `:unarchive(id)`
- `c.crm.campaigns:enrol(id, body?)` — `{person_ids?}`; absent means every eligible person
- `c.crm.campaigns:discover(id)` / `:discover_stop(id)` — company discovery runs
- `c.crm.campaigns:sequence_put(id, body)` — replaces the whole sequence:
  `{steps=[{subject, body_text?, body_html?, step?, offset_days?, template_version?}]}`
- `c.crm.campaigns:setup_put(id, body)` —
  `{brief?, locale?, timezone?, send_window?,
  review_policy?, confirm_no_human_review?, identity_pool?, budget?}`
  / `:setup(id)` reads it back
- `c.crm.campaigns:sequencer(id, body)` —
  `{sequencer="neutron"|"salesforge"|"smartlead",
  sequencer_ref?}`
- `c.crm.campaigns:experiments(id)` / `:experiment_create(id, body)` —
  `{name, arms=[{name, subject?, text?, html?}], holdout_fraction?}`
- `c.crm.experiments:close(id)`

People:

- `c.crm.people:list(query?)` — query `{page, limit, campaign, verdict, state, q}`
- `c.crm.people:get(id)` / `:touches(id)`
- `c.crm.people:state(id, body)` — `{to, reason}` moves a person along the pipeline
- `c.crm.people:touch(id, body)` — `{channel, label, direction?}` hand-logs an off-channel touch
- `c.crm.people:verify(body)` — `{campaign, person_ids?}`
- `c.crm.people:reopen(id)` / `:reopen_many(body)` — `{campaign, person_ids}` (1..50)
- `c.crm.people:import(body, opts?)` — `{campaign, csv, filename?, mapping?, mapping_check?}`. **It
  dry-runs by default**: `opts.dry_run` is true unless you pass `false`, because the dry run is a
  query parameter the route reads and not a body key, so a caller who forgets it would otherwise
  import for real on the first try. A dry run returns the plan and writes nothing
- `c.crm.people:imports(query)` — `{campaign}` is required; the route 400s without it
- `c.crm.people:import_batch(batch)` / `:import_delete(batch)` / `:import_check(sha256)`

Inbox, sends and replies:

- `c.crm.inbox:list(query?)` — query `{label, campaign, identity, unanswered, cursor, limit}`
- `c.crm.inbox:thread(id)`
- `c.crm.inbox:draft_get(id)` / `:draft_create(body)` / `:draft_update(id, body)` /
  `:draft_delete(id)` / `:draft_send(id)` — create takes
  `{person_id, body_text, campaign?, identity?, subject?, claim_ids?}`
- `c.crm.inbox:touch_draft(touch_id, body?)` — `{label?}` composes a reply to one inbound turn
- `c.crm.inbox:touch_label(touch_id, body)` — `{label}` corrects a reply's reading
- `c.crm.sends:pending(query?)` / `:approve(id)` / `:reject(id)` — the held queue
- `c.crm.replies:classify(email_id, body)` — `{kind="positive"|"declined"|"other"}`
- `c.crm.suppressions:list()` / `:add(body)` / `:import(body)` / `:lift(scope, value)`

Sending estate:

- `c.crm.fleet:get()` / `:summary()` / `:mailboxes()` / `:fleet_mailboxes(id)`
- `c.crm.fleet:fleets_get()` / `:fleets_put(body)` — `{fleets={id=label}}`
- `c.crm.domains:list()` / `:add(body)` / `:resume(domain)` / `:sync()`
- `c.crm.domains:pause(domain, body?)` — `{reason?}`; absent records "paused by <caller>"
- `c.crm.domains:recheck(domain, body?)` — `{dkim_selector?}` re-runs the DNS and auth checks
- `c.crm.identities:list()` / `:create(body)` / `:update(body)` — send-as addresses

Integrations, money and evidence:

- `c.crm.integrations:list()` / `:put(provider, body)` / `:test(provider)` / `:webhook(provider)` /
  `:webhook_secret(provider)` / `:secret_delete(provider, key)` / `:sequences(provider)`
- `c.crm.integrations:account_create(provider, body)` — `{label?, fleet_label?}`
- `c.crm.integrations:account_update(provider, account, body)` — `{label?, fleet_label?}`: the
  rename path only, so it can never write a credential by accident
- `c.crm.integrations:account_put(provider, account, body)` — `{secrets?, settings?}`
- `c.crm.integrations:account_delete(provider, account)` — the first account cannot be removed;
  clear its keys instead
- `c.crm.integrations:account_test(provider, account)` / `:account_webhook(provider, account)` /
  `:account_webhook_secret(provider, account)` / `:account_secret_delete(provider, account, key)`
- `c.crm.integrations:lead_data_order_put(body)` / `:lead_data_figures()` — the paid-data waterfall
- `c.crm.budgets:get()` / `:put(body)` —
  `{scope_id, auto_approve_cents, monthly_cap_cents,
  scope_kind?}`. `scope_kind` is `"actor"` or,
  for anything else including absence, `"campaign"`; the stored ceiling is keyed on the pair, so one
  `scope_id` can hold one of each
- `c.crm.costs:get()` — the route takes no filters
- `c.crm.deals:list(query?)` / `c.crm.deals:advance(id, body)`
- `c.crm.claims:list(query?)` — query `{citable, locale}`; `citable="1"` is a literal and then
  `locale` is required, returning only approved claims still inside their review date
- `c.crm.claims:create(body)` / `:update(id, body)` / `:retire(id)` / `:restore(id)`
- `c.crm.claims:approve(id, body)` — `{review_by="YYYY-MM-DD"}` is required: the approval carries
  the date the claim stops being citable, and the approver is the caller, never a name in the body
- `c.crm.reports:conversion(query?)` / `:corrections(query?)` / `:deliverability(query?)` — each
  takes `{since}`; conversion wants `YYYY-MM-DD`, the other two take anything `Date` parses
- `c.crm.reports:rot()` — takes no filters
- `c.crm.reports:weekly_note(query?)` — `{week}`, a Monday-anchored `YYYY-MM-DD`; it defaults to the
  current week and feeds the other three reports as their `since`
- `c.crm:overview()` / `:agent()`
- `c.crm.proposals:list()` / `:accept(id)` / `:decline(id)` — a proposal settles once, so a second
  decision is a 409 rather than a silent no-op
- `c.crm:sequencer_events(query?)` / `:sequencer_event_reapply(id)`

### Work tracker

- `c.work.projects:list()` / `:report(id)` / `:timeline(id)`
- `c.work.projects:board(id, query?)` — query `{sprint, epic, label, assignee, done, cancelled}`.
  `sprint` takes `"current"`, `"none"` or an id; `done` and `cancelled` are compared against the
  literal `"1"`, and `done="1"` shows cancelled cards too
- `c.work.projects:meta(id, body)` — PATCH `{key}` sets the reference prefix
- `c.work.projects:labels(id)` / `:label_add(id, body)` / `:stages(id)` / `:stages_put(id, body)`
- `c.work.projects:sprints(id)` / `:sprint_add(id, body)` / `:plans(id)` / `:plan_add(id, body)`
- `c.work.projects:adopt_code_refs(id, body?)` — `{dry_run?}`
- `c.work.items:list(query?)` — query
  `{kind, status, parent_id, repository_id, project_id,
  sprint_id, assignee, labels, due_before, q}`.
  `parent_id` takes a uuid or the literal `"root"` for top-level items only; `due_before` is an ISO
  date-time; `labels` is comma-separated
- `c.work.items:get(id)` / `:create(body)` / `:update(id, body)` — update is optimistic-locked and
  needs `{expected_revision}`
- `c.work.items:move(id, body, opts?)` —
  `{to_status, expected_revision, before_id?,
  after_id?, sprint_id?}`. Leaving `sprint_id` out
  keeps the item's sprint. Taking it out of one needs a JSON null, which a Lua table cannot carry —
  `sprint_id = nil` is simply an absent key — so pass `opts.unassign_sprint = true` and the body
  goes on the wire with `"sprint_id":null`. Both spellings are dropped from the body first, because
  the route reads `sprint_id ?? sprintId` and a leftover camelCase key would outrank the null
- `c.work.items:claim(id, body)` — `{expected_revision}`
- `c.work.items:comment(id, body)` / `:relate(id, body)` / `:unrelate(id, target_id, kind)`
- `c.work.items:snapshot(id)` / `:revisions(id)` / `:code_link(id, body)` / `:code_unlink(id)`
- `c.work.plans:get(id)` / `:update(id, body)` / `:submit(id)` / `:decide(id, body)` /
  `:signoff(id)` / `:withdraw_signoff(id)` / `:complete(id)`
- `c.work.sprints:update(id, body)`
- `c.work.sprints:close(id, body?)` — `{carry_to?}` decides where unfinished items go: `"backlog"`
  (the default), `"next"` for the next scheduled sprint, or another sprint's id
- `c.work.labels:update(id, body)` / `:delete(id)`
- `c.work:ready(query?)` / `:repositories()` / `:import_clickup(body)`

```lua
local neutron = require("assay.neutron")

-- take over a newborn instance with its bootstrap token, mint a real one
local boot = neutron.client("https://agent.example.com", { token = env.get("BOOTSTRAP") })
local real = boot.tokens:mint("fleet-ops") -- bootstrap token goes dead after this

-- configure it end to end with the real token
local c = neutron.client("https://agent.example.com", { token = real.token })
c.connections:set("gitlab-bot", { kind = "gitlab", token = env.get("BOT_PAT") })
c.agents:create("Reviewer", {
  mode = "approval-gated",
  assay_modules = { "assay.gitlab", "assay.k8s" },
})
```

A procedure running inside the instance talks to itself over localhost, and reports a non-2xx rather
than dying on it:

```lua
local neutron = require("assay.neutron")
local c = neutron.client() -- NEUTRON_URL + NEUTRON_TOKEN

local held, status = c.crm.sends:pending({ campaign = "alpha" })
if status ~= 200 then
  print("held queue unavailable: " .. status)
  return
end
for _, send in ipairs(held.sends or {}) do
  print(send.id .. " -> " .. (send.person or "?"))
end
```

Not mirrored: `GET /api/admin/crm/people/:id/photo` returns an image rather than JSON, so it does
not fit this client's JSON-in, JSON-out contract. Reach it with `http.get` and the same headers, or
through `c:request` when only its status matters.
