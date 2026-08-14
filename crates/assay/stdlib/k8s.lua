--- @module assay.k8s
--- @description Kubernetes API client for Kubernetes clusters. 30+ resource types, CRDs, readiness checks, pod logs, rollouts. Multi-cluster via kubeconfig contexts (pass opts.context on any call, or k8s.use_context(name)); EKS aws exec-plugin auth is minted in-process.
--- @category kubernetes
--- @icon kubernetes
--- @keywords kubernetes, k8s, pods, deployments, services, secrets, configmaps, namespaces, crd, custom-resources, rbac, events, logs, rollout, nodes, readiness, wait, deploy, deployment, kubeconfig, context, multi-cluster, eks
--- @env KUBERNETES_SERVICE_HOST, KUBERNETES_SERVICE_PORT, KUBECONFIG, ASSAY_K8S_CONTEXT, HOME
--- @quickref M.contexts(opts?) -> [name], current | List kubeconfig context names + current-context
--- @quickref M.use_context(name) -> nil | Set the default kubeconfig context for all calls
--- @quickref M.register_crd(kind, api_group, version, plural, cluster_scoped?) -> nil | Register custom resource
--- @quickref M.get(path, opts?) -> resource | GET any K8s API path
--- @quickref M.post(path, body, opts?) -> resource | POST to any K8s API path
--- @quickref M.put(path, body, opts?) -> resource | PUT to any K8s API path
--- @quickref M.patch(path, body, opts?) -> resource | PATCH any K8s API path
--- @quickref M.delete(path, opts?) -> nil | DELETE any K8s API path
--- @quickref M.resources:get(namespace, kind, name, opts?) -> resource | Get resource by kind and name
--- @quickref M.resources:list(namespace, kind, opts?) -> {items} | List resources by kind
--- @quickref M.resources:create(namespace, kind, body, opts?) -> resource | Create resource
--- @quickref M.resources:update(namespace, kind, name, body, opts?) -> resource | Update resource
--- @quickref M.resources:patch(namespace, kind, name, body, opts?) -> resource | Patch resource
--- @quickref M.resources:delete(namespace, kind, name, opts?) -> nil | Delete resource
--- @quickref M.resources:exists(namespace, kind, name, opts?) -> bool | Check if resource exists
--- @quickref M.resources:is_ready(namespace, kind, name, opts?) -> bool | Check if resource is ready
--- @quickref M.resources:wait_ready(namespace, kind, name, timeout_secs?, opts?) -> true | Wait for readiness
--- @quickref M.secrets:get(namespace, name, opts?) -> {key=value} | Get decoded secret data
--- @quickref M.configmaps:get(namespace, name, opts?) -> {key=value} | Get ConfigMap data
--- @quickref M.pods:list(namespace, opts?) -> {items} | List pods in namespace
--- @quickref M.pods:status(namespace, opts?) -> {running, pending, failed, total} | Get pod status counts
--- @quickref M.pods:logs(namespace, pod_name, opts?) -> string | Get pod logs
--- @quickref M.pods:exec(namespace, pod_name, command, opts?) -> {stdout, stderr, exit_code} | Exec a command in a pod over WebSocket (gated)
--- @quickref M.services:endpoints(namespace, name, opts?) -> [ip] | Get service endpoint IPs
--- @quickref M.deployments:rollout_status(namespace, name, opts?) -> {desired, ready, complete} | Get deployment rollout
--- @quickref M.nodes:status(opts?) -> [{name, ready, roles, capacity}] | Get node statuses
--- @quickref M.namespaces:exists(name, opts?) -> bool | Check if namespace exists
--- @quickref M.events:for_resource(namespace, kind, name, opts?) -> {items} | Get events for resource
--- @quickref M.events:list(namespace, opts?) -> {items} | List events in namespace

local M = {}

local _http = nil
local function get_http()
  if not _http then
    local ca_path = "/var/run/secrets/kubernetes.io/serviceaccount/ca.crt"
    local ok, client = pcall(http.client, { ca_cert_file = ca_path })
    if ok then
      _http = client
    else
      _http = http.client({})
    end
  end
  return _http
end

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

-- ===== kubeconfig contexts (multi-cluster) =====
--
-- Cluster targeting, in priority order:
--   1. opts.base_url (+ opts.token)                — raw, unchanged behavior
--   2. opts.context                                — a kubeconfig context, per call
--   3. M.use_context(name) / ASSAY_K8S_CONTEXT env — a default kubeconfig context
--   4. in-cluster ServiceAccount                   — the pod's own cluster (default)
-- Contexts are read from opts.kubeconfig, $KUBECONFIG, or ~/.kube/config. User
-- auth supported: a static `token`, or the aws `eks get-token` exec plugin —
-- which is recognized and minted IN-PROCESS via assay.aws.eks (subprocesses are
-- blocked in readonly mode, so the plugin command is never actually run).

local EKS_TOKEN_TTL_SECS = 600

M._default_context = nil
local _kubecfg = nil -- { path, doc } parse cache
local _ctx_cache = {} -- context name -> { base_url, client, static_token, exec, token, token_expires_at }

function M.use_context(name)
  M._default_context = name
end

local function kubeconfig_path(opts)
  return (opts and opts.kubeconfig) or env.get("KUBECONFIG")
    or ((env.get("HOME") or "") .. "/.kube/config")
end

local function load_kubeconfig(opts)
  local path = kubeconfig_path(opts)
  if _kubecfg and _kubecfg.path == path then return _kubecfg.doc end
  local ok, text = pcall(fs.read, path)
  if not ok then
    error("k8s: cannot read kubeconfig at " .. path .. " (set KUBECONFIG or pass opts.kubeconfig)")
  end
  local doc = yaml.parse(text)
  _kubecfg = { path = path, doc = doc }
  return doc
end

--- List the kubeconfig's context names and its current-context.
function M.contexts(opts)
  local doc = load_kubeconfig(opts)
  local names = {}
  for _, c in ipairs(doc.contexts or {}) do names[#names + 1] = c.name end
  return names, doc["current-context"]
end

local function find_named(list, name)
  for _, e in ipairs(list or {}) do
    if e.name == name then return e end
  end
end

-- Recognize the aws `eks get-token` exec plugin and lift what we need to mint
-- the token in-process: cluster name, region, role, profile (from args or the
-- exec env block).
local function parse_aws_exec(exec)
  local cmd = exec.command or ""
  if not (cmd == "aws" or cmd:match("/aws$")) then return nil end
  local args = exec.args or {}
  local seen_get_token = false
  local flags = {}
  local i = 1
  while i <= #args do
    local a = args[i]
    if a == "get-token" then
      seen_get_token = true
    elseif a:match("^%-%-") and args[i + 1] and not args[i + 1]:match("^%-%-") then
      flags[a] = args[i + 1]
      i = i + 1
    end
    i = i + 1
  end
  if not seen_get_token or not flags["--cluster-name"] then return nil end
  local from_env = {}
  for _, e in ipairs(exec.env or {}) do
    from_env[e.name] = e.value
  end
  return {
    cluster_name = flags["--cluster-name"],
    region = flags["--region"] or from_env.AWS_REGION or from_env.AWS_DEFAULT_REGION,
    role_arn = flags["--role-arn"],
    profile = flags["--profile"] or from_env.AWS_PROFILE,
    access_key = from_env.AWS_ACCESS_KEY_ID,
    secret_key = from_env.AWS_SECRET_ACCESS_KEY,
    session_token = from_env.AWS_SESSION_TOKEN,
  }
end

local function resolve_context(name, opts)
  local cached = _ctx_cache[name]
  if not cached then
    local doc = load_kubeconfig(opts)
    local ctx = find_named(doc.contexts, name)
    if not ctx then
      error("k8s: context '" .. name .. "' not found in kubeconfig " .. kubeconfig_path(opts))
    end
    local cluster_entry = find_named(doc.clusters, ctx.context.cluster)
    local user_entry = find_named(doc.users, ctx.context.user)
    if not cluster_entry or not user_entry then
      error("k8s: kubeconfig context '" .. name .. "' references a missing cluster or user")
    end
    local cluster = cluster_entry.cluster
    local user = user_entry.user or {}

    local client_opts = {}
    if cluster["certificate-authority-data"] then
      client_opts.ca_cert = base64.decode(cluster["certificate-authority-data"])
    elseif cluster["certificate-authority"] then
      client_opts.ca_cert_file = cluster["certificate-authority"]
    end
    local ok, client = pcall(http.client, client_opts)
    if not ok then client = http.client({}) end

    if user["client-certificate-data"] or user["client-certificate"] then
      error("k8s: context '" .. name .. "' uses client-certificate auth, which is not supported")
    end
    local exec_cfg = nil
    if not user.token and user.exec then
      exec_cfg = parse_aws_exec(user.exec)
      if not exec_cfg then
        error(
          "k8s: context '" .. name .. "' uses an unsupported exec credential plugin ("
            .. tostring(user.exec.command)
            .. ") — only static tokens and the aws eks get-token plugin (minted in-process) are supported"
        )
      end
    elseif not user.token then
      error("k8s: context '" .. name .. "' has no supported auth (token or aws eks get-token exec)")
    end

    cached = {
      base_url = cluster.server:gsub("/+$", ""),
      client = client,
      static_token = user.token,
      exec = exec_cfg,
      token = nil,
      token_expires_at = 0,
    }
    _ctx_cache[name] = cached
  end

  if cached.static_token then
    cached.token = cached.static_token
  elseif not cached.token or os.time() > cached.token_expires_at then
    local eks = require("assay.aws.eks")
    cached.token = eks.get_token(cached.exec.cluster_name, {
      region = cached.exec.region,
      role_arn = cached.exec.role_arn,
      profile = cached.exec.profile,
      access_key = cached.exec.access_key,
      secret_key = cached.exec.secret_key,
      session_token = cached.exec.session_token,
    })
    cached.token_expires_at = os.time() + EKS_TOKEN_TTL_SECS - 60
  end
  return cached
end

-- Resolve where a call goes: {base, client, token}. See the priority order in
-- the section comment above. Fully backward compatible — with no context
-- configured, behavior is identical to the pre-context module.
local function target(opts)
  opts = opts or {}
  if opts.base_url then
    return { base = opts.base_url, client = get_http(), token = opts.token }
  end
  local ctx = opts.context or M._default_context or env.get("ASSAY_K8S_CONTEXT")
  if ctx then
    local t = resolve_context(ctx, opts)
    return { base = t.base_url, client = t.client, token = opts.token or t.token }
  end
  return { base = api_base(), client = get_http(), token = opts.token }
end

-- Percent-encode a value for a URL query component. Keeps the RFC 3986
-- unreserved set literal and encodes everything else.
local function url_encode(s)
  return (tostring(s):gsub("[^%w%-%._~]", function(c)
    return string.format("%%%02X", string.byte(c))
  end))
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

-- ===== Raw HTTP verbs (top-level) =====

function M.get(path, opts)
  local t = target(opts)
  local resp = t.client:get(t.base .. path, {
    headers = auth_headers(t.token),
  })
  if resp.status ~= 200 then
    error("k8s.get: HTTP " .. resp.status .. " " .. path .. ": " .. resp.body)
  end
  return json.parse(resp.body)
end

function M.post(path, body, opts)
  local t = target(opts)
  local resp = t.client:post(t.base .. path, body, {
    headers = auth_headers(t.token),
  })
  if resp.status < 200 or resp.status >= 300 then
    error("k8s.post: HTTP " .. resp.status .. " " .. path .. ": " .. resp.body)
  end
  return json.parse(resp.body)
end

function M.put(path, body, opts)
  local t = target(opts)
  local resp = t.client:put(t.base .. path, body, {
    headers = auth_headers(t.token),
  })
  if resp.status < 200 or resp.status >= 300 then
    error("k8s.put: HTTP " .. resp.status .. " " .. path .. ": " .. resp.body)
  end
  return json.parse(resp.body)
end

function M.patch(path, body, opts)
  opts = opts or {}
  local t = target(opts)
  local hdrs = auth_headers(t.token)
  hdrs["Content-Type"] = opts.content_type or "application/merge-patch+json"
  local encoded = type(body) == "table" and json.encode(body) or body
  local resp = t.client:patch(t.base .. path, encoded, {
    headers = hdrs,
  })
  if resp.status < 200 or resp.status >= 300 then
    error("k8s.patch: HTTP " .. resp.status .. " " .. path .. ": " .. resp.body)
  end
  return json.parse(resp.body)
end

function M.delete(path, opts)
  local t = target(opts)
  local resp = t.client:delete(t.base .. path, {
    headers = auth_headers(t.token),
  })
  if resp.status < 200 or resp.status >= 300 then
    error("k8s.delete: HTTP " .. resp.status .. " " .. path .. ": " .. resp.body)
  end
end

-- ===== Resources sub-object =====

M.resources = {}

function M.resources:get(namespace, kind, name, opts)
  return M.get(M._resource_path(namespace, kind, name), opts)
end

function M.resources:list(namespace, kind, opts)
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

function M.resources:create(namespace, kind, body, opts)
  return M.post(M._list_path(namespace, kind), body, opts)
end

function M.resources:update(namespace, kind, name, body, opts)
  return M.put(M._resource_path(namespace, kind, name), body, opts)
end

function M.resources:patch(namespace, kind, name, body, opts)
  return M.patch(M._resource_path(namespace, kind, name), body, opts)
end

function M.resources:delete(namespace, kind, name, opts)
  return M.delete(M._resource_path(namespace, kind, name), opts)
end

function M.resources:exists(namespace, kind, name, opts)
  local api_path = M._resource_path(namespace, kind, name)
  local t = target(opts)
  local resp = t.client:get(t.base .. api_path, {
    headers = auth_headers(t.token),
  })
  return resp.status == 200
end

function M.resources:is_ready(namespace, kind, name, opts)
  local resource = M.resources:get(namespace, kind, name, opts)
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

function M.resources:wait_ready(namespace, kind, name, timeout_secs, opts)
  timeout_secs = timeout_secs or 60
  local interval = 2
  local elapsed = 0
  while elapsed < timeout_secs do
    local ok, ready = pcall(M.resources.is_ready, M.resources, namespace, kind, name, opts)
    if ok and ready then
      return true
    end
    sleep(interval)
    elapsed = elapsed + interval
  end
  error("k8s.wait_ready: " .. kind .. "/" .. name .. " not ready after " .. timeout_secs .. "s")
end

-- ===== Secrets sub-object =====

M.secrets = {}

function M.secrets:get(namespace, name, opts)
  local data = M.resources:get(namespace, "secret", name, opts)
  local decoded = {}
  if data.data then
    for k, v in pairs(data.data) do
      decoded[k] = base64.decode(v)
    end
  end
  return decoded
end

-- ===== ConfigMaps sub-object =====

M.configmaps = {}

function M.configmaps:get(namespace, name, opts)
  local data = M.resources:get(namespace, "configmap", name, opts)
  return data.data or {}
end

-- ===== Pods sub-object =====

M.pods = {}

function M.pods:list(namespace, opts)
  return M.resources:list(namespace, "pod", opts)
end

function M.pods:status(namespace, opts)
  local pod_list = M.pods:list(namespace, opts)
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

function M.pods:logs(namespace, pod_name, opts)
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
  local t = target(opts)
  local resp = t.client:get(t.base .. path, {
    headers = auth_headers(t.token),
  })
  if resp.status ~= 200 then
    error("k8s.logs: HTTP " .. resp.status .. " " .. path .. ": " .. resp.body)
  end
  return resp.body
end

-- Map a Kubernetes v1.Status (sent on the exec error channel when the process
-- exits) to a numeric exit code. Success is 0. A Failure carries the code in
-- details.causes[] with reason "ExitCode"; without that cause a Failure
-- defaults to 1.
function M._exit_code_from_status(status)
  if type(status) ~= "table" then return 0 end
  if status.status == "Success" then return 0 end
  if status.status == "Failure" then
    local details = status.details
    if details and details.causes then
      for _, cause in ipairs(details.causes) do
        if cause.reason == "ExitCode" then
          return tonumber(cause.message) or 1
        end
      end
    end
    return 1
  end
  return 0
end

-- Demultiplex Kubernetes exec channel frames. Each frame's first byte is the
-- channel: 1=stdout, 2=stderr, 3=error/status (a v1.Status JSON with the exit
-- code). Channel 0 (stdin) and 4 (resize) are ignored on read.
function M._demux_exec_frames(frames)
  local stdout, stderr = {}, {}
  local exit_code = 0
  for _, frame in ipairs(frames) do
    if #frame >= 1 then
      local channel = string.byte(frame, 1)
      local payload = string.sub(frame, 2)
      if channel == 1 then
        stdout[#stdout + 1] = payload
      elseif channel == 2 then
        stderr[#stderr + 1] = payload
      elseif channel == 3 and #payload > 0 then
        local ok, status = pcall(json.parse, payload)
        if ok then exit_code = M._exit_code_from_status(status) end
      end
    end
  end
  return {
    stdout = table.concat(stdout),
    stderr = table.concat(stderr),
    exit_code = exit_code,
  }
end

-- Exec a command in a pod over the Kubernetes streaming exec endpoint using the
-- v4.channel.k8s.io WebSocket subprotocol. `command` is a string (single argv
-- element) or an array of strings. `opts`: {container, stdin, tty, timeout_secs,
-- token, base_url, insecure}. Returns {stdout, stderr, exit_code}. Routes
-- through the gated `ws.connect`, so read-only mode blocks it and approval mode
-- suspends it. `insecure` defaults to true (cluster API servers present a
-- cluster-CA cert the runtime does not trust by default).
function M.pods:exec(namespace, pod_name, command, opts)
  opts = opts or {}
  local argv = type(command) == "table" and command or { command }
  local insecure = opts.insecure
  if insecure == nil then insecure = true end

  local params = {}
  if opts.container then params[#params + 1] = "container=" .. url_encode(opts.container) end
  for _, arg in ipairs(argv) do
    params[#params + 1] = "command=" .. url_encode(arg)
  end
  params[#params + 1] = "stdout=true"
  params[#params + 1] = "stderr=true"
  params[#params + 1] = "stdin=" .. tostring(opts.stdin == true)
  params[#params + 1] = "tty=" .. tostring(opts.tty == true)

  local t = target(opts)
  local ws_base = t.base:gsub("^https://", "wss://"):gsub("^http://", "ws://")
  local url = ws_base
    .. "/api/v1/namespaces/" .. namespace .. "/pods/" .. pod_name .. "/exec"
    .. "?" .. table.concat(params, "&")

  local conn = ws.connect(url, {
    subprotocols = { "v4.channel.k8s.io" },
    headers = auth_headers(t.token),
    insecure = insecure,
  })

  local frames = {}
  local deadline = opts.timeout_secs and (time() + opts.timeout_secs) or nil
  while true do
    if deadline and time() > deadline then break end
    local ok, frame = pcall(ws.recv, conn)
    if not ok or frame == nil then break end
    frames[#frames + 1] = frame
  end
  pcall(ws.close, conn)

  return M._demux_exec_frames(frames)
end

-- ===== Services sub-object =====

M.services = {}

function M.services:endpoints(namespace, name, opts)
  local ep = M.resources:get(namespace, "endpoints", name, opts)
  local ips = {}
  for _, subset in ipairs(ep.subsets or {}) do
    for _, addr in ipairs(subset.addresses or {}) do
      ips[#ips + 1] = addr.ip
    end
  end
  return ips
end

-- ===== Deployments sub-object =====

M.deployments = {}

function M.deployments:rollout_status(namespace, name, opts)
  local deploy = M.resources:get(namespace, "deployment", name, opts)
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

-- ===== Nodes sub-object =====

M.nodes = {}

function M.nodes:status(opts)
  local nodes_list = M.get("/api/v1/nodes", opts)
  local result = {}
  for _, node in ipairs(nodes_list.items or {}) do
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

-- ===== Namespaces sub-object =====

M.namespaces = {}

function M.namespaces:exists(name, opts)
  return M.resources:exists(nil, "namespace", name, opts)
end

-- ===== Events sub-object =====

M.events = {}

function M.events:list(namespace, opts)
  return M.resources:list(namespace, "event", opts)
end

function M.events:for_resource(namespace, kind, name, opts)
  return M.resources:list(namespace, "event", {
    field_selector = "involvedObject.kind=" .. kind .. ",involvedObject.name=" .. name,
    base_url = (opts or {}).base_url,
    token = (opts or {}).token,
  })
end

return M
