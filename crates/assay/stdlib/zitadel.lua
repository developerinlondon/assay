--- @module assay.zitadel
--- @description Zitadel OIDC identity management. Projects, OIDC apps, IdPs, users, login policies.
--- @keywords zitadel, oidc, identity, projects, applications, idp, users, authentication, domain, app, login-policy, user, password, google, machine-key, jwt, saml
--- @quickref c.domains:ensure_primary(domain) -> bool | Set organization primary domain
--- @quickref c.projects:find(name) -> project|nil | Find project by name
--- @quickref c.projects:create(name, opts?) -> project | Create a new project
--- @quickref c.projects:ensure(name, opts?) -> project | Ensure project exists
--- @quickref c.apps:find(project_id, name) -> app|nil | Find OIDC app by name
--- @quickref c.apps:create_oidc(project_id, opts) -> app | Create OIDC application
--- @quickref c.apps:ensure_oidc(project_id, opts) -> app | Ensure OIDC app exists
--- @quickref c.idps:find(name) -> idp|nil | Find identity provider by name
--- @quickref c.idps:ensure_google(opts) -> idp_id|nil | Ensure Google IdP exists
--- @quickref c.idps:ensure_oidc(opts) -> idp_id|nil | Ensure generic OIDC IdP exists
--- @quickref c.idps:add_to_login_policy(idp_id) -> bool | Add IdP to login policy
--- @quickref c.users:search(query) -> [user] | Search users
--- @quickref c.users:update_email(user_id, email) -> bool | Update user email
--- @quickref c.login_policy:get() -> policy|nil | Get login policy
--- @quickref c.login_policy:update(policy) -> bool | Update login policy
--- @quickref c.login_policy:disable_password() -> bool | Disable password-based login

local M = {}

function M.client(opts)
  opts = opts or {}
  local url = opts.url
  local domain = opts.domain
  assert.not_nil(url, "zitadel.client: url required")
  assert.not_nil(domain, "zitadel.client: domain required")

  local base_url = url:gsub("/+$", "")
  local host_header = "auth." .. domain
  local access_token = nil

  -- Private: authenticate via machine key JWT
  local function authenticate(key_data)
    -- key_data: { userId, key, keyId } -- from machine key JSON
    local now = time()
    local claims = {
      iss = key_data.userId,
      sub = key_data.userId,
      aud = "https://auth." .. domain,
      iat = now,
      exp = now + 300,
    }
    local jwt_token = crypto.jwt_sign(claims, key_data.key, "RS256", { kid = key_data.keyId })

    local token_body = "grant_type=urn%3Aietf%3Aparams%3Aoauth%3Agrant-type%3Ajwt-bearer"
      .. "&scope=openid+urn%3Azitadel%3Aiam%3Aorg%3Aproject%3Aid%3Azitadel%3Aaud"
      .. "&assertion=" .. jwt_token
    local resp = http.post(base_url .. "/oauth/v2/token", token_body, {
      headers = { ["Content-Type"] = "application/x-www-form-urlencoded", ["Host"] = host_header },
    })
    if resp.status ~= 200 then
      error("zitadel: token exchange failed (HTTP " .. resp.status .. "): " .. resp.body)
    end
    local data = json.parse(resp.body)
    assert.not_nil(data.access_token, "zitadel: no access_token in token response")
    access_token = data.access_token
    return access_token
  end

  -- Authenticate from machine key data (table) or file path (string)
  if opts.machine_key then
    authenticate(opts.machine_key)
  elseif opts.machine_key_file then
    local key_json = fs.read(opts.machine_key_file)
    local key_data = json.parse(key_json)
    assert.not_nil(key_data.userId, "zitadel: machine key missing userId")
    assert.not_nil(key_data.key, "zitadel: machine key missing key")
    assert.not_nil(key_data.keyId, "zitadel: machine key missing keyId")
    authenticate(key_data)
  elseif opts.token then
    access_token = opts.token
  else
    error("zitadel.client: one of machine_key, machine_key_file, or token required")
  end

  -- Shared HTTP helpers (captured by all sub-object methods as upvalues)

  local function headers()
    return {
      ["Authorization"] = "Bearer " .. access_token,
      ["Content-Type"] = "application/json",
      ["Host"] = host_header,
    }
  end

  local function api_get(path)
    local resp = http.get(base_url .. path, { headers = headers() })
    return resp
  end

  local function api_post(path, body)
    local resp = http.post(base_url .. path, body or "{}", { headers = headers() })
    return resp
  end

  local function api_put(path, body)
    local resp = http.put(base_url .. path, body or "{}", { headers = headers() })
    return resp
  end

  -- ===== Client =====

  -- Expose some fields for test assertions
  local c = {
    url = base_url,
    domain = domain,
    host_header = host_header,
    access_token = access_token,
  }

  -- ===== Domains =====

  c.domains = {}

  function c.domains:ensure_primary(target_domain)
    local resp = api_get("/admin/v1/orgs/me/domains")
    if resp.status ~= 200 then
      log.warn("zitadel: could not list org domains (HTTP " .. resp.status .. ")")
      return false
    end
    local data = json.parse(resp.body)
    if data.result then
      for _, d in ipairs(data.result) do
        if d.domainName == target_domain and d.isPrimary then
          log.info("Org primary domain already set to " .. target_domain)
          return true
        end
      end
    end
    -- Add domain (may already exist -- 409 is OK)
    local add_resp = api_post("/admin/v1/orgs/me/domains", { domain = target_domain })
    if add_resp.status ~= 200 and add_resp.status ~= 409 then
      log.warn("zitadel: could not add domain (HTTP " .. add_resp.status .. ")")
      return false
    end
    local primary_resp = api_post("/admin/v1/orgs/me/domains/" .. target_domain .. "/_set_primary", {})
    if primary_resp.status == 200 then
      log.info("Set org primary domain to " .. target_domain)
      return true
    end
    log.warn("zitadel: could not set primary domain (HTTP " .. primary_resp.status .. ")")
    return false
  end

  -- ===== Projects =====

  c.projects = {}

  function c.projects:find(name)
    local resp = api_post("/management/v1/projects/_search", {
      queries = { { nameQuery = { name = name, method = "TEXT_QUERY_METHOD_EQUALS" } } },
    })
    if resp.status ~= 200 then return nil end
    local data = json.parse(resp.body)
    if data.result and #data.result > 0 then
      return data.result[1]
    end
    return nil
  end

  function c.projects:create(name, opts_proj)
    opts_proj = opts_proj or {}
    local body = { name = name }
    if opts_proj.projectRoleAssertion ~= nil then
      body.projectRoleAssertion = opts_proj.projectRoleAssertion
    end
    local resp = api_post("/management/v1/projects", body)
    if resp.status ~= 200 then
      error("zitadel: failed to create project '" .. name .. "' (HTTP " .. resp.status .. "): " .. resp.body)
    end
    local data = json.parse(resp.body)
    log.info("Created project '" .. name .. "' (id=" .. tostring(data.id) .. ")")
    return data
  end

  function c.projects:ensure(name, opts_proj)
    local existing = c.projects:find(name)
    if existing then
      log.info("Project '" .. name .. "' already exists (id=" .. tostring(existing.id) .. ")")
      return existing
    end
    return c.projects:create(name, opts_proj)
  end

  -- ===== OIDC Apps =====

  c.apps = {}

  function c.apps:find(project_id, name)
    local body = {
      query = { limit = 100 },
      queries = { { nameQuery = { name = name, method = "TEXT_QUERY_METHOD_EQUALS" } } },
    }
    local resp = api_post("/management/v1/projects/" .. project_id .. "/apps/_search", body)
    if resp.status ~= 200 then
      -- Fallback: try without query filter (older Zitadel versions)
      resp = api_post("/management/v1/projects/" .. project_id .. "/apps/_search", { query = { limit = 100 } })
      if resp.status ~= 200 then return nil end
    end
    local data = json.parse(resp.body)
    if data.result then
      for _, a in ipairs(data.result) do
        if a.name == name then return a end
      end
    end
    return nil
  end

  function c.apps:create_oidc(project_id, opts_app)
    local redirect_uri = "https://" .. opts_app.subdomain .. "." .. domain .. opts_app.callbackPath
    local logout_uri = "https://" .. opts_app.subdomain .. "." .. domain .. "/"
    local body = {
      name = opts_app.name,
      redirectUris = opts_app.redirectUris or { redirect_uri },
      postLogoutRedirectUris = opts_app.postLogoutRedirectUris or { logout_uri },
      responseTypes = opts_app.responseTypes or { "OIDC_RESPONSE_TYPE_CODE" },
      grantTypes = opts_app.grantTypes or { "OIDC_GRANT_TYPE_AUTHORIZATION_CODE", "OIDC_GRANT_TYPE_REFRESH_TOKEN" },
      appType = opts_app.appType or "OIDC_APP_TYPE_WEB",
      authMethodType = opts_app.authMethodType or "OIDC_AUTH_METHOD_TYPE_BASIC",
      accessTokenType = opts_app.accessTokenType or "OIDC_TOKEN_TYPE_BEARER",
      accessTokenRoleAssertion = opts_app.accessTokenRoleAssertion ~= false,
      idTokenRoleAssertion = opts_app.idTokenRoleAssertion ~= false,
      idTokenUserinfoAssertion = opts_app.idTokenUserinfoAssertion ~= false,
      devMode = opts_app.devMode or false,
      clockSkew = opts_app.clockSkew or "0s",
    }
    local resp = api_post("/management/v1/projects/" .. project_id .. "/apps/oidc", body)
    if resp.status == 409 then
      log.info("OIDC app '" .. opts_app.name .. "' already exists (409), looking up...")
      local existing = c.apps:find(project_id, opts_app.name)
      if existing then return existing end
      log.warn("OIDC app '" .. opts_app.name .. "' exists (409) but search did not find it, returning stub")
      return { id = "existing", name = opts_app.name }
    end
    if resp.status ~= 200 then
      error("zitadel: failed to create OIDC app '" .. opts_app.name .. "' (HTTP " .. resp.status .. "): " .. resp.body)
    end
    local data = json.parse(resp.body)
    log.info("Created OIDC app '" .. opts_app.name .. "' (clientId=" .. tostring(data.clientId) .. ")")
    return data
  end

  function c.apps:ensure_oidc(project_id, opts_app)
    local existing = c.apps:find(project_id, opts_app.name)
    if existing then
      log.info("OIDC app '" .. opts_app.name .. "' already exists (id=" .. tostring(existing.id) .. ")")
      return existing
    end
    return c.apps:create_oidc(project_id, opts_app)
  end

  -- ===== IdPs =====

  c.idps = {}

  function c.idps:find(name)
    local resp = api_post("/admin/v1/idps/templates/_search", {
      queries = { { idpNameQuery = { name = name, method = "TEXT_QUERY_METHOD_EQUALS" } } },
    })
    if resp.status ~= 200 then return nil end
    local data = json.parse(resp.body)
    if data.result and #data.result > 0 then
      return data.result[1]
    end
    return nil
  end

  function c.idps:ensure_google(opts_idp)
    local existing = c.idps:find("Google")
    if existing then
      log.info("Google IdP already exists (id=" .. existing.id .. ")")
      return existing.id
    end
    local body = {
      name = "Google",
      clientId = opts_idp.clientId,
      clientSecret = opts_idp.clientSecret,
      scopes = opts_idp.scopes or { "openid", "email", "profile" },
      providerOptions = opts_idp.providerOptions or {
        isLinkingAllowed = true,
        isCreationAllowed = true,
        isAutoCreation = true,
        isAutoUpdate = true,
      },
    }
    local resp = api_post("/admin/v1/idps/google", body)
    if resp.status ~= 200 then
      log.warn("zitadel: failed to create Google IdP (HTTP " .. resp.status .. ")")
      return nil
    end
    local data = json.parse(resp.body)
    local idp_id = data.idp_id or data.id
    log.info("Created Google IdP (id=" .. tostring(idp_id) .. ")")
    return idp_id
  end

  function c.idps:ensure_oidc(opts_idp)
    local name = opts_idp.name
    assert.not_nil(name, "zitadel: ensure_oidc_idp requires name")
    local existing = c.idps:find(name)
    local provider_options = opts_idp.providerOptions or {
      isLinkingAllowed = true,
      isCreationAllowed = true,
      isAutoCreation = true,
      isAutoUpdate = true,
      autoLinking = opts_idp.autoLinking or "AUTO_LINKING_OPTION_EMAIL",
    }
    local body = {
      name = name,
      clientId = opts_idp.clientId,
      clientSecret = opts_idp.clientSecret,
      issuer = opts_idp.issuer,
      scopes = opts_idp.scopes or { "openid", "email", "profile" },
      isIdTokenMapping = opts_idp.isIdTokenMapping ~= false,
      providerOptions = provider_options,
    }
    if existing then
      log.info(name .. " IdP already exists (id=" .. existing.id .. "), updating...")
      local resp = api_put("/admin/v1/idps/generic_oidc/" .. existing.id, body)
      if resp.status == 200 then
        log.info(name .. " IdP updated")
      else
        log.warn("zitadel: failed to update " .. name .. " IdP (HTTP " .. resp.status .. ")")
      end
      return existing.id
    end
    local resp = api_post("/admin/v1/idps/generic_oidc", body)
    if resp.status ~= 200 then
      log.warn("zitadel: failed to create " .. name .. " IdP (HTTP " .. resp.status .. "): " .. resp.body)
      return nil
    end
    local data = json.parse(resp.body)
    local idp_id = data.id
    log.info("Created " .. name .. " IdP (id=" .. tostring(idp_id) .. ")")
    return idp_id
  end

  function c.idps:add_to_login_policy(idp_id)
    local resp = api_post("/admin/v1/policies/login/idps", {
      idpId = idp_id,
      ownerType = "IDPOWNERTYPE_SYSTEM",
    })
    if resp.status == 200 then
      log.info("IdP " .. idp_id .. " added to login policy")
      return true
    elseif resp.status == 409 then
      log.info("IdP " .. idp_id .. " already in login policy")
      return true
    end
    log.warn("zitadel: failed to add IdP to login policy (HTTP " .. resp.status .. ")")
    return false
  end

  -- ===== Users =====

  c.users = {}

  function c.users:search(query)
    local resp = api_post("/management/v1/users/_search", query)
    if resp.status ~= 200 then
      log.warn("zitadel: user search failed (HTTP " .. resp.status .. ")")
      return {}
    end
    local data = json.parse(resp.body)
    return data.result or {}
  end

  function c.users:update_email(user_id, email)
    local resp = api_put("/management/v1/users/" .. user_id .. "/email", {
      email = email,
      isEmailVerified = true,
    })
    if resp.status == 200 then
      log.info("Updated user " .. user_id .. " email to " .. email)
      return true
    end
    log.warn("zitadel: failed to update user email (HTTP " .. resp.status .. ")")
    return false
  end

  -- ===== Login Policy =====

  c.login_policy = {}

  function c.login_policy:get()
    local resp = api_get("/admin/v1/policies/login")
    if resp.status ~= 200 then return nil end
    local data = json.parse(resp.body)
    return data.policy
  end

  function c.login_policy:update(policy)
    local resp = api_put("/admin/v1/policies/login", policy)
    if resp.status == 200 then
      log.info("Login policy updated")
      return true
    end
    log.warn("zitadel: failed to update login policy (HTTP " .. resp.status .. "): " .. resp.body)
    return false
  end

  function c.login_policy:disable_password()
    local policy = c.login_policy:get()
    if not policy then
      log.warn("zitadel: could not read login policy")
      return false
    end
    if not policy.allowUsernamePassword then
      log.info("Password login already disabled")
      return true
    end
    return c.login_policy:update({
      allowUsernamePassword = false,
      allowExternalIdp = true,
      allowRegister = policy.allowRegister or false,
      forceMfa = policy.forceMfa or false,
      passwordlessType = policy.passwordlessType or "PASSWORDLESS_TYPE_NOT_ALLOWED",
      hidePasswordReset = true,
      passwordCheckLifetime = policy.passwordCheckLifetime,
      externalLoginCheckLifetime = policy.externalLoginCheckLifetime,
      mfaInitSkipLifetime = policy.mfaInitSkipLifetime,
      secondFactorCheckLifetime = policy.secondFactorCheckLifetime,
      multiFactorCheckLifetime = policy.multiFactorCheckLifetime,
    })
  end

  return c
end

return M
