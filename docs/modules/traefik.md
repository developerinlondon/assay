## assay.traefik

Traefik reverse proxy API. Routers, services, middlewares, entrypoints, TLS status.
Module-level functions (no client needed): `M.function(url, ...)`.

- `M.overview(url)` → overview — Get Traefik dashboard overview
- `M.version(url)` → version — Get Traefik version
- `M.entrypoints(url)` → [entrypoint] — List all entrypoints
- `M.entrypoint(url, name)` → entrypoint — Get entrypoint by name
- `M.http_routers(url)` → [router] — List HTTP routers
- `M.http_router(url, name)` → router — Get HTTP router by name
- `M.http_services(url)` → [service] — List HTTP services
- `M.http_service(url, name)` → service — Get HTTP service by name
- `M.http_middlewares(url)` → [middleware] — List HTTP middlewares
- `M.http_middleware(url, name)` → middleware — Get HTTP middleware by name
- `M.tcp_routers(url)` → [router] — List TCP routers
- `M.tcp_services(url)` → [service] — List TCP services
- `M.rawdata(url)` → data — Get raw Traefik configuration data
- `M.is_router_enabled(url, name)` → bool — Check if router status is "enabled"
- `M.router_has_tls(url, name)` → bool — Check if router has TLS configured
- `M.service_server_count(url, name)` → number — Count load balancer servers for service
- `M.healthy_routers(url)` → enabled, errored — Count enabled vs errored HTTP routers (two return values)

Example:
```lua
local traefik = require("assay.traefik")
local enabled, errored = traefik.healthy_routers("http://traefik:8080")
assert.eq(errored, 0, "Some routers have errors")
```
