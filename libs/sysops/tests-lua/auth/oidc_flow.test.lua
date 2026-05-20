--! sysops auth pages — /auth/login, /auth/callback, /auth/logout.
--!
--! Each handler is exercised in isolation with a stub OIDC client
--! and a real session signer + store. The login/callback pair is
--! tested as a roundtrip: take the state planted by /auth/login and
--! feed it back into /auth/callback so they share the same store.
--!
--! Run via:
--!   LUA_PATH='libs/?.lua;libs/?/init.lua;libs/sysops/?.lua;libs/sysops/tests-lua/?.lua;;' \
--!     assay libs/sysops/tests-lua/auth/oidc_flow.test.lua

local ctx     = require("sysops.ctx")
local session = require("sysops.session")

print("[sysops.auth.flow]")

local KEY = "0123456789abcdef0123456789abcdef" -- 32 bytes

-- ---------------------------------------------------------------------
-- Stub OIDC client — only the methods our pages actually call.
-- ---------------------------------------------------------------------

local function stub_oidc(behaviours)
  behaviours = behaviours or {}
  return {
    authorize_url = function(self, state, verifier)
      return "https://idp.test/auth/authorize?state="
        .. state
        .. "&code_challenge=" .. (verifier:sub(1, 8))
    end,
    exchange_code = function(self, code, verifier)
      if behaviours.token_err then
        return nil, { status = 400, body = "bad code" }
      end
      return {
        access_token  = "AT-1",
        id_token      = "ID-1",
        refresh_token = "RT-1",
        expires_in    = 3600,
      }
    end,
    verify_id_token = function(self, id_token)
      if behaviours.verify_err then
        return nil, "bad signature"
      end
      return {
        sub   = behaviours.sub or "alice@example",
        email = behaviours.email or "alice@example",
        iss   = "https://idp.test",
        aud   = "sysops",
        exp   = os.time() + 3600,
      }
    end,
  }
end

-- ---------------------------------------------------------------------
-- Test fixture: reset ctx between tests.
-- ---------------------------------------------------------------------

local function setup_ctx(oidc_stub, signer_opts)
  ctx.oidc_client    = oidc_stub
  ctx.session_signer = session.new(signer_opts or { signing_key = KEY, ttl_seconds = 3600 })
  ctx.session_store  = session.store_new()
  return ctx
end

local function teardown_ctx()
  ctx.oidc_client    = nil
  ctx.session_signer = nil
  ctx.session_store  = nil
end

-- ---------------------------------------------------------------------
-- 1. /auth/login redirects to authorize URL + plants pending state.
-- ---------------------------------------------------------------------

do
  setup_ctx(stub_oidc())
  local login = require("pages.auth.login")

  local r = login.page({ params = { return_to = "/dashboard" } })
  assert.eq(r.status, 302, "redirects to IdP")
  assert.not_nil(r.headers.Location:find("https://idp.test/auth/authorize", 1, true),
                 "Location points at IdP authorize endpoint")
  -- Pull the state out of the Location and confirm session_store has it.
  local state = r.headers.Location:match("state=([^&]+)")
  assert.not_nil(state, "state planted in Location URL")
  local pending = ctx.session_store:take_pending(state)
  assert.not_nil(pending, "session_store has pending entry")
  assert.eq(pending.return_to, "/dashboard", "return_to roundtrips")
  teardown_ctx()
  print("  ok /auth/login redirects + plants pending state")
end

-- ---------------------------------------------------------------------
-- 2. /auth/login with no return_to defaults to "/".
-- ---------------------------------------------------------------------

do
  setup_ctx(stub_oidc())
  local login = require("pages.auth.login")
  local r = login.page({})
  local state = r.headers.Location:match("state=([^&]+)")
  local pending = ctx.session_store:take_pending(state)
  assert.eq(pending.return_to, "/", "default return_to is /")
  teardown_ctx()
  print("  ok /auth/login defaults return_to to /")
end

-- ---------------------------------------------------------------------
-- 3. /auth/callback happy-path: roundtrip with login.
-- ---------------------------------------------------------------------

do
  setup_ctx(stub_oidc({ sub = "alice@example", email = "alice@example" }))
  local login    = require("pages.auth.login")
  local callback = require("pages.auth.callback")

  local r1 = login.page({ params = { return_to = "/apps" } })
  local state = r1.headers.Location:match("state=([^&]+)")

  local r2 = callback.page({
    params = { code = "the-code", state = state },
  })
  assert.eq(r2.status, 302, "callback redirects after token exchange")
  assert.eq(r2.headers.Location, "/apps", "redirected back to original return_to")
  assert.not_nil(r2.headers["Set-Cookie"], "Set-Cookie header present")
  assert.not_nil(r2.headers["Set-Cookie"]:find("sysops_session=", 1, true),
                 "cookie has the expected name")
  assert.not_nil(r2.headers["Set-Cookie"]:find("HttpOnly", 1, true), "HttpOnly")
  assert.not_nil(r2.headers["Set-Cookie"]:find("Secure", 1, true), "Secure")
  assert.not_nil(r2.headers["Set-Cookie"]:find("SameSite=Lax", 1, true), "SameSite=Lax")

  -- The state should now be consumed.
  local consumed = ctx.session_store:take_pending(state)
  assert.eq(consumed, nil, "callback consumed the state (one-shot)")
  -- The refresh token should be stored.
  local stored = ctx.session_store:get_refresh("alice@example")
  assert.eq(stored.refresh_token, "RT-1", "refresh token stashed server-side")
  teardown_ctx()
  print("  ok /auth/callback exchanges + sets cookie + stores refresh")
end

-- ---------------------------------------------------------------------
-- 4. /auth/callback rejects unknown state.
-- ---------------------------------------------------------------------

do
  setup_ctx(stub_oidc())
  local callback = require("pages.auth.callback")
  local r = callback.page({
    params = { code = "x", state = "state-never-planted" },
  })
  assert.eq(r.status, 400, "unknown state → 400")
  assert.not_nil(r.body:find("invalid or expired state", 1, true), "human-readable error")
  teardown_ctx()
  print("  ok /auth/callback rejects unknown state")
end

-- ---------------------------------------------------------------------
-- 5. /auth/callback surfaces IdP error params.
-- ---------------------------------------------------------------------

do
  setup_ctx(stub_oidc())
  local callback = require("pages.auth.callback")
  local r = callback.page({
    params = { error = "access_denied", error_description = "user denied consent" },
  })
  assert.eq(r.status, 400, "IdP error → 400")
  assert.not_nil(r.body:find("access_denied", 1, true), "error code surfaced")
  teardown_ctx()
  print("  ok /auth/callback surfaces IdP error")
end

-- ---------------------------------------------------------------------
-- 6. /auth/callback surfaces token-endpoint failure.
-- ---------------------------------------------------------------------

do
  setup_ctx(stub_oidc({ token_err = true }))
  local login    = require("pages.auth.login")
  local callback = require("pages.auth.callback")
  local r1 = login.page({})
  local state = r1.headers.Location:match("state=([^&]+)")
  local r2 = callback.page({ params = { code = "x", state = state } })
  assert.eq(r2.status, 502, "token exchange failure → 502")
  assert.not_nil(r2.body:find("token exchange failed", 1, true), "error message")
  teardown_ctx()
  print("  ok /auth/callback surfaces token-exchange failure")
end

-- ---------------------------------------------------------------------
-- 7. /auth/callback rejects bad id_token signature.
-- ---------------------------------------------------------------------

do
  setup_ctx(stub_oidc({ verify_err = true }))
  local login    = require("pages.auth.login")
  local callback = require("pages.auth.callback")
  local r1 = login.page({})
  local state = r1.headers.Location:match("state=([^&]+)")
  local r2 = callback.page({ params = { code = "x", state = state } })
  assert.eq(r2.status, 401, "verify failure → 401")
  assert.not_nil(r2.body:find("id_token verify failed", 1, true), "error message")
  teardown_ctx()
  print("  ok /auth/callback rejects bad id_token")
end

-- ---------------------------------------------------------------------
-- 8. /auth/logout clears cookie + revokes refresh.
-- ---------------------------------------------------------------------

do
  setup_ctx(stub_oidc())
  local login    = require("pages.auth.login")
  local callback = require("pages.auth.callback")
  local logout   = require("pages.auth.logout")

  -- Establish a session.
  local r1 = login.page({})
  local state = r1.headers.Location:match("state=([^&]+)")
  local r2 = callback.page({ params = { code = "x", state = state } })
  local cookie_pair = r2.headers["Set-Cookie"]:match("^([^;]+)")
  assert.not_nil(cookie_pair, "set-cookie pair extracted")
  assert.not_nil(ctx.session_store:get_refresh("alice@example"),
                 "refresh token in store before logout")

  -- Hit /auth/logout with the cookie.
  local r3 = logout.page({ headers = { cookie = cookie_pair } })
  assert.eq(r3.status, 302, "logout redirects")
  assert.eq(r3.headers.Location, "/", "redirected to /")
  assert.not_nil(r3.headers["Set-Cookie"]:find("Max%-Age=0"), "cookie cleared via Max-Age=0")
  assert.eq(ctx.session_store:get_refresh("alice@example"), nil,
            "refresh token revoked server-side")
  teardown_ctx()
  print("  ok /auth/logout clears cookie + revokes refresh")
end

-- ---------------------------------------------------------------------
-- 9. /auth/logout without a cookie is a no-op redirect.
-- ---------------------------------------------------------------------

do
  setup_ctx(stub_oidc())
  local logout = require("pages.auth.logout")
  local r = logout.page({})
  assert.eq(r.status, 302, "logout always redirects")
  assert.eq(r.headers.Location, "/", "redirected to /")
  teardown_ctx()
  print("  ok /auth/logout no-op redirect when not signed in")
end

print("[sysops.auth.flow] ok")
