--- @module assay.neutron
--- @description Neutron self-hosted agent platform — full admin API: agents (personas, tool policies, guardrails, baked assay modules), secrets, git-host connections, workspace/guide resources, roles, instance settings, API tokens, usage. One client per instance; manage a fleet by creating several.
--- @category saas
--- @keywords neutron, agent, agents, admin, fleet, secrets, connections, workspaces, guides, roles, tokens, persona, tool-policy
--- @env NEUTRON_URL, NEUTRON_TOKEN, CF_ACCESS_CLIENT_ID, CF_ACCESS_CLIENT_SECRET
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

  local function headers()
    local h = {
      Authorization = "Bearer " .. token,
      ["Content-Type"] = "application/json",
    }
    if cf_id and cf_secret then
      h["CF-Access-Client-Id"] = cf_id
      h["CF-Access-Client-Secret"] = cf_secret
    end
    return h
  end

  local function request(method, path_str, payload)
    local url_full = base_url .. path_str
    local resp
    if method == "GET" then
      resp = http.get(url_full, { headers = headers() })
    elseif method == "POST" then
      resp = http.post(url_full, payload or {}, { headers = headers() })
    elseif method == "PUT" then
      resp = http.put(url_full, payload or {}, { headers = headers() })
    elseif method == "DELETE" then
      resp = http.delete(url_full, { headers = headers() })
    end
    if resp.status >= 400 then
      error("neutron: " .. method .. " " .. path_str .. " HTTP " .. resp.status .. ": " .. resp.body)
    end
    if resp.body == nil or resp.body == "" then return true end
    return json.parse(resp.body)
  end

  local c = {}

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

  return c
end

return M
