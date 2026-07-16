--- @module assay.openstack
--- @description OpenStack inventory client. Keystone v3 auth and service discovery; identity, compute, image, network, and quota reads.
--- @keywords openstack, keystone, nova, glance, neutron, identity, compute, image, network, quota, servers
--- @quickref c:authenticate() -> table | Authenticate with Keystone v3 (POST, approval-gated)
--- @quickref c.identity:list_projects(opts?) -> [project] | List projects (GET, read)
--- @quickref c.identity:list_users(opts?) -> [user] | List users (GET, read)
--- @quickref c.compute:list_servers(opts?) -> [server] | List detailed servers (GET, read)
--- @quickref c.compute:get_limits(opts?) -> limits | Show compute limits (GET, read)
--- @quickref c.compute:get_quota(project_id, opts?) -> quota | Show compute quota detail (GET, read)
--- @quickref c.image:list_images(opts?) -> [image] | List images (GET, read)
--- @quickref c.network:list_networks(opts?) -> [network] | List networks (GET, read)
--- @quickref c.network:list_ports(opts?) -> [port] | List ports (GET, read)
--- @quickref c.network:get_quota(project_id) -> quota | Show network quota (GET, read)

local M = {}

local function encode(value)
  return tostring(value):gsub("([^A-Za-z0-9%-_.~])", function(ch)
    return string.format("%%%02X", string.byte(ch))
  end)
end

local function build_query(params)
  local parts = {}
  for key, value in pairs(params or {}) do
    if value ~= nil then
      if type(value) == "table" then
        for _, item in ipairs(value) do
          parts[#parts + 1] = encode(key) .. "=" .. encode(item)
        end
      else
        parts[#parts + 1] = encode(key) .. "=" .. encode(value)
      end
    end
  end
  table.sort(parts)
  if #parts == 0 then return "" end
  return "?" .. table.concat(parts, "&")
end

local function strip_slash(url)
  return url:gsub("/+$", "")
end

local function domain_ref(name, id)
  if id then return { id = id } end
  return { name = name or "Default" }
end

function M.client(auth_url, opts)
  opts = opts or {}
  local identity_url = strip_slash(auth_url or "")
  if identity_url == "" then error("openstack: auth_url is required") end

  local token = opts.token
  local catalog = opts.catalog or {}
  local project_id = opts.project_id
  local endpoints = opts.endpoints or {}
  local region = opts.region
  local endpoint_interface = opts.interface or "public"

  local function auth_headers()
    return { ["Accept"] = "application/json", ["Content-Type"] = "application/json" }
  end

  local function service_headers()
    return {
      ["Accept"] = "application/json",
      ["Content-Type"] = "application/json",
      ["X-Auth-Token"] = token,
    }
  end

  local function password_payload()
    if not opts.username or not opts.password then
      error("openstack: username and password are required")
    end
    if not opts.project_id and not opts.project_name then
      error("openstack: project_id or project_name is required")
    end
    local project
    if opts.project_id then
      project = { id = opts.project_id }
    else
      project = {
        name = opts.project_name,
        domain = domain_ref(opts.project_domain_name, opts.project_domain_id),
      }
    end
    return {
      auth = {
        identity = {
          methods = { "password" },
          password = {
            user = {
              name = opts.username,
              password = opts.password,
              domain = domain_ref(opts.user_domain_name, opts.user_domain_id),
            },
          },
        },
        scope = { project = project },
      },
    }
  end

  local function authenticate()
    if token then return { project_id = project_id, catalog = catalog } end
    local path_str = "/auth/tokens"
    local resp = http.post(identity_url .. path_str, password_payload(), {
      headers = auth_headers(),
    })
    if resp.status ~= 200 and resp.status ~= 201 then
      error("openstack: POST " .. path_str .. " HTTP " .. resp.status .. ": " .. resp.body)
    end
    token = resp.headers["x-subject-token"] or resp.headers["X-Subject-Token"]
    if not token or token == "" then
      error("openstack: Keystone response missing X-Subject-Token")
    end
    local data = json.parse(resp.body)
    local token_data = data.token or {}
    catalog = token_data.catalog or {}
    if token_data.project then project_id = token_data.project.id end
    return { project_id = project_id, catalog = catalog }
  end

  local function catalog_endpoint(service_type)
    for _, service in ipairs(catalog) do
      if service.type == service_type then
        for _, endpoint in ipairs(service.endpoints or {}) do
          local same_interface = endpoint.interface == endpoint_interface
          local same_region = not region or endpoint.region == region or endpoint.region_id == region
          if same_interface and same_region then return strip_slash(endpoint.url) end
        end
      end
    end
    return nil
  end

  local function endpoint_for(service_type)
    authenticate()
    local endpoint = endpoints[service_type]
    if endpoint then return strip_slash(endpoint) end
    endpoint = catalog_endpoint(service_type)
    if endpoint then return endpoint end
    if service_type == "identity" then return identity_url end
    error("openstack: no endpoint for " .. service_type)
  end

  local function api_get(service_type, path_str, query, nil_on_404)
    local base_url = endpoint_for(service_type)
    local resp = http.get(base_url .. path_str .. build_query(query), {
      headers = service_headers(),
    })
    if nil_on_404 and resp.status == 404 then return nil end
    if resp.status < 200 or resp.status >= 300 then
      error("openstack: GET " .. path_str .. " HTTP " .. resp.status .. ": " .. resp.body)
    end
    if not resp.body or resp.body == "" then return {} end
    return json.parse(resp.body)
  end

  local function list(service_type, path_str, key, query)
    local data = api_get(service_type, path_str, query, false)
    return data[key] or {}
  end

  local function get(service_type, path_str, key)
    local data = api_get(service_type, path_str, nil, true)
    if not data then return nil end
    if key then return data[key] end
    return data
  end

  local c = {}

  function c:authenticate()
    return authenticate()
  end

  c.identity = {}

  function c.identity:list_projects(query)
    return list("identity", "/projects", "projects", query)
  end

  function c.identity:get_project(id)
    return get("identity", "/projects/" .. encode(id), "project")
  end

  function c.identity:list_users(query)
    return list("identity", "/users", "users", query)
  end

  function c.identity:get_user(id)
    return get("identity", "/users/" .. encode(id), "user")
  end

  function c.identity:list_regions(query)
    return list("identity", "/regions", "regions", query)
  end

  c.compute = {}

  function c.compute:list_servers(query)
    return list("compute", "/servers/detail", "servers", query)
  end

  function c.compute:get_server(id)
    return get("compute", "/servers/" .. encode(id), "server")
  end

  function c.compute:get_limits(query)
    local data = api_get("compute", "/limits", query, false)
    return data.limits or {}
  end

  function c.compute:get_quota(id, query)
    local data = api_get("compute", "/os-quota-sets/" .. encode(id) .. "/detail", query, false)
    return data.quota_set or {}
  end

  c.image = {}

  function c.image:list_images(query)
    return list("image", "/images", "images", query)
  end

  function c.image:get_image(id)
    return get("image", "/images/" .. encode(id))
  end

  c.network = {}

  function c.network:list_networks(query)
    return list("network", "/v2.0/networks", "networks", query)
  end

  function c.network:get_network(id)
    return get("network", "/v2.0/networks/" .. encode(id), "network")
  end

  function c.network:list_subnets(query)
    return list("network", "/v2.0/subnets", "subnets", query)
  end

  function c.network:get_subnet(id)
    return get("network", "/v2.0/subnets/" .. encode(id), "subnet")
  end

  function c.network:list_ports(query)
    return list("network", "/v2.0/ports", "ports", query)
  end

  function c.network:get_port(id)
    return get("network", "/v2.0/ports/" .. encode(id), "port")
  end

  function c.network:list_routers(query)
    return list("network", "/v2.0/routers", "routers", query)
  end

  function c.network:get_router(id)
    return get("network", "/v2.0/routers/" .. encode(id), "router")
  end

  function c.network:list_security_groups(query)
    return list("network", "/v2.0/security-groups", "security_groups", query)
  end

  function c.network:get_security_group(id)
    return get("network", "/v2.0/security-groups/" .. encode(id), "security_group")
  end

  function c.network:get_quota(id)
    local data = api_get("network", "/v2.0/quotas/" .. encode(id), nil, false)
    return data.quota or {}
  end

  return c
end

return M
