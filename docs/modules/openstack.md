---
category: Infrastructure
---

## assay.openstack

OpenStack inventory client with Keystone v3 password authentication, service-catalog discovery, and
GET-only identity, compute, image, network, and quota operations.

### Client

```lua
local openstack = require("assay.openstack")
local c = openstack.client("https://identity.example.com/v3", {
  username = env.get("OS_USERNAME"),
  password = env.get("OS_PASSWORD"),
  project_name = env.get("OS_PROJECT_NAME"),
  user_domain_name = env.get("OS_USER_DOMAIN_NAME") or "Default",
  project_domain_name = env.get("OS_PROJECT_DOMAIN_NAME") or "Default",
  region = env.get("OS_REGION_NAME") or "RegionOne",
  interface = env.get("OS_INTERFACE") or "public",
})
```

`openstack.client(auth_url, opts)` accepts:

- Password auth: `username`, `password`, and either `project_name` or `project_id`. Domain names
  default to `Default`; `user_domain_id` and `project_domain_id` are also supported.
- Existing-token auth: `token`. This skips the Keystone authentication request.
- Endpoint selection: `region` and `interface` (default `public`) select from the Keystone service
  catalog.
- Endpoint overrides: `endpoints = {identity=..., compute=..., image=..., network=...}`. These are
  useful with an existing token or a catalog that omits a service.
- Preloaded catalog: `catalog`, using the Keystone v3 service-catalog shape.

The client authenticates lazily on the first operation. `c:authenticate()` triggers authentication
explicitly and returns the scoped project ID plus service catalog; the token stays internal to the
client.

### Endpoint versions

Clouds disagree on whether the service catalog carries the API version. Some publish image as
`https://host/glance`, others as `https://host/glance/v2`. The client resolves an endpoint, then
appends the version the service expects — `v2` for image, `v2.0` for network — only when the
resolved URL does not already end in one. Both catalog conventions work, and an `endpoints` override
follows the same rule, so pinning `.../glance/v2` yields one version segment rather than two.

Compute needs no such default (the catalog entry is project-scoped and already versioned, e.g.
`/nova/v2.1/<project_id>`) and identity comes from the `auth_url` you pass.

### Identity

- `c.identity:list_projects(opts?)` -> `[project]`
- `c.identity:get_project(id)` -> `project`|nil
- `c.identity:list_users(opts?)` -> `[user]`
- `c.identity:get_user(id)` -> `user`|nil
- `c.identity:list_regions(opts?)` -> `[region]`

### Compute

- `c.compute:list_servers(opts?)` -> `[server]` — detailed server list.
- `c.compute:get_server(id)` -> `server`|nil
- `c.compute:get_limits(opts?)` -> `limits`
- `c.compute:get_quota(project_id, opts?)` -> `quota_set` — detailed project quota.

### Images

- `c.image:list_images(opts?)` -> `[image]`
- `c.image:get_image(id)` -> `image`|nil

### Network

- `c.network:list_networks(opts?)` -> `[network]`
- `c.network:get_network(id)` -> `network`|nil
- `c.network:list_subnets(opts?)` -> `[subnet]`
- `c.network:get_subnet(id)` -> `subnet`|nil
- `c.network:list_ports(opts?)` -> `[port]`
- `c.network:get_port(id)` -> `port`|nil
- `c.network:list_routers(opts?)` -> `[router]`
- `c.network:get_router(id)` -> `router`|nil
- `c.network:list_security_groups(opts?)` -> `[security_group]`
- `c.network:get_security_group(id)` -> `security_group`|nil
- `c.network:get_quota(project_id)` -> `quota`

List-method option keys are sent as query parameters. Array values produce repeated parameters, as
required by APIs such as OpenStack Networking's `fields` and multi-value filters.

### Readonly and approval modes

All inventory methods use HTTP GET, and the module publishes no create, update, delete, server
action, or console methods.

Keystone password authentication uses `POST /auth/tokens`. Assay readonly mode therefore blocks
password authentication. Use approval mode and approve that authentication operation, or supply a
pre-issued token plus endpoint overrides for a fully readonly execution path. The module does not
bypass or weaken Assay's HTTP operation gate.

```lua
local openstack = require("assay.openstack")
local c = openstack.client("https://identity.example.com/v3", {
  token = env.get("OS_TOKEN"),
  endpoints = {
    compute = env.get("OS_COMPUTE_ENDPOINT"),
    image = env.get("OS_IMAGE_ENDPOINT"),
    network = env.get("OS_NETWORK_ENDPOINT"),
  },
})

local result = {
  servers = c.compute:list_servers({ all_tenants = true }),
  images = c.image:list_images({ status = "active" }),
  networks = c.network:list_networks(),
}

return result
```
