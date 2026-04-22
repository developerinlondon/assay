--- @module assay.traefik
--- @description Traefik reverse proxy API. Routers, services, middlewares, entrypoints, TLS status.
--- @keywords traefik, proxy, routers, services, middlewares, entrypoints, loadbalancer, http, tcp, tls, configuration, dashboard, ingress
--- @quickref c.entrypoints:list() -> [entrypoint] | List entrypoints
--- @quickref c.entrypoints:get(name) -> entrypoint | Get entrypoint by name
--- @quickref c.routers:list() -> [router] | List HTTP routers
--- @quickref c.routers:get(name) -> router | Get HTTP router by name
--- @quickref c.routers:is_enabled(name) -> bool | Check if router is enabled
--- @quickref c.routers:has_tls(name) -> bool | Check if router has TLS
--- @quickref c.routers:healthy() -> enabled, errored | Count healthy vs errored routers
--- @quickref c.services:list() -> [service] | List HTTP services
--- @quickref c.services:get(name) -> service | Get HTTP service by name
--- @quickref c.services:server_count(name) -> number | Count load balancer servers
--- @quickref c.middlewares:list() -> [middleware] | List HTTP middlewares
--- @quickref c.middlewares:get(name) -> middleware | Get HTTP middleware by name
--- @quickref c.tcp:routers() -> [router] | List TCP routers
--- @quickref c.tcp:services() -> [service] | List TCP services
--- @quickref c.info:overview() -> overview | Get Traefik dashboard overview
--- @quickref c.info:version() -> version | Get Traefik version
--- @quickref c.info:rawdata() -> data | Get raw configuration data

local M = {}

function M.client(url)
  local base_url = url:gsub("/+$", "")

  local function api_get(path_str)
    local resp = http.get(base_url .. path_str, { headers = {} })
    if resp.status ~= 200 then
      error("traefik: GET " .. path_str .. " HTTP " .. resp.status .. ": " .. resp.body)
    end
    return json.parse(resp.body)
  end

  local c = {}

  -- ===== Info =====

  c.info = {}

  function c.info:overview()
    return api_get("/api/overview")
  end

  function c.info:version()
    return api_get("/api/version")
  end

  function c.info:rawdata()
    return api_get("/api/rawdata")
  end

  -- ===== Entrypoints =====

  c.entrypoints = {}

  function c.entrypoints:list()
    return api_get("/api/entrypoints")
  end

  function c.entrypoints:get(name)
    return api_get("/api/entrypoints/" .. name)
  end

  -- ===== Routers (HTTP) =====

  c.routers = {}

  function c.routers:list()
    return api_get("/api/http/routers")
  end

  function c.routers:get(name)
    return api_get("/api/http/routers/" .. name)
  end

  function c.routers:is_enabled(name)
    local router = c.routers:get(name)
    return router.status == "enabled"
  end

  function c.routers:has_tls(name)
    local router = c.routers:get(name)
    return router.tls ~= nil
  end

  function c.routers:healthy()
    local routers = c.routers:list()
    local enabled = 0
    local errored = 0
    for _, router in ipairs(routers) do
      if router.status == "enabled" then
        enabled = enabled + 1
      else
        errored = errored + 1
      end
    end
    return enabled, errored
  end

  -- ===== Services (HTTP) =====

  c.services = {}

  function c.services:list()
    return api_get("/api/http/services")
  end

  function c.services:get(name)
    return api_get("/api/http/services/" .. name)
  end

  function c.services:server_count(name)
    local service = c.services:get(name)
    if not service.loadBalancer or not service.loadBalancer.servers then
      return 0
    end
    return #service.loadBalancer.servers
  end

  -- ===== Middlewares (HTTP) =====

  c.middlewares = {}

  function c.middlewares:list()
    return api_get("/api/http/middlewares")
  end

  function c.middlewares:get(name)
    return api_get("/api/http/middlewares/" .. name)
  end

  -- ===== TCP =====

  c.tcp = {}

  function c.tcp:routers()
    return api_get("/api/tcp/routers")
  end

  function c.tcp:services()
    return api_get("/api/tcp/services")
  end

  return c
end

return M
