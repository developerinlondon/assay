# 27 · sysops auth gateway

**Status:** spec **Date:** 2026-05-20 **Branch:** TBD (`feat/sysops-auth-gateway`) **Builds on:**
`25-v0.1.5-sysops-auth-vault-pages.md`, `26-oidc-provider-agnostic.md`, in-tree `libs/sysops` 0.1.7

## Goal

Make the bundled assay deployment (gondor and similar) drop the admin-token prompt after a user
signs in via OIDC, without touching `assay-engine`, `assay-auth`, or `assay-dashboard` code. Sysops
becomes the public front door; engine stays bearer-only behind it.

## Why this exists

Today, gondor exposes assay-engine directly. The auth and engine dashboard SPAs detect "no admin
token, no session" at boot and render a paste-your-token banner — even after a user has signed in
through assay-auth's OIDC flow. That session cookie has nowhere useful to go because every admin
endpoint on the engine gates on `require_admin_bearer`, which ignores cookies and JWTs.

Xandar's production deployment solved the same problem with a Traefik ForwardAuth + xandar-ui BFF
sitting in front of the engine. The engine itself is unchanged; xandar-ui does the OIDC dance and
injects a JWT on each upstream call. We can apply the same pattern in-process with sysops as the
BFF, using sysops's existing engine HTTP client primitive — no proxy needed, no engine changes
needed.

## Non-goals

- Touching `assay-engine`, `assay-auth`, or `assay-dashboard` code. All four dashboard SPAs work
  unchanged once sysops intercepts `/api/v1/engine/auth/whoami` and proxies the rest with admin
  bearer injection.
- Standalone vault-only / workflow-only deployments. Those consumers bring their own auth and remain
  unaffected by this work.
- Multi-tenant or per-user fine-grained authz beyond the existing Zanzibar `admin` role check.

## Architecture

```
                   gondor.fcar.ai
                       │
                       │ HTTPS — only gondor_session cookie (HttpOnly)
                       ▼
┌───────────────────────────────────────────────────┐
│  sysops (lua, mounted by gondor-sysops)           │
│                                                   │
│  OWNS (renders directly):                         │
│   /  /apps  /machines/*  /services/*              │
│   /zanzibar/*                                     │
│   /auth/login   /auth/callback   /auth/logout     │
│   /auth/users  /auth/sessions  /auth/oidc  …      │
│   /vault/kv  /vault/transit  /vault/sealing  …    │
│                                                   │
│  INTERCEPTS (answers from session, no upstream):  │
│   /api/v1/engine/auth/whoami                      │
│                                                   │
│  PROXIES (forward + inject admin bearer):         │
│   /workflow  /workflow/*                          │
│   /engine/console  /engine/console/*              │
│   /shared/*                                       │
│   /api/v1/engine/* (everything not intercepted)   │
└────────────────────────┬──────────────────────────┘
                         │ upstream: 127.0.0.1:8080
                         │ Authorization: Bearer <admin-bearer>
                         │ X-Forwarded-User: <sub>
                         ▼
┌───────────────────────────────────────────────────┐
│  gondor-engine (assay-engine, unchanged)          │
│  binds 127.0.0.1 only — not publicly reachable    │
└───────────────────────────────────────────────────┘
```

## Components to build (all in `libs/sysops/`)

| Module                                       | Responsibility                                                |
| -------------------------------------------- | ------------------------------------------------------------- |
| `libs/sysops/oidc.lua`                       | OIDC client: discovery, authorize-redirect, callback, refresh |
| `libs/sysops/session.lua`                    | Signed cookie issue/verify; in-memory session store           |
| `libs/sysops/gateway.lua`                    | whoami intercept + reverse-proxy w/ admin-bearer injection    |
| `libs/sysops/pages/auth/login.lua`           | `/auth/login` redirect-to-authorize                           |
| `libs/sysops/pages/auth/callback.lua`        | OIDC callback handler: code → tokens → session cookie         |
| `libs/sysops/pages/auth/logout.lua`          | revoke session, clear cookie                                  |
| `libs/sysops/middleware/require_session.lua` | Gate for sysops's own /auth/* and /vault/* pages              |
| `libs/sysops/middleware/require_admin.lua`   | Zanzibar role check on `sub` before privileged paths          |

## mount() opts extension

```lua
sysops.mount(routes, {
  -- existing opts...
  oidc = {
    issuer      = "https://gondor.fcar.ai/auth",   -- assay-auth IdP or external
    client_id   = "sysops",
    client_secret = "<from-secret-store>",         -- optional for public clients
    redirect_uri  = "https://gondor.fcar.ai/auth/callback",
    scopes        = { "openid", "profile", "email" },
  },
  session = {
    cookie_name = "gondor_session",                -- default: "sysops_session"
    signing_key = "<from-secret-store>",
    ttl_seconds = 86400,                           -- default: 24h
  },
  gateway = {
    engine_upstream    = "http://127.0.0.1:8080",
    admin_bearer       = "<from-secret-store>",    -- the engine's admin api key
    proxy_paths        = { "/workflow", "/engine/console", "/shared", "/api/v1/engine" },
    intercept_whoami   = true,                     -- default true
  },
  authz = {
    require_zanzibar_admin = true,                 -- default true: gate proxy on sub admin role
    bootstrap_first_admin  = true,                 -- default true: first login on empty store gets admin
  },
})
```

## Tasks

### Task 1: OIDC client module (libs/sysops/oidc.lua)

Implements the OIDC Authorization Code + PKCE flow. Pure lua — calls inject via `opts.http`.

**Files:**

- Create: `libs/sysops/oidc.lua`
- Test: `libs/sysops/tests-lua/oidc.test.lua`

- [ ] **Step 1: Write failing test for discovery**

```lua
-- libs/sysops/tests-lua/oidc.test.lua
local oidc = require("sysops.oidc")
local stubs = require("sysops.tests-lua.stubs")

describe("oidc.discover", function()
  it("returns endpoints from /.well-known/openid-configuration", function()
    local http = stubs.http({
      ["https://example.test/.well-known/openid-configuration"] = {
        status = 200,
        body = {
          authorization_endpoint = "https://example.test/auth/authorize",
          token_endpoint         = "https://example.test/auth/token",
          jwks_uri               = "https://example.test/auth/jwks",
        },
      },
    })
    local client = oidc.new({ issuer = "https://example.test", http = http })
    local endpoints, err = client:discover()
    assert.is_nil(err)
    assert.equals("https://example.test/auth/authorize", endpoints.authorization_endpoint)
  end)
end)
```

Run: `cd libs/sysops && lua tests-lua/oidc.test.lua` → expected FAIL ("module not found").

- [ ] **Step 2: Minimal oidc.lua skeleton with discover()**

```lua
-- libs/sysops/oidc.lua
local M = {}

local function http_get_json(http, url)
  local r = http.request({ method = "GET", url = url })
  if not r or r.status ~= 200 then
    return nil, { status = (r and r.status) or 0, body = r and r.body }
  end
  return r.body
end

function M.new(opts)
  assert(opts and opts.issuer, "oidc.new: opts.issuer required")
  assert(opts and opts.http,   "oidc.new: opts.http required (HTTP client)")
  local self = { issuer = opts.issuer, http = opts.http }
  self._discovery_url = self.issuer:gsub("/$", "") .. "/.well-known/openid-configuration"

  function self:discover()
    if self._endpoints then return self._endpoints end
    local body, err = http_get_json(self.http, self._discovery_url)
    if err then return nil, err end
    self._endpoints = body
    return body
  end

  return self
end

return M
```

Run test → expected PASS.

- [ ] **Step 3: Add authorize-URL builder with PKCE state**

```lua
-- in libs/sysops/oidc.lua
local sha256, base64url = require("sysops.crypto").sha256, require("sysops.crypto").base64url

function M.new(opts)
  -- ... existing ...
  function self:authorize_url(state, code_verifier, redirect_uri, scopes)
    local endpoints = assert(self:discover())
    local code_challenge = base64url(sha256(code_verifier))
    local params = {
      response_type = "code",
      client_id     = opts.client_id,
      redirect_uri  = redirect_uri,
      scope         = table.concat(scopes or { "openid", "profile", "email" }, " "),
      state         = state,
      code_challenge = code_challenge,
      code_challenge_method = "S256",
    }
    return endpoints.authorization_endpoint .. "?" .. encode_query(params)
  end
  return self
end
```

Write a test for `authorize_url` returns a URL containing each parameter. Run, expect PASS.

- [ ] **Step 4: Add `exchange_code` (token endpoint)**

```lua
function self:exchange_code(code, code_verifier, redirect_uri)
  local endpoints = assert(self:discover())
  local r = self.http.request({
    method = "POST",
    url    = endpoints.token_endpoint,
    headers = { ["content-type"] = "application/x-www-form-urlencoded" },
    body   = encode_query({
      grant_type    = "authorization_code",
      code          = code,
      redirect_uri  = redirect_uri,
      client_id     = opts.client_id,
      client_secret = opts.client_secret,  -- optional
      code_verifier = code_verifier,
    }),
  })
  if not r or r.status ~= 200 then
    return nil, { status = (r and r.status) or 0, body = r and r.body }
  end
  return r.body  -- { access_token, id_token, refresh_token, expires_in }
end
```

Test: stub returns a token response, assert fields parsed. Run, expect PASS.

- [ ] **Step 5: Add `refresh` (refresh_token grant)**

Same shape as `exchange_code` but `grant_type = "refresh_token"`. Test stub returns refreshed
tokens. Run, expect PASS.

- [ ] **Step 6: Add `verify_id_token` (JWKS validation)**

Pull `jwks_uri` from discovery, fetch JWKS once and cache. Use existing jwt library (verify which
one — `lua-resty-jwt` or similar; if none, document the gap and stub). Test stubs a known-key signed
token. Run, expect PASS.

- [ ] **Step 7: Commit**

```bash
git add libs/sysops/oidc.lua libs/sysops/tests-lua/oidc.test.lua
git commit -m "feat(sysops): OIDC client with PKCE for auth gateway"
```

---

### Task 2: Session cookie module (libs/sysops/session.lua)

HMAC-signed cookie carrying `{ sub, email, exp, refresh_ref }`. Refresh tokens stored server-side
(in-process map for v1; can move to engine vault later).

**Files:**

- Create: `libs/sysops/session.lua`
- Test: `libs/sysops/tests-lua/session.test.lua`

- [ ] **Step 1: Failing test for issue + verify roundtrip**

```lua
local session = require("sysops.session")
local s = session.new({ signing_key = "0123456789abcdef0123456789abcdef" })

it("issues a cookie and verifies it back", function()
  local cookie = s:issue({ sub = "alice@example", exp = os.time() + 3600 })
  local claims, err = s:verify(cookie)
  assert.is_nil(err)
  assert.equals("alice@example", claims.sub)
end)
```

Run, expect FAIL.

- [ ] **Step 2: Implement issue()/verify() with HMAC-SHA256**

```lua
-- libs/sysops/session.lua
local crypto = require("sysops.crypto")
local json   = require("dkjson")  -- or whatever sysops uses; verify in step 0
local M = {}

local function b64u(s) return crypto.base64url(s) end
local function b64u_decode(s) return crypto.base64url_decode(s) end

function M.new(opts)
  assert(opts.signing_key and #opts.signing_key >= 32,
         "session: signing_key must be ≥32 bytes")
  local self = {}

  function self:issue(claims)
    local payload = b64u(json.encode(claims))
    local sig = b64u(crypto.hmac_sha256(opts.signing_key, payload))
    return payload .. "." .. sig
  end

  function self:verify(cookie_value)
    local payload, sig = cookie_value:match("^([^.]+)%.([^.]+)$")
    if not payload then return nil, "malformed" end
    local expected = b64u(crypto.hmac_sha256(opts.signing_key, payload))
    if not crypto.consteq(sig, expected) then return nil, "bad signature" end
    local claims = json.decode(b64u_decode(payload))
    if not claims then return nil, "bad payload" end
    if claims.exp and claims.exp < os.time() then return nil, "expired" end
    return claims
  end

  return self
end

return M
```

Run, expect PASS.

- [ ] **Step 3: Add tamper test**

```lua
it("rejects tampered payload", function()
  local cookie = s:issue({ sub = "alice", exp = os.time() + 3600 })
  local payload, sig = cookie:match("^([^.]+)%.([^.]+)$")
  local tampered = payload:gsub("alice", "mallory") .. "." .. sig
  local claims, err = s:verify(tampered)
  assert.is_nil(claims)
  assert.equals("bad signature", err)
end)
```

Run, expect PASS (HMAC check should catch it).

- [ ] **Step 4: Add expiry test**

```lua
it("rejects expired cookie", function()
  local cookie = s:issue({ sub = "alice", exp = os.time() - 1 })
  local claims, err = s:verify(cookie)
  assert.is_nil(claims)
  assert.equals("expired", err)
end)
```

Run, expect PASS.

- [ ] **Step 5: Commit**

```bash
git add libs/sysops/session.lua libs/sysops/tests-lua/session.test.lua
git commit -m "feat(sysops): HMAC-signed session cookies"
```

---

### Task 3: Login/callback/logout pages

**Files:**

- Create: `libs/sysops/pages/auth/login.lua`
- Create: `libs/sysops/pages/auth/callback.lua`
- Create: `libs/sysops/pages/auth/logout.lua`
- Modify: `libs/sysops/mount.lua` — register routes
- Test: `libs/sysops/tests-lua/auth/oidc_flow.test.lua`

- [ ] **Step 1: Failing end-to-end OIDC happy-path test**

Test boots a fake OIDC IdP (stub HTTP), mounts sysops with `oidc` opts, simulates `GET /auth/login`
→ 302 to authorize → simulated callback w/ code → 302 to `/` with session cookie.

```lua
it("login → authorize redirect → callback sets session cookie", function()
  local routes = { GET = {}, POST = {} }
  sysops.mount(routes, mount_opts_with_oidc())

  local r1 = call(routes, "GET", "/auth/login")
  assert.equals(302, r1.status)
  assert.is_truthy(r1.headers["location"]:match("authorization_endpoint"))

  -- pretend we got back a code
  local r2 = call(routes, "GET", "/auth/callback?code=fake&state=" .. state_from(r1))
  assert.equals(302, r2.status)
  assert.is_truthy(r2.headers["set-cookie"]:match("gondor_session="))
end)
```

Run, expect FAIL.

- [ ] **Step 2: Implement /auth/login**

```lua
-- libs/sysops/pages/auth/login.lua
local M = {}

function M.handler(ctx, req)
  local state = ctx.crypto.random_token(32)
  local verifier = ctx.crypto.random_token(64)
  ctx.session_store:put_pending(state, { verifier = verifier, return_to = req.query.return_to or "/" })
  local url = ctx.oidc:authorize_url(state, verifier, ctx.config.oidc.redirect_uri, ctx.config.oidc.scopes)
  return { status = 302, headers = { location = url } }
end

return M
```

Add the route in `mount.lua` GET_ROUTES: `["/auth/login"] = "auth_login"`. Wire
`pages.auth.login.handler` into the handler dispatch.

Run first half of test, expect PASS.

- [ ] **Step 3: Implement /auth/callback**

```lua
-- libs/sysops/pages/auth/callback.lua
local M = {}

function M.handler(ctx, req)
  local pending = ctx.session_store:take_pending(req.query.state)
  if not pending then return { status = 400, body = "invalid state" } end

  local tokens, err = ctx.oidc:exchange_code(
    req.query.code, pending.verifier, ctx.config.oidc.redirect_uri)
  if err then return { status = 502, body = "token exchange failed: " .. (err.body or "") } end

  local claims, verr = ctx.oidc:verify_id_token(tokens.id_token)
  if verr then return { status = 401, body = "id_token verify failed" } end

  ctx.session_store:put_refresh(claims.sub, tokens.refresh_token)

  local cookie = ctx.session:issue({
    sub  = claims.sub,
    email = claims.email,
    exp  = os.time() + ctx.config.session.ttl_seconds,
  })
  return {
    status = 302,
    headers = {
      location = pending.return_to,
      ["set-cookie"] = ctx.config.session.cookie_name .. "=" .. cookie ..
        "; HttpOnly; Secure; SameSite=Lax; Path=/; Max-Age=" .. ctx.config.session.ttl_seconds,
    },
  }
end

return M
```

Wire route. Run full test, expect PASS.

- [ ] **Step 4: Implement /auth/logout**

```lua
-- libs/sysops/pages/auth/logout.lua
local M = {}
function M.handler(ctx, req)
  local claims = ctx.current_session(req)
  if claims then ctx.session_store:revoke(claims.sub) end
  return {
    status = 302,
    headers = {
      location = "/",
      ["set-cookie"] = ctx.config.session.cookie_name .. "=; Max-Age=0; Path=/",
    },
  }
end
return M
```

Test logout clears cookie. Run, expect PASS.

- [ ] **Step 5: Commit**

```bash
git add libs/sysops/pages/auth/{login,callback,logout}.lua libs/sysops/mount.lua \
        libs/sysops/tests-lua/auth/oidc_flow.test.lua
git commit -m "feat(sysops): /auth/login, /auth/callback, /auth/logout pages"
```

---

### Task 4: whoami intercept

**Files:**

- Create: `libs/sysops/gateway.lua`
- Modify: `libs/sysops/mount.lua`
- Test: `libs/sysops/tests-lua/gateway_whoami.test.lua`

- [ ] **Step 1: Failing test**

```lua
it("whoami returns 200 with session identity when cookie present", function()
  local routes = mount_with_session_cookie({ sub = "alice@example", email = "alice@example" })
  local r = call(routes, "GET", "/api/v1/engine/auth/whoami",
                 { cookie = "gondor_session=" .. issued_cookie })
  assert.equals(200, r.status)
  local body = json.decode(r.body)
  assert.equals("alice@example", body.sub)
end)

it("whoami returns 401 with no cookie", function()
  local routes = mount_with_session_cookie(nil)
  local r = call(routes, "GET", "/api/v1/engine/auth/whoami")
  assert.equals(401, r.status)
end)
```

Run, expect FAIL.

- [ ] **Step 2: Implement gateway.whoami**

```lua
-- libs/sysops/gateway.lua
local M = {}

function M.whoami(ctx, req)
  local cookie = parse_cookie(req.headers.cookie or "", ctx.config.session.cookie_name)
  if not cookie then return { status = 401, body = '{"error":"no session"}' } end
  local claims, err = ctx.session:verify(cookie)
  if err then return { status = 401, body = '{"error":"' .. err .. '"}' } end
  return {
    status = 200,
    headers = { ["content-type"] = "application/json" },
    body = ctx.json.encode({
      sub     = claims.sub,
      email   = claims.email,
      user_id = claims.sub,
    }),
  }
end

return M
```

Register `["/api/v1/engine/auth/whoami"] = "gateway_whoami"` in mount.lua GET routes.

Run, expect PASS for both cases.

- [ ] **Step 3: Commit**

```bash
git add libs/sysops/gateway.lua libs/sysops/mount.lua \
        libs/sysops/tests-lua/gateway_whoami.test.lua
git commit -m "feat(sysops): /whoami intercept defuses dashboard token banners"
```

---

### Task 5: Reverse-proxy with admin-bearer injection

**Files:**

- Modify: `libs/sysops/gateway.lua`
- Modify: `libs/sysops/mount.lua` — register wildcard proxy routes
- Test: `libs/sysops/tests-lua/gateway_proxy.test.lua`

**Design — dual-mode gateway** (preserves all access patterns, not just OIDC browser users):

```
gateway.proxy(req):
  if req.headers.Authorization starts with "Bearer ":
      # Caller brought their own bearer (admin key OR a JWT from a
      # trusted issuer). Pass through unchanged. Engine validates.
      # Preserves: ssh+curl, CI scripts, SPA token-banner mode,
      # customer's-own-IdP-JWT.
      forward(req, no extra headers)
  elif valid_session(req.cookie):
      # Browser user signed in via OIDC. No bearer from them; sysops
      # injects the admin bearer it holds server-side.
      forward(req, Authorization: Bearer <admin-bearer-from-opts>,
                   X-Forwarded-User: <sub>, X-User-Id: <sub>)
  else:
      return 401
```

The Zanzibar role check (configurable) ONLY runs on the session-injected branch — callers arriving
with their own bearer have already proven authority by holding the secret, no further gate is
meaningful.

- [ ] **Step 1: Failing test — three scenarios**

```lua
local function workflow_runs_stub()
  return stubs.engine({
    ["/api/v1/engine/workflow/runs"] = function(req)
      return { status = 200, body = '{"runs":[],"echo_auth":"' ..
                                   tostring(req.headers.authorization or "") .. '"}' }
    end,
  })
end

it("session-only: sysops injects admin bearer", function()
  local routes, cookie = mount_with_session({ sub = "alice" },
                                            { admin_bearer = "TEST-ADMIN" },
                                            workflow_runs_stub())
  local r = call(routes, "GET", "/api/v1/engine/workflow/runs",
                 { cookie = "gondor_session=" .. cookie })
  assert.equals(200, r.status)
  assert.equals("Bearer TEST-ADMIN", json.parse(r.body).echo_auth)
end)

it("caller-supplied bearer: passed through unchanged", function()
  local routes = mount_with_session(nil,
                                    { admin_bearer = "TEST-ADMIN" },
                                    workflow_runs_stub())
  local r = call(routes, "GET", "/api/v1/engine/workflow/runs",
                 { authorization = "Bearer CALLER-OWN-BEARER" })
  assert.equals(200, r.status)
  -- Sysops did NOT replace the caller's bearer with TEST-ADMIN
  assert.equals("Bearer CALLER-OWN-BEARER", json.parse(r.body).echo_auth)
end)

it("no session, no bearer: 401", function()
  local routes = mount_with_session(nil,
                                    { admin_bearer = "TEST-ADMIN" },
                                    workflow_runs_stub())
  local r = call(routes, "GET", "/api/v1/engine/workflow/runs")
  assert.equals(401, r.status)
end)
```

Run, expect FAIL.

- [ ] **Step 2: Implement dual-mode gateway.proxy**

```lua
function M.proxy(ctx, req)
  local incoming_auth = req.headers.authorization or req.headers.Authorization
  if incoming_auth and incoming_auth:match("^[Bb]earer ") then
    -- Pass-through mode: trust the caller's bearer; engine validates.
    return forward(ctx, req, {})
  end

  -- Session-injection mode: require a valid sysops session cookie.
  local cookie = parse_cookie(req.headers.cookie or "", ctx.config.session.cookie_name)
  local claims, verr = nil, nil
  if cookie then claims, verr = ctx.session:verify(cookie) end
  if not claims then return { status = 401, body = '{"error":"unauthenticated"}' } end

  if ctx.config.authz.require_zanzibar_admin then
    local ok = ctx.auth_sdk.zanzibar.check("user:" .. claims.sub, "admin", "engine:core")
    if not ok then return { status = 403, body = '{"error":"forbidden"}' } end
  end

  return forward(ctx, req, {
    ["authorization"]    = "Bearer " .. ctx.config.gateway.admin_bearer,
    ["x-forwarded-user"] = claims.sub,
    ["x-user-id"]        = claims.sub,
  })
end

local function forward(ctx, req, extra_headers)
  local headers = strip_hop_by_hop(req.headers)
  for k, v in pairs(extra_headers) do headers[k] = v end
  local upstream = ctx.engine.request({
    method  = req.method,
    path    = req.path,
    query   = req.raw_query,
    headers = headers,
    body    = req.body,
  })
  return {
    status  = upstream.status,
    headers = strip_hop_by_hop(upstream.headers),
    body    = upstream.body,
  }
end
```

Wire route: catch-all `["/api/v1/engine/*"] = "gateway_proxy"`. Order matters — register AFTER
`/api/v1/engine/auth/whoami` so the more-specific match wins (verify this is how the codebase's
route table resolves; if not, add explicit precedence).

Run, expect PASS for both cases.

- [ ] **Step 3: Add `/workflow`, `/engine/console`, `/shared/*` SPA-asset proxy**

Same handler, no Zanzibar check on asset paths (just session). Add a `gateway.proxy_assets` variant
or a flag to skip the role check based on path category.

Test that `GET /workflow/` returns 200 (proxies engine's SPA HTML).

- [ ] **Step 4: Commit**

```bash
git add libs/sysops/gateway.lua libs/sysops/mount.lua \
        libs/sysops/tests-lua/gateway_proxy.test.lua
git commit -m "feat(sysops): reverse-proxy w/ admin-bearer injection + Zanzibar role check"
```

---

### Task 6: Session middleware for sysops's own pages

Today sysops's `/auth/*` and `/vault/*` pages call into engine via the `auth_sdk` using the admin
bearer (per `libs/sysops/auth/session.lua`). They need to also require a valid `gondor_session`
cookie so unauthenticated browsers can't reach them.

**Files:**

- Create: `libs/sysops/middleware/require_session.lua`
- Modify: `libs/sysops/pages.lua` — wrap auth/vault/zanzibar handlers
- Test: `libs/sysops/tests-lua/require_session.test.lua`

- [ ] **Step 1: Failing test — `/auth/users` without cookie redirects to /auth/login**

```lua
it("/auth/users without session → 302 /auth/login", function()
  local routes = mount_with_session_cookie(nil)
  local r = call(routes, "GET", "/auth/users")
  assert.equals(302, r.status)
  assert.equals("/auth/login?return_to=%2Fauth%2Fusers", r.headers.location)
end)
```

Run, expect FAIL.

- [ ] **Step 2: Implement middleware**

```lua
-- libs/sysops/middleware/require_session.lua
local M = {}
function M.wrap(inner_handler)
  return function(ctx, req)
    local cookie = parse_cookie(req.headers.cookie or "", ctx.config.session.cookie_name)
    local claims = cookie and ({ ctx.session:verify(cookie) })[1]
    if not claims then
      return { status = 302, headers = { location = "/auth/login?return_to=" .. urlencode(req.path) } }
    end
    req.session_claims = claims
    return inner_handler(ctx, req)
  end
end
return M
```

In `pages.lua`, wrap every handler whose route starts with `/auth/` (except `/auth/login`,
`/auth/callback`, `/auth/logout`) or `/vault/` or `/zanzibar/`.

Run, expect PASS.

- [ ] **Step 3: Commit**

```bash
git add libs/sysops/middleware/require_session.lua libs/sysops/pages.lua \
        libs/sysops/tests-lua/require_session.test.lua
git commit -m "feat(sysops): require session for auth/vault/zanzibar pages"
```

---

### Task 7: Bootstrap first admin

On a fresh deployment, no Zanzibar tuple grants admin to anyone. First user to log in via OIDC
should land in a setup page that grants them admin. Subsequent users go through normal flow.

**Files:**

- Create: `libs/sysops/pages/auth/bootstrap.lua`
- Modify: `libs/sysops/pages/auth/callback.lua` — redirect to bootstrap on first login
- Test: `libs/sysops/tests-lua/auth/bootstrap.test.lua`

- [ ] **Step 1: Failing test — first login lands at /auth/bootstrap, accepts grant**

```lua
it("first login (zero admins in zanzibar) → /auth/bootstrap → grants admin", function()
  -- mount where auth_sdk.zanzibar.list_admins returns {}
  local routes = mount_first_run()
  local r1 = simulate_callback(routes, "alice@example")
  assert.equals("/auth/bootstrap", r1.headers.location:match("^([^?]+)"))

  local r2 = call(routes, "POST", "/auth/bootstrap",
                  { cookie = "gondor_session=" .. cookie_from(r1) })
  assert.equals(302, r2.status)
  -- subsequent zanzibar.list_admins includes alice
  local admins = mounted_ctx.auth_sdk.zanzibar.list_admins()
  assert.includes(admins, "user:alice@example")
end)
```

Run, expect FAIL.

- [ ] **Step 2: Implement /auth/bootstrap GET + POST**

```lua
-- libs/sysops/pages/auth/bootstrap.lua
local M = {}

local function admins_exist(auth_sdk)
  local list = auth_sdk.zanzibar.list_admins() or {}
  return #list > 0
end

function M.get(ctx, req)
  if admins_exist(ctx.auth_sdk) then return { status = 404 } end
  return { status = 200, body = render_bootstrap_page(req.session_claims) }
end

function M.post(ctx, req)
  if admins_exist(ctx.auth_sdk) then return { status = 409, body = "admins already exist" } end
  local sub = req.session_claims.sub
  ctx.auth_sdk.zanzibar.write_tuple("user:" .. sub, "admin", "engine:core")
  ctx.audit:log("bootstrap_admin", { sub = sub })
  return { status = 302, headers = { location = "/" } }
end

return M
```

In callback.lua, after issuing the cookie, branch on `admins_exist(ctx.auth_sdk)`:

- if no admins → redirect to `/auth/bootstrap`
- else → redirect to `pending.return_to`

Run, expect PASS.

- [ ] **Step 3: Commit**

```bash
git add libs/sysops/pages/auth/bootstrap.lua libs/sysops/pages/auth/callback.lua \
        libs/sysops/tests-lua/auth/bootstrap.test.lua
git commit -m "feat(sysops): bootstrap first admin on fresh deployment"
```

---

### Task 8: Wire mount.lua opts + ctx threading

**Files:**

- Modify: `libs/sysops/mount.lua` — accept new opts, build modules, attach to ctx
- Modify: `libs/sysops/ctx.lua` — surface `oidc`, `session`, `session_store`, `gateway` on ctx
- Test: `libs/sysops/tests-lua/mount.test.lua`

- [ ] **Step 1: Failing test — mount() with new opts wires everything**

```lua
it("mount() exposes oidc, session, gateway via ctx", function()
  local routes = { GET = {}, POST = {} }
  sysops.mount(routes, full_opts_with_auth())
  assert.is_truthy(routes.GET["/auth/login"])
  assert.is_truthy(routes.GET["/auth/callback"])
  assert.is_truthy(routes.GET["/api/v1/engine/auth/whoami"])
  -- proxy catch-all
  assert.is_truthy(routes.GET["/api/v1/engine/*"])
end)
```

Run, expect FAIL.

- [ ] **Step 2: Extend mount.lua opts schema + ctx**

In `mount.lua`:

- after parsing existing opts, validate `oidc`, `session`, `gateway`, `authz` blocks
- build `ctx.oidc = require("sysops.oidc").new(opts.oidc + { http = ... })`
- build `ctx.session = require("sysops.session").new(opts.session)`
- build `ctx.session_store` (in-memory map for v1)
- register the new routes in GET_ROUTES / POST_ROUTES

Run, expect PASS.

- [ ] **Step 3: Add backward-compat path — if `opts.oidc == nil`, skip all gateway/auth wiring**

The library MUST keep working for existing consumers that don't opt into the auth gateway. Just
don't register the auth pages or proxy routes if `opts.oidc` is absent.

Test that mounting without `opts.oidc` doesn't register `/auth/login`.

- [ ] **Step 4: Commit**

```bash
git add libs/sysops/mount.lua libs/sysops/ctx.lua libs/sysops/tests-lua/mount.test.lua
git commit -m "feat(sysops): mount opts for auth gateway (oidc, session, gateway, authz)"
```

---

### Task 9: Bump sysops version + CHANGELOG

**Files:**

- Modify: `libs/sysops/VERSION`
- Modify: `libs/sysops/README.md`
- Modify: `CHANGELOG.md` (root)

- [ ] **Step 1: Bump VERSION 0.1.7 → 0.2.0** (minor: new mount opts, additive)

```
0.2.0
```

- [ ] **Step 2: Update README with the auth-gateway opts and a new "Auth Gateway" section**

Document the four new opts blocks (oidc, session, gateway, authz), with the minimal example for
gondor.

- [ ] **Step 3: Add CHANGELOG entry**

```
## sysops 0.2.0
- Auth gateway: OIDC client, /whoami intercept, reverse-proxy with admin-bearer injection.
  Bundled assay deployments no longer prompt for admin token after OIDC login.
- Mount opts: `oidc`, `session`, `gateway`, `authz`. All optional — existing consumers unaffected.
```

- [ ] **Step 4: Commit**

```bash
git add libs/sysops/VERSION libs/sysops/README.md CHANGELOG.md
git commit -m "chore(sysops): 0.1.7 → 0.2.0 (auth gateway)"
```

---

### Task 10: Consumer wiring (out-of-tree, in gondor repo)

This task lives in the gondor repo (separate PR). Listed here so we know it's required to close the
loop.

**Files (in gondor repo):**

- Modify: `gondor/scripts/main.lua` (or equivalent mount call)
- Modify: `gondor/config.toml` or env wiring

- [ ] **Step 1: Add new sysops mount opts**

```lua
local sysops = require("sysops.mount")
sysops.mount(routes, {
  -- existing opts unchanged
  oidc = {
    issuer       = env("GONDOR_OIDC_ISSUER", "http://127.0.0.1:8080/auth"),
    client_id    = env("GONDOR_OIDC_CLIENT_ID", "sysops"),
    redirect_uri = env("GONDOR_PUBLIC_URL", "https://gondor.fcar.ai") .. "/auth/callback",
    scopes       = { "openid", "profile", "email" },
  },
  session = {
    cookie_name = "gondor_session",
    signing_key = read_secret("session-key"),
    ttl_seconds = 86400,
  },
  gateway = {
    engine_upstream = env("GONDOR_ENGINE_URL", "http://127.0.0.1:8080"),
    admin_bearer   = read_secret("engine-admin-bearer"),
  },
  authz = {
    require_zanzibar_admin = true,
    bootstrap_first_admin  = true,
  },
})
```

- [ ] **Step 2: Bind engine to localhost only**

In gondor's engine config (assay-engine.toml):

```toml
[server]
bind_addr = "127.0.0.1:8080" # was 0.0.0.0:8080
```

- [ ] **Step 3: Register sysops as an OAuth2 client of assay-auth's IdP**

Bootstrap script or one-shot SQL/CLI:

```
POST /api/v1/engine/auth/admin/oidc/clients
{ "client_id": "sysops",
  "redirect_uris": ["https://gondor.fcar.ai/auth/callback"],
  "grant_types": ["authorization_code", "refresh_token"],
  "response_types": ["code"],
  "scope": "openid profile email" }
```

- [ ] **Step 4: Restart sysops + engine, smoke test**

Manual:

- Visit `https://gondor.fcar.ai/` → 302 to /auth/login → 302 to engine /auth/authorize
- Authenticate
- Land back at `/` with cookie set
- Visit `/auth/users`, `/vault/kv`, `/workflow/`, `/engine/console` — all reachable, no token prompt

- [ ] **Step 5: Commit (in gondor repo)**

```bash
cd <gondor-repo>
git checkout -b feat/sysops-auth-gateway
git add ... && git commit -m "feat: enable sysops auth gateway (no admin-token prompt)"
```

---

## Verification

After Task 10, end-to-end check from a browser:

| Step                                                                  | Expected                                                         |
| --------------------------------------------------------------------- | ---------------------------------------------------------------- |
| Open `https://gondor.fcar.ai/` cold                                   | 302 → /auth/login → IdP login → back to `/`. Session cookie set. |
| Open `/auth/users` (sysops page)                                      | 200, lists users                                                 |
| Open `/auth/console` (dashboard SPA)                                  | 200, no token banner, user list renders                          |
| Open `/vault/console` (dashboard SPA)                                 | 200, no token banner                                             |
| Open `/workflow/` (dashboard SPA)                                     | 200, runs render                                                 |
| Open `/engine/console` (dashboard SPA)                                | 200, no token banner, panes render                               |
| Curl `127.0.0.1:8080/api/v1/engine/workflow/runs` with admin bearer   | 200 — m2m path still works                                       |
| Curl `gondor.fcar.ai/api/v1/engine/workflow/runs` no cookie no bearer | 401                                                              |
| Logout → visit `/auth/users`                                          | 302 → /auth/login                                                |

## Available primitives (verified)

assay-lua exposes these Rust-bound globals — referenced throughout the plan:

| Global      | Surface used by this plan                                                                                                                                                       |
| ----------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `http`      | `http.get/post/put/delete/patch{ url, body, headers }`                                                                                                                          |
| `json`      | `json.parse(s)`, `json.encode(t)`                                                                                                                                               |
| `crypto`    | `hmac(key, data, "sha256", raw?)`, `hash(s, "sha256")` (hex), `random(n)` (alnum), `jwt_sign(claims, key, {alg="HS256"})`, `jwt_verify(token, key, {...})`, `jwt_decode(token)` |
| `base64`    | `encode(s)`, `decode(s)` — standard base64 (NOT url-safe; sysops wraps with a 3-line url-safe variant)                                                                          |
| `assay.url` | `encode(s)`, `encode_form(t)` — RFC 3986 + form-urlencoded (stdlib `url.lua`)                                                                                                   |

Session cookie format = JWT signed with `HS256` against `opts.session.signing_key`. No bespoke HMAC
scheme needed; `crypto.jwt_sign/verify` handles it. PKCE `code_challenge` =
`b64url(hex_decode(crypto.hash(verifier, "sha256")))` — three lines of pure lua.

## Deployment shapes this plan supports

```
1. No-OIDC                    sysops not in path; engine direct w/ admin bearer.
                              (mount.lua skips auth routes if opts.oidc absent.)

2. Customer's own IdP, direct No sysops. Customer's SPA does OIDC against their IdP,
                              sends JWT to engine. Engine's external_issuers
                              accepts. assay-auth not loaded.

3. Customer's IdP via sysops  sysops.opts.oidc.issuer points at customer's IdP.
                              Sysops issues its own session cookie; proxies
                              engine calls w/ admin bearer.

4. Bundled (gondor pattern)   sysops.opts.oidc.issuer = engine's assay-auth IdP.
                              assay-auth federates upstream (e.g. Google) per
                              its existing upstream_providers config.
```

Task 8 step 3 (backward-compat path) covers shape 1. Shape 2 needs no sysops work. Shapes 3 and 4
share the same code; only configuration differs.

## Open questions before implementation

1. **`auth_sdk.zanzibar.list_admins` and `write_tuple`** — do those exist on the existing
   `sysops.auth.zanzibar` SDK, or do we need to extend it? Verify at start of Task 7 by reading
   `libs/sysops/auth/zanzibar.lua`.
2. **In-process session_store vs persistent** — v1 uses Lua-table session_store keyed by `sub`
   holding refresh tokens. Acceptable to lose refresh tokens on sysops restart (users re-login). For
   multi-replica sysops, would need to externalize to engine vault or postgres. Out of scope for v1.
3. **Route precedence** — does the existing route table resolve more-specific matches before
   wildcards? `/api/v1/engine/auth/whoami` (specific) must beat `/api/v1/engine/*` (wildcard).
   Verify in `mount.lua` glob behaviour before Task 5; if not, refactor to a Trie or explicit
   ordering.
4. **CSP** — bundled deployment should ship strict Content-Security-Policy headers. Tracking
   separately, not in this plan.

## Scope summary

```
Code change footprint
─────────────────────

  assay-engine          0 lines
  assay-auth            0 lines
  assay-dashboard       0 lines
  libs/sysops          ~800 lines added (modules + tests)
  gondor (separate)    ~40 lines (mount opts + bind config)
```

No engine restart-compat concerns. No dashboard-SPA UI changes. All four dashboard SPAs (auth,
vault, workflow, engine) work unchanged because their `api.js` already sends cookies via
`credentials: 'same-origin'` and the renderTokenBanner branch is suppressed by the `/whoami`
intercept.
