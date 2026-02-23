--- @module assay.certmanager
--- @description cert-manager certificate lifecycle. Certificates, issuers, ACME orders and challenges.
--- @keywords certmanager, certificates, issuers, acme, tls, kubernetes
--- @quickref c:certificates(namespace) -> {items} | List certificates in namespace
--- @quickref c:certificate(namespace, name) -> cert|nil | Get certificate by name
--- @quickref c:certificate_status(namespace, name) -> {ready, not_after, renewal_time} | Get certificate status
--- @quickref c:is_certificate_ready(namespace, name) -> bool | Check if certificate is ready
--- @quickref c:wait_certificate_ready(namespace, name, timeout_secs?) -> true | Wait for certificate readiness
--- @quickref c:issuers(namespace) -> {items} | List issuers in namespace
--- @quickref c:issuer(namespace, name) -> issuer|nil | Get issuer by name
--- @quickref c:is_issuer_ready(namespace, name) -> bool | Check if issuer is ready
--- @quickref c:cluster_issuers() -> {items} | List cluster issuers
--- @quickref c:cluster_issuer(name) -> issuer|nil | Get cluster issuer by name
--- @quickref c:is_cluster_issuer_ready(name) -> bool | Check if cluster issuer is ready
--- @quickref c:certificate_requests(namespace) -> {items} | List certificate requests
--- @quickref c:certificate_request(namespace, name) -> request|nil | Get certificate request
--- @quickref c:is_request_approved(namespace, name) -> bool | Check if request is approved
--- @quickref c:orders(namespace) -> {items} | List ACME orders
--- @quickref c:order(namespace, name) -> order|nil | Get ACME order
--- @quickref c:challenges(namespace) -> {items} | List ACME challenges
--- @quickref c:challenge(namespace, name) -> challenge|nil | Get ACME challenge
--- @quickref c:all_certificates_ready(namespace) -> {ready, not_ready, total} | Check all certificates readiness
--- @quickref c:all_issuers_ready(namespace) -> {ready, not_ready, total} | Check all issuers readiness

local M = {}

function M.client(url, token)
  local c = {
    url = url:gsub("/+$", ""),
    token = token,
  }

  local function headers(self)
    return { ["Authorization"] = "Bearer " .. self.token }
  end

  local function api_get(self, api_path)
    local resp = http.get(self.url .. api_path, { headers = headers(self) })
    if resp.status == 404 then return nil end
    if resp.status ~= 200 then
      error("certmanager: GET " .. api_path .. " HTTP " .. resp.status .. ": " .. resp.body)
    end
    return json.parse(resp.body)
  end

  local function api_list(self, api_path)
    local resp = http.get(self.url .. api_path, { headers = headers(self) })
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

  -- Certificates

  function c:certificates(namespace)
    return api_list(self, "/apis/cert-manager.io/v1/namespaces/" .. namespace .. "/certificates")
  end

  function c:certificate(namespace, name)
    return api_get(self, "/apis/cert-manager.io/v1/namespaces/" .. namespace .. "/certificates/" .. name)
  end

  function c:certificate_status(namespace, name)
    local cert = self:certificate(namespace, name)
    if not cert then return nil end
    local status = cert.status or {}
    local ready_cond = find_condition(cert, "Ready")
    return {
      ready = ready_cond ~= nil and ready_cond.status == "True",
      not_after = status.notAfter,
      not_before = status.notBefore,
      renewal_time = status.renewalTime,
      revision = status.revision,
      conditions = status.conditions or {},
    }
  end

  function c:is_certificate_ready(namespace, name)
    local cert = self:certificate(namespace, name)
    if not cert then return false end
    local cond = find_condition(cert, "Ready")
    return cond ~= nil and cond.status == "True"
  end

  function c:wait_certificate_ready(namespace, name, timeout_secs)
    timeout_secs = timeout_secs or 300
    local start = time()
    while true do
      if self:is_certificate_ready(namespace, name) then
        return true
      end
      if time() - start >= timeout_secs then
        error("certmanager: timeout waiting for certificate " .. namespace .. "/" .. name .. " to be ready")
      end
      sleep(5)
    end
  end

  -- Issuers

  function c:issuers(namespace)
    return api_list(self, "/apis/cert-manager.io/v1/namespaces/" .. namespace .. "/issuers")
  end

  function c:issuer(namespace, name)
    return api_get(self, "/apis/cert-manager.io/v1/namespaces/" .. namespace .. "/issuers/" .. name)
  end

  function c:is_issuer_ready(namespace, name)
    local iss = self:issuer(namespace, name)
    if not iss then return false end
    local cond = find_condition(iss, "Ready")
    return cond ~= nil and cond.status == "True"
  end

  -- ClusterIssuers

  function c:cluster_issuers()
    return api_list(self, "/apis/cert-manager.io/v1/clusterissuers")
  end

  function c:cluster_issuer(name)
    return api_get(self, "/apis/cert-manager.io/v1/clusterissuers/" .. name)
  end

  function c:is_cluster_issuer_ready(name)
    local iss = self:cluster_issuer(name)
    if not iss then return false end
    local cond = find_condition(iss, "Ready")
    return cond ~= nil and cond.status == "True"
  end

  -- CertificateRequests

  function c:certificate_requests(namespace)
    return api_list(self, "/apis/cert-manager.io/v1/namespaces/" .. namespace .. "/certificaterequests")
  end

  function c:certificate_request(namespace, name)
    return api_get(self, "/apis/cert-manager.io/v1/namespaces/" .. namespace .. "/certificaterequests/" .. name)
  end

  function c:is_request_approved(namespace, name)
    local req = self:certificate_request(namespace, name)
    if not req then return false end
    local cond = find_condition(req, "Approved")
    return cond ~= nil and cond.status == "True"
  end

  -- ACME Orders

  function c:orders(namespace)
    return api_list(self, "/apis/acme.cert-manager.io/v1/namespaces/" .. namespace .. "/orders")
  end

  function c:order(namespace, name)
    return api_get(self, "/apis/acme.cert-manager.io/v1/namespaces/" .. namespace .. "/orders/" .. name)
  end

  -- ACME Challenges

  function c:challenges(namespace)
    return api_list(self, "/apis/acme.cert-manager.io/v1/namespaces/" .. namespace .. "/challenges")
  end

  function c:challenge(namespace, name)
    return api_get(self, "/apis/acme.cert-manager.io/v1/namespaces/" .. namespace .. "/challenges/" .. name)
  end

  -- Utilities

  function c:all_certificates_ready(namespace)
    local list = self:certificates(namespace)
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

  function c:all_issuers_ready(namespace)
    local list = self:issuers(namespace)
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

  return c
end

return M
