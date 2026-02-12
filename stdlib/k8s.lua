local M = {}

local function api_base()
  local host = env.get("KUBERNETES_SERVICE_HOST")
  local port = env.get("KUBERNETES_SERVICE_PORT") or "443"
  if not host then
    error("k8s: not running in a Kubernetes pod (KUBERNETES_SERVICE_HOST not set)")
  end
  return "https://" .. host .. ":" .. port
end

local function sa_token()
  return fs.read("/var/run/secrets/kubernetes.io/serviceaccount/token")
end

local function auth_headers(token)
  return { Authorization = "Bearer " .. (token or sa_token()) }
end

local RESOURCE_PATHS = {
  pod                   = { api = "/api/v1",                              plural = "pods" },
  service               = { api = "/api/v1",                              plural = "services" },
  secret                = { api = "/api/v1",                              plural = "secrets" },
  configmap             = { api = "/api/v1",                              plural = "configmaps" },
  endpoints             = { api = "/api/v1",                              plural = "endpoints" },
  serviceaccount        = { api = "/api/v1",                              plural = "serviceaccounts" },
  persistentvolumeclaim = { api = "/api/v1",                              plural = "persistentvolumeclaims" },
  pvc                   = { api = "/api/v1",                              plural = "persistentvolumeclaims" },
  limitrange            = { api = "/api/v1",                              plural = "limitranges" },
  resourcequota         = { api = "/api/v1",                              plural = "resourcequotas" },
  event                 = { api = "/api/v1",                              plural = "events" },
  namespace             = { api = "/api/v1",                              plural = "namespaces", cluster = true },
  node                  = { api = "/api/v1",                              plural = "nodes", cluster = true },
  persistentvolume      = { api = "/api/v1",                              plural = "persistentvolumes", cluster = true },
  pv                    = { api = "/api/v1",                              plural = "persistentvolumes", cluster = true },
  deployment            = { api = "/apis/apps/v1",                        plural = "deployments" },
  statefulset           = { api = "/apis/apps/v1",                        plural = "statefulsets" },
  daemonset             = { api = "/apis/apps/v1",                        plural = "daemonsets" },
  replicaset            = { api = "/apis/apps/v1",                        plural = "replicasets" },
  job                   = { api = "/apis/batch/v1",                       plural = "jobs" },
  cronjob               = { api = "/apis/batch/v1",                       plural = "cronjobs" },
  ingress               = { api = "/apis/networking.k8s.io/v1",           plural = "ingresses" },
  ingressclass          = { api = "/apis/networking.k8s.io/v1",           plural = "ingressclasses", cluster = true },
  networkpolicy         = { api = "/apis/networking.k8s.io/v1",           plural = "networkpolicies" },
  storageclass          = { api = "/apis/storage.k8s.io/v1",             plural = "storageclasses", cluster = true },
  role                  = { api = "/apis/rbac.authorization.k8s.io/v1",   plural = "roles" },
  rolebinding           = { api = "/apis/rbac.authorization.k8s.io/v1",   plural = "rolebindings" },
  clusterrole           = { api = "/apis/rbac.authorization.k8s.io/v1",   plural = "clusterroles", cluster = true },
  clusterrolebinding    = { api = "/apis/rbac.authorization.k8s.io/v1",   plural = "clusterrolebindings", cluster = true },
  hpa                   = { api = "/apis/autoscaling/v2",                 plural = "horizontalpodautoscalers" },
  poddisruptionbudget   = { api = "/apis/policy/v1",                      plural = "poddisruptionbudgets" },
  pdb                   = { api = "/apis/policy/v1",                      plural = "poddisruptionbudgets" },
}

function M.register_crd(kind, api_group, version, plural, cluster_scoped)
  RESOURCE_PATHS[kind:lower()] = {
    api = "/apis/" .. api_group .. "/" .. version,
    plural = plural,
    cluster = cluster_scoped or false,
  }
end

function M._resource_path(namespace, kind, name)
  local info = RESOURCE_PATHS[kind:lower()]
  if not info then
    error("k8s: unknown resource kind '" .. kind .. "'. Use k8s.register_crd() for custom resources or k8s.get() with a raw path.")
  end
  if info.cluster then
    return info.api .. "/" .. info.plural .. "/" .. name
  end
  return info.api .. "/namespaces/" .. namespace .. "/" .. info.plural .. "/" .. name
end

function M._list_path(namespace, kind)
  local info = RESOURCE_PATHS[kind:lower()]
  if not info then
    error("k8s: unknown resource kind '" .. kind .. "'. Use k8s.register_crd() for custom resources or k8s.get() with a raw path.")
  end
  if info.cluster then
    return info.api .. "/" .. info.plural
  end
  return info.api .. "/namespaces/" .. namespace .. "/" .. info.plural
end

function M.get(path, opts)
  opts = opts or {}
  local url = (opts.base_url or api_base()) .. path
  local resp = http.get(url, {
    headers = auth_headers(opts.token),
  })
  if resp.status ~= 200 then
    error("k8s.get: HTTP " .. resp.status .. " " .. path .. ": " .. resp.body)
  end
  return json.parse(resp.body)
end

function M.post(path, body, opts)
  opts = opts or {}
  local url = (opts.base_url or api_base()) .. path
  local resp = http.post(url, body, {
    headers = auth_headers(opts.token),
  })
  if resp.status < 200 or resp.status >= 300 then
    error("k8s.post: HTTP " .. resp.status .. " " .. path .. ": " .. resp.body)
  end
  return json.parse(resp.body)
end

function M.put(path, body, opts)
  opts = opts or {}
  local url = (opts.base_url or api_base()) .. path
  local resp = http.put(url, body, {
    headers = auth_headers(opts.token),
  })
  if resp.status < 200 or resp.status >= 300 then
    error("k8s.put: HTTP " .. resp.status .. " " .. path .. ": " .. resp.body)
  end
  return json.parse(resp.body)
end

function M.patch(path, body, opts)
  opts = opts or {}
  local url = (opts.base_url or api_base()) .. path
  local hdrs = auth_headers(opts.token)
  hdrs["Content-Type"] = opts.content_type or "application/merge-patch+json"
  local encoded = type(body) == "table" and json.encode(body) or body
  local resp = http.patch(url, encoded, {
    headers = hdrs,
  })
  if resp.status < 200 or resp.status >= 300 then
    error("k8s.patch: HTTP " .. resp.status .. " " .. path .. ": " .. resp.body)
  end
  return json.parse(resp.body)
end

function M.delete(path, opts)
  opts = opts or {}
  local url = (opts.base_url or api_base()) .. path
  local resp = http.delete(url, {
    headers = auth_headers(opts.token),
  })
  if resp.status < 200 or resp.status >= 300 then
    error("k8s.delete: HTTP " .. resp.status .. " " .. path .. ": " .. resp.body)
  end
end

function M.get_resource(namespace, kind, name, opts)
  return M.get(M._resource_path(namespace, kind, name), opts)
end

function M.list(namespace, kind, opts)
  opts = opts or {}
  local path = M._list_path(namespace, kind)
  local params = {}
  if opts.label_selector then params[#params + 1] = "labelSelector=" .. opts.label_selector end
  if opts.field_selector then params[#params + 1] = "fieldSelector=" .. opts.field_selector end
  if opts.limit then params[#params + 1] = "limit=" .. opts.limit end
  if #params > 0 then
    path = path .. "?" .. table.concat(params, "&")
  end
  return M.get(path, opts)
end

function M.create(namespace, kind, body, opts)
  return M.post(M._list_path(namespace, kind), body, opts)
end

function M.update(namespace, kind, name, body, opts)
  return M.put(M._resource_path(namespace, kind, name), body, opts)
end

function M.patch_resource(namespace, kind, name, body, opts)
  return M.patch(M._resource_path(namespace, kind, name), body, opts)
end

function M.delete_resource(namespace, kind, name, opts)
  return M.delete(M._resource_path(namespace, kind, name), opts)
end

function M.exists(namespace, kind, name, opts)
  opts = opts or {}
  local api_path = M._resource_path(namespace, kind, name)
  local url = (opts.base_url or api_base()) .. api_path
  local resp = http.get(url, {
    headers = auth_headers(opts.token),
  })
  return resp.status == 200
end

function M.get_secret(namespace, name, opts)
  local data = M.get_resource(namespace, "secret", name, opts)
  local decoded = {}
  if data.data then
    for k, v in pairs(data.data) do
      decoded[k] = base64.decode(v)
    end
  end
  return decoded
end

function M.get_configmap(namespace, name, opts)
  local data = M.get_resource(namespace, "configmap", name, opts)
  return data.data or {}
end

function M.list_pods(namespace, opts)
  return M.list(namespace, "pod", opts)
end

function M.list_events(namespace, opts)
  return M.list(namespace, "event", opts)
end

function M.pod_status(namespace, opts)
  local pod_list = M.list_pods(namespace, opts)
  local counts = { running = 0, pending = 0, succeeded = 0, failed = 0, unknown = 0, total = 0 }
  for _, pod in ipairs(pod_list.items or {}) do
    counts.total = counts.total + 1
    local phase = (pod.status and pod.status.phase or "Unknown"):lower()
    if counts[phase] then
      counts[phase] = counts[phase] + 1
    else
      counts.unknown = counts.unknown + 1
    end
  end
  return counts
end

function M.is_ready(namespace, kind, name, opts)
  local resource = M.get_resource(namespace, kind, name, opts)
  local kind_lower = kind:lower()

  if kind_lower == "deployment" or kind_lower == "statefulset" then
    local status = resource.status or {}
    local desired = status.replicas or 0
    local ready = status.readyReplicas or 0
    return ready >= desired and desired > 0
  end

  if kind_lower == "daemonset" then
    local status = resource.status or {}
    local desired = status.desiredNumberScheduled or 0
    local ready = status.numberReady or 0
    return ready >= desired and desired > 0
  end

  if kind_lower == "job" then
    local status = resource.status or {}
    return (status.succeeded or 0) >= 1
  end

  if kind_lower == "node" then
    local conditions = (resource.status or {}).conditions or {}
    for _, cond in ipairs(conditions) do
      if cond.type == "Ready" then
        return cond.status == "True"
      end
    end
    return false
  end

  local conditions = (resource.status or {}).conditions or {}
  for _, cond in ipairs(conditions) do
    if cond.type == "Ready" then
      return cond.status == "True"
    end
  end

  local phase = (resource.status or {}).phase
  if phase then
    return phase == "Active" or phase == "Running" or phase == "Bound" or phase == "Ready"
  end

  return false
end

function M.wait_ready(namespace, kind, name, timeout_secs, opts)
  timeout_secs = timeout_secs or 60
  local interval = 2
  local elapsed = 0
  while elapsed < timeout_secs do
    local ok, ready = pcall(M.is_ready, namespace, kind, name, opts)
    if ok and ready then
      return true
    end
    sleep(interval)
    elapsed = elapsed + interval
  end
  error("k8s.wait_ready: " .. kind .. "/" .. name .. " not ready after " .. timeout_secs .. "s")
end

function M.service_endpoints(namespace, name, opts)
  local ep = M.get_resource(namespace, "endpoints", name, opts)
  local ips = {}
  for _, subset in ipairs(ep.subsets or {}) do
    for _, addr in ipairs(subset.addresses or {}) do
      ips[#ips + 1] = addr.ip
    end
  end
  return ips
end

function M.logs(namespace, pod_name, opts)
  opts = opts or {}
  local path = "/api/v1/namespaces/" .. namespace .. "/pods/" .. pod_name .. "/log"
  local params = {}
  if opts.tail then params[#params + 1] = "tailLines=" .. opts.tail end
  if opts.container then params[#params + 1] = "container=" .. opts.container end
  if opts.previous then params[#params + 1] = "previous=true" end
  if opts.since then params[#params + 1] = "sinceSeconds=" .. opts.since end
  if #params > 0 then
    path = path .. "?" .. table.concat(params, "&")
  end
  local url = (opts.base_url or api_base()) .. path
  local resp = http.get(url, {
    headers = auth_headers(opts.token),
  })
  if resp.status ~= 200 then
    error("k8s.logs: HTTP " .. resp.status .. " " .. path .. ": " .. resp.body)
  end
  return resp.body
end

function M.rollout_status(namespace, name, opts)
  local deploy = M.get_resource(namespace, "deployment", name, opts)
  local status = deploy.status or {}
  local spec = deploy.spec or {}
  return {
    desired = spec.replicas or 0,
    updated = status.updatedReplicas or 0,
    ready = status.readyReplicas or 0,
    available = status.availableReplicas or 0,
    unavailable = status.unavailableReplicas or 0,
    complete = (status.updatedReplicas or 0) == (spec.replicas or 0)
      and (status.readyReplicas or 0) == (spec.replicas or 0),
  }
end

function M.node_status(opts)
  local nodes = M.get("/api/v1/nodes", opts)
  local result = {}
  for _, node in ipairs(nodes.items or {}) do
    local ready = false
    for _, cond in ipairs((node.status or {}).conditions or {}) do
      if cond.type == "Ready" then
        ready = cond.status == "True"
      end
    end
    result[#result + 1] = {
      name = node.metadata.name,
      ready = ready,
      roles = {},
      capacity = (node.status or {}).capacity or {},
      allocatable = (node.status or {}).allocatable or {},
    }
    for label, _ in pairs(node.metadata.labels or {}) do
      local role = label:match("^node%-role%.kubernetes%.io/(.+)$")
      if role then
        result[#result].roles[#result[#result].roles + 1] = role
      end
    end
  end
  return result
end

function M.namespace_exists(name, opts)
  return M.exists(nil, "namespace", name, opts)
end

function M.events_for(namespace, kind, name, opts)
  return M.list(namespace, "event", {
    field_selector = "involvedObject.kind=" .. kind .. ",involvedObject.name=" .. name,
    base_url = (opts or {}).base_url,
    token = (opts or {}).token,
  })
end

return M
