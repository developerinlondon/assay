---
category: AI & Agents
---

## assay.neutron

Neutron self-hosted agent platform — the full admin API from Lua: agents (personas, tool policies,
guardrails, baked assay modules), secrets, git-host connections, workspace/guide resources, roles,
instance settings, API tokens, usage. One client per instance; manage a fleet by creating several.
Client: `neutron.client(url?, opts?)` — `url` defaults to env `NEUTRON_URL`; `opts.token` to env
`NEUTRON_TOKEN` (a bearer minted in the instance's Settings → API, or its `NEUTRON_BOOTSTRAP_TOKEN`
on first boot). Instances behind Cloudflare Access also send a service token: `opts.cf_client_id` /
`opts.cf_client_secret` (env `CF_ACCESS_CLIENT_ID` / `CF_ACCESS_CLIENT_SECRET`).

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
