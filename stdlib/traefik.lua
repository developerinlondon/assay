local M = {}

local function api_get(url, path_str)
  local base = url:gsub("/+$", "")
  local resp = http.get(base .. path_str, { headers = {} })

  if resp.status ~= 200 then
    error("traefik: GET " .. path_str .. " HTTP " .. resp.status .. ": " .. resp.body)
  end

  return json.parse(resp.body)
end

function M.overview(url)
  return api_get(url, "/api/overview")
end

function M.version(url)
  return api_get(url, "/api/version")
end

function M.entrypoints(url)
  return api_get(url, "/api/entrypoints")
end

function M.entrypoint(url, name)
  return api_get(url, "/api/entrypoints/" .. name)
end

function M.http_routers(url)
  return api_get(url, "/api/http/routers")
end

function M.http_router(url, name)
  return api_get(url, "/api/http/routers/" .. name)
end

function M.http_services(url)
  return api_get(url, "/api/http/services")
end

function M.http_service(url, name)
  return api_get(url, "/api/http/services/" .. name)
end

function M.http_middlewares(url)
  return api_get(url, "/api/http/middlewares")
end

function M.http_middleware(url, name)
  return api_get(url, "/api/http/middlewares/" .. name)
end

function M.tcp_routers(url)
  return api_get(url, "/api/tcp/routers")
end

function M.tcp_services(url)
  return api_get(url, "/api/tcp/services")
end

function M.rawdata(url)
  return api_get(url, "/api/rawdata")
end

function M.is_router_enabled(url, name)
  local router = M.http_router(url, name)
  return router.status == "enabled"
end

function M.router_has_tls(url, name)
  local router = M.http_router(url, name)
  return router.tls ~= nil
end

function M.service_server_count(url, name)
  local service = M.http_service(url, name)
  if not service.loadBalancer or not service.loadBalancer.servers then
    return 0
  end
  return #service.loadBalancer.servers
end

function M.healthy_routers(url)
  local routers = M.http_routers(url)
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

return M
