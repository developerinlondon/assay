## assay.traefik

Traefik reverse proxy API. Routers, services, middlewares, entrypoints, TLS status.
Client: `traefik.client(url)`.

- `c.info:overview()` → overview — Get Traefik dashboard overview
- `c.info:version()` → version — Get Traefik version
- `c.info:rawdata()` → data — Get raw Traefik configuration data
- `c.entrypoints:list()` → [entrypoint] — List all entrypoints
- `c.entrypoints:get(name)` → entrypoint — Get entrypoint by name
- `c.routers:list()` → [router] — List HTTP routers
- `c.routers:get(name)` → router — Get HTTP router by name
- `c.routers:is_enabled(name)` → bool — Check if router status is "enabled"
- `c.routers:has_tls(name)` → bool — Check if router has TLS configured
- `c.routers:healthy()` → enabled, errored — Count enabled vs errored HTTP routers (two return values)
- `c.services:list()` → [service] — List HTTP services
- `c.services:get(name)` → service — Get HTTP service by name
- `c.services:server_count(name)` → number — Count load balancer servers for service
- `c.middlewares:list()` → [middleware] — List HTTP middlewares
- `c.middlewares:get(name)` → middleware — Get HTTP middleware by name
- `c.tcp:routers()` → [router] — List TCP routers
- `c.tcp:services()` → [service] — List TCP services

Example:
```lua
local traefik = require("assay.traefik")
local c = traefik.client("http://traefik:8080")
local enabled, errored = c.routers:healthy()
assert.eq(errored, 0, "Some routers have errors")
```
