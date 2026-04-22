--- @module assay.certmanager
--- @description cert-manager certificate lifecycle. Certificates, issuers, ACME orders and challenges.
--- @keywords certmanager, certificates, issuers, acme, tls, kubernetes, letsencrypt, order, challenge, request, approval, readiness, wait, ssl
--- @quickref c.certificates:list(namespace) -> {items} | List certificates in namespace
--- @quickref c.certificates:get(namespace, name) -> cert|nil | Get certificate by name
--- @quickref c.certificates:status(namespace, name) -> {ready, not_after, renewal_time} | Get certificate status
--- @quickref c.certificates:is_ready(namespace, name) -> bool | Check if certificate is ready
--- @quickref c.certificates:wait_ready(namespace, name, timeout_secs?) -> true | Wait for certificate readiness
--- @quickref c.certificates:all_ready(namespace) -> {ready, not_ready, total} | Check all certificates readiness
--- @quickref c.issuers:list(namespace) -> {items} | List issuers in namespace
--- @quickref c.issuers:get(namespace, name) -> issuer|nil | Get issuer by name
--- @quickref c.issuers:is_ready(namespace, name) -> bool | Check if issuer is ready
--- @quickref c.issuers:all_ready(namespace) -> {ready, not_ready, total} | Check all issuers readiness
--- @quickref c.cluster_issuers:list() -> {items} | List cluster issuers
--- @quickref c.cluster_issuers:get(name) -> issuer|nil | Get cluster issuer by name
--- @quickref c.cluster_issuers:is_ready(name) -> bool | Check if cluster issuer is ready
--- @quickref c.requests:list(namespace) -> {items} | List certificate requests
--- @quickref c.requests:get(namespace, name) -> request|nil | Get certificate request
--- @quickref c.requests:is_approved(namespace, name) -> bool | Check if request is approved
--- @quickref c.orders:list(namespace) -> {items} | List ACME orders
--- @quickref c.orders:get(namespace, name) -> order|nil | Get ACME order
--- @quickref c.challenges:list(namespace) -> {items} | List ACME challenges
--- @quickref c.challenges:get(namespace, name) -> challenge|nil | Get ACME challenge

local M = {}

function M.client(url, token)
  local base_url = url:gsub("/+$", "")

  -- Shared HTTP helpers (captured by all sub-object methods as upvalues)

  local function headers()
    return { ["Authorization"] = "Bearer " .. token }
  end

  local function api_get(api_path)
    local resp = http.get(base_url .. api_path, { headers = headers() })
    if resp.status == 404 then return nil end
    if resp.status ~= 200 then
      error("certmanager: GET " .. api_path .. " HTTP " .. resp.status .. ": " .. resp.body)
    end
    return json.parse(resp.body)
  end

  local function api_list(api_path)
    local resp = http.get(base_url .. api_path, { headers = headers() })
    if resp.status == 404 then return { items = {} } end
    if resp.status ~= 200 then
      error("certmanager: LIST " .. api_path .. " HTTP " .. resp.status .. ": " .. resp.body)
    end
    return json.parse(resp.body)
  end

  local function find_condition(resource, cond_type)
    if not resource or not resource.status or not resource.status.conditions then
      return nil
    end
    for i = 1, #resource.status.conditions do
      local cond = resource.status.conditions[i]
      if cond.type == cond_type then
        return cond
      end
    end
    return nil
  end

  -- ===== Client =====

  local c = {}

  -- ===== Certificates =====

  c.certificates = {}

  function c.certificates:list(namespace)
    return api_list("/apis/cert-manager.io/v1/namespaces/" .. namespace .. "/certificates")
  end

  function c.certificates:get(namespace, name)
    return api_get("/apis/cert-manager.io/v1/namespaces/" .. namespace .. "/certificates/" .. name)
  end

  function c.certificates:status(namespace, name)
    local cert = c.certificates:get(namespace, name)
    if not cert then return nil end
    local st = cert.status or {}
    local ready_cond = find_condition(cert, "Ready")
    return {
      ready = ready_cond ~= nil and ready_cond.status == "True",
      not_after = st.notAfter,
      not_before = st.notBefore,
      renewal_time = st.renewalTime,
      revision = st.revision,
      conditions = st.conditions or {},
    }
  end

  function c.certificates:is_ready(namespace, name)
    local cert = c.certificates:get(namespace, name)
    if not cert then return false end
    local cond = find_condition(cert, "Ready")
    return cond ~= nil and cond.status == "True"
  end

  function c.certificates:wait_ready(namespace, name, timeout_secs)
    timeout_secs = timeout_secs or 300
    local start = time()
    while true do
      if c.certificates:is_ready(namespace, name) then
        return true
      end
      if time() - start >= timeout_secs then
        error("certmanager: timeout waiting for certificate " .. namespace .. "/" .. name .. " to be ready")
      end
      sleep(5)
    end
  end

  function c.certificates:all_ready(namespace)
    local list = c.certificates:list(namespace)
    local items = list.items or {}
    local ready_count = 0
    local not_ready_count = 0
    local not_ready_names = {}
    for i = 1, #items do
      local cert = items[i]
      local cond = find_condition(cert, "Ready")
      if cond and cond.status == "True" then
        ready_count = ready_count + 1
      else
        not_ready_count = not_ready_count + 1
        local cert_name = (cert.metadata or {}).name or "unknown"
        not_ready_names[#not_ready_names + 1] = cert_name
      end
    end
    return {
      ready = ready_count,
      not_ready = not_ready_count,
      total = #items,
      not_ready_names = not_ready_names,
    }
  end

  -- ===== Issuers =====

  c.issuers = {}

  function c.issuers:list(namespace)
    return api_list("/apis/cert-manager.io/v1/namespaces/" .. namespace .. "/issuers")
  end

  function c.issuers:get(namespace, name)
    return api_get("/apis/cert-manager.io/v1/namespaces/" .. namespace .. "/issuers/" .. name)
  end

  function c.issuers:is_ready(namespace, name)
    local iss = c.issuers:get(namespace, name)
    if not iss then return false end
    local cond = find_condition(iss, "Ready")
    return cond ~= nil and cond.status == "True"
  end

  function c.issuers:all_ready(namespace)
    local list = c.issuers:list(namespace)
    local items = list.items or {}
    local ready_count = 0
    local not_ready_count = 0
    local not_ready_names = {}
    for i = 1, #items do
      local iss = items[i]
      local cond = find_condition(iss, "Ready")
      if cond and cond.status == "True" then
        ready_count = ready_count + 1
      else
        not_ready_count = not_ready_count + 1
        local iss_name = (iss.metadata or {}).name or "unknown"
        not_ready_names[#not_ready_names + 1] = iss_name
      end
    end
    return {
      ready = ready_count,
      not_ready = not_ready_count,
      total = #items,
      not_ready_names = not_ready_names,
    }
  end

  -- ===== ClusterIssuers =====

  c.cluster_issuers = {}

  function c.cluster_issuers:list()
    return api_list("/apis/cert-manager.io/v1/clusterissuers")
  end

  function c.cluster_issuers:get(name)
    return api_get("/apis/cert-manager.io/v1/clusterissuers/" .. name)
  end

  function c.cluster_issuers:is_ready(name)
    local iss = c.cluster_issuers:get(name)
    if not iss then return false end
    local cond = find_condition(iss, "Ready")
    return cond ~= nil and cond.status == "True"
  end

  -- ===== CertificateRequests =====

  c.requests = {}

  function c.requests:list(namespace)
    return api_list("/apis/cert-manager.io/v1/namespaces/" .. namespace .. "/certificaterequests")
  end

  function c.requests:get(namespace, name)
    return api_get("/apis/cert-manager.io/v1/namespaces/" .. namespace .. "/certificaterequests/" .. name)
  end

  function c.requests:is_approved(namespace, name)
    local req = c.requests:get(namespace, name)
    if not req then return false end
    local cond = find_condition(req, "Approved")
    return cond ~= nil and cond.status == "True"
  end

  -- ===== ACME Orders =====

  c.orders = {}

  function c.orders:list(namespace)
    return api_list("/apis/acme.cert-manager.io/v1/namespaces/" .. namespace .. "/orders")
  end

  function c.orders:get(namespace, name)
    return api_get("/apis/acme.cert-manager.io/v1/namespaces/" .. namespace .. "/orders/" .. name)
  end

  -- ===== ACME Challenges =====

  c.challenges = {}

  function c.challenges:list(namespace)
    return api_list("/apis/acme.cert-manager.io/v1/namespaces/" .. namespace .. "/challenges")
  end

  function c.challenges:get(namespace, name)
    return api_get("/apis/acme.cert-manager.io/v1/namespaces/" .. namespace .. "/challenges/" .. name)
  end

  return c
end

return M
