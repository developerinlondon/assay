--! sysops.oidc tests — Authorization Code + PKCE client.
--!
--! Run via:
--!   LUA_PATH='libs/?.lua;libs/?/init.lua;libs/sysops/?.lua;libs/sysops/tests-lua/?.lua;;' \
--!     assay libs/sysops/tests-lua/oidc.test.lua

local oidc = require("sysops.oidc")

-- ---------------------------------------------------------------------
-- Stub HTTP — driven by a function the test installs; lets each `do`
-- block control what discovery/token endpoints return.
-- ---------------------------------------------------------------------

local function stub_http(handler)
  return {
    get = function(url_str, opts)
      return handler({
        method  = "GET",
        url     = url_str,
        headers = opts and opts.headers or {},
      })
    end,
    post = function(url_str, body, opts)
      return handler({
        method  = "POST",
        url     = url_str,
        headers = opts and opts.headers or {},
        body    = body,
      })
    end,
  }
end

local function discovery_doc()
  return {
    authorization_endpoint = "https://idp.test/auth/authorize",
    token_endpoint         = "https://idp.test/auth/token",
    jwks_uri               = "https://idp.test/auth/jwks",
    issuer                 = "https://idp.test",
  }
end

print("[sysops.oidc]")

-- ---------------------------------------------------------------------
-- 1. discover() pulls /.well-known/openid-configuration once and caches.
-- ---------------------------------------------------------------------

do
  local calls = 0
  local http_stub = stub_http(function(req)
    if req.url:match("openid%-configuration$") then
      calls = calls + 1
      return { status = 200, body = json.encode(discovery_doc()) }
    end
    error("unexpected URL: " .. req.url)
  end)

  local client = oidc.new({
    issuer    = "https://idp.test",
    client_id = "sysops",
    http      = http_stub,
  })
  local doc, err = client:discover()
  assert.eq(err, nil, "discover returns no error")
  assert.eq(doc.authorization_endpoint, "https://idp.test/auth/authorize",
            "discover returns authorize endpoint")
  client:discover() -- second call
  assert.eq(calls, 1, "discover caches after first hit")
  print("  ok discover() caches discovery document")
end

-- ---------------------------------------------------------------------
-- 2. issuer with trailing slash gets normalised.
-- ---------------------------------------------------------------------

do
  local seen_url = nil
  local http_stub = stub_http(function(req)
    seen_url = req.url
    return { status = 200, body = json.encode(discovery_doc()) }
  end)

  local client = oidc.new({
    issuer    = "https://idp.test/",
    client_id = "sysops",
    http      = http_stub,
  })
  client:discover()
  assert.eq(seen_url, "https://idp.test/.well-known/openid-configuration",
            "trailing slash on issuer is stripped before discovery URL")
  print("  ok issuer trailing slash normalised")
end

-- ---------------------------------------------------------------------
-- 3. authorize_url() embeds all PKCE + OIDC params.
-- ---------------------------------------------------------------------

do
  local http_stub = stub_http(function(_)
    return { status = 200, body = json.encode(discovery_doc()) }
  end)

  local client = oidc.new({
    issuer       = "https://idp.test",
    client_id    = "sysops",
    redirect_uri = "https://app.example/auth/callback",
    scopes       = { "openid", "email", "profile" },
    http         = http_stub,
  })

  local state = "abc123"
  local verifier = "v" .. string.rep("a", 63) -- 64 chars (PKCE 43-128 range)
  local u = client:authorize_url(state, verifier)

  assert.not_nil(u:find("^https://idp%.test/auth/authorize%?"), "authorize URL has correct base")
  assert.not_nil(u:find("response_type=code", 1, true), "response_type=code present")
  assert.not_nil(u:find("client_id=sysops", 1, true), "client_id present")
  assert.not_nil(u:find("state=" .. state, 1, true), "state present")
  assert.not_nil(u:find("code_challenge_method=S256", 1, true), "challenge method S256")
  assert.not_nil(u:find("code_challenge=", 1, true), "code_challenge present")
  assert.not_nil(u:find("scope=openid", 1, true), "scope present (form-encoded space)")
  assert.not_nil(u:find("redirect_uri=", 1, true), "redirect_uri present")

  -- Confirm code_challenge is 43 b64url chars: sha256 → 32 bytes →
  -- 43 b64url chars after stripping padding.
  local cc = u:match("code_challenge=([A-Za-z0-9%-_]+)")
  assert.eq(#cc, 43, "code_challenge length = 43 (base64url(sha256))")
  print("  ok authorize_url() embeds PKCE + OIDC params")
end

-- ---------------------------------------------------------------------
-- 4. exchange_code() POSTs to token endpoint and parses tokens.
-- ---------------------------------------------------------------------

do
  local seen_body, seen_ct = nil, nil
  local http_stub = stub_http(function(req)
    if req.method == "GET" then
      return { status = 200, body = json.encode(discovery_doc()) }
    end
    if req.method == "POST" then
      seen_body = req.body
      seen_ct   = req.headers["Content-Type"]
      return {
        status = 200,
        body   = json.encode({
          access_token  = "AT-1",
          id_token      = "ID-1",
          refresh_token = "RT-1",
          expires_in    = 3600,
          token_type    = "Bearer",
        }),
      }
    end
    error("unexpected method: " .. (req.method or "nil"))
  end)

  local client = oidc.new({
    issuer        = "https://idp.test",
    client_id     = "sysops",
    client_secret = "shh",
    redirect_uri  = "https://app.example/auth/callback",
    http          = http_stub,
  })
  local tokens, err = client:exchange_code("the-code", "the-verifier")
  assert.eq(err, nil, "exchange_code returns no error")
  assert.eq(tokens.access_token, "AT-1", "access_token parsed")
  assert.eq(tokens.id_token, "ID-1", "id_token parsed")
  assert.eq(tokens.refresh_token, "RT-1", "refresh_token parsed")
  assert.eq(seen_ct, "application/x-www-form-urlencoded", "content-type set")
  assert.not_nil(seen_body:find("grant_type=authorization_code", 1, true), "grant_type set")
  assert.not_nil(seen_body:find("code=the%-code", 1, false), "code present")
  assert.not_nil(seen_body:find("code_verifier=the%-verifier", 1, false), "verifier present")
  assert.not_nil(seen_body:find("client_id=sysops", 1, true), "client_id present")
  assert.not_nil(seen_body:find("client_secret=shh", 1, true), "client_secret present")
  print("  ok exchange_code() POSTs token request with PKCE verifier")
end

-- ---------------------------------------------------------------------
-- 5. exchange_code() surfaces non-2xx as { status, body } error.
-- ---------------------------------------------------------------------

do
  local http_stub = stub_http(function(req)
    if req.method == "GET" then
      return { status = 200, body = json.encode(discovery_doc()) }
    end
    return { status = 401, body = '{"error":"invalid_grant"}' }
  end)
  local client = oidc.new({ issuer = "https://idp.test", client_id = "x", http = http_stub })
  local tokens, err = client:exchange_code("bad", "v")
  assert.eq(tokens, nil, "no tokens returned on error")
  assert.not_nil(err, "err is non-nil")
  assert.eq(err.status, 401, "error.status surfaced")
  print("  ok exchange_code() surfaces 401 as structured error")
end

-- ---------------------------------------------------------------------
-- 6. refresh() uses refresh_token grant.
-- ---------------------------------------------------------------------

do
  local seen_body = nil
  local http_stub = stub_http(function(req)
    if req.method == "GET" then
      return { status = 200, body = json.encode(discovery_doc()) }
    end
    seen_body = req.body
    return {
      status = 200,
      body   = json.encode({
        access_token = "AT-2",
        expires_in   = 3600,
        token_type   = "Bearer",
      }),
    }
  end)
  local client = oidc.new({ issuer = "https://idp.test", client_id = "x", http = http_stub })
  local tokens, err = client:refresh("OLD-RT")
  assert.eq(err, nil, "refresh returns no error")
  assert.eq(tokens.access_token, "AT-2", "access_token refreshed")
  assert.not_nil(seen_body:find("grant_type=refresh_token", 1, true), "grant_type=refresh_token sent")
  assert.not_nil(seen_body:find("refresh_token=OLD%-RT", 1, false), "old refresh token sent")
  print("  ok refresh() uses refresh_token grant")
end

print("[sysops.oidc] ok")
