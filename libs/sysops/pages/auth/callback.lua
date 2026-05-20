--! /auth/callback — OIDC redirect target.
--!
--! Takes the (code, state) tuple the IdP sent back, looks up the
--! pending verifier+return_to, exchanges the code for tokens, verifies
--! the id_token signature/issuer/audience against the issuer's JWKS,
--! stashes the refresh_token server-side (keyed on sub), and sets the
--! session cookie. 302s back to the original return_to.

local ctx       = require("sysops.ctx")
local session   = require("sysops.session")
local bootstrap = require("pages.auth.bootstrap")

local M = {}

local function cookie_attrs(name, value, max_age)
  return string.format(
    "%s=%s; HttpOnly; Secure; SameSite=Lax; Path=/; Max-Age=%d",
    name, value, max_age
  )
end

function M.page(req)
  if not ctx.oidc_client or not ctx.session_signer or not ctx.session_store then
    return { status = 503, body = "auth gateway not configured" }
  end

  local q = (req and req.params) or {}

  -- IdP-side errors (user denied, invalid client, …) arrive as ?error=…
  if q.error then
    local desc = q.error_description or ""
    return { status = 400, body = "OIDC error: " .. q.error .. " " .. desc }
  end
  if type(q.code) ~= "string" or type(q.state) ~= "string" then
    return { status = 400, body = "missing code or state" }
  end

  local pending = ctx.session_store:take_pending(q.state)
  if not pending then
    return { status = 400, body = "invalid or expired state" }
  end

  local tokens, terr = ctx.oidc_client:exchange_code(q.code, pending.verifier)
  if terr then
    return {
      status = 502,
      body   = "token exchange failed: " .. tostring(terr.body or terr.status or ""),
    }
  end
  if type(tokens.id_token) ~= "string" then
    return { status = 502, body = "token response missing id_token" }
  end

  local claims, verr = ctx.oidc_client:verify_id_token(tokens.id_token)
  if verr then
    return { status = 401, body = "id_token verify failed: " .. tostring(verr) }
  end
  if type(claims) ~= "table" or type(claims.sub) ~= "string" then
    return { status = 401, body = "id_token missing sub claim" }
  end

  if tokens.refresh_token then
    local exp_at = tokens.expires_in and (os.time() + tokens.expires_in) or nil
    ctx.session_store:put_refresh(claims.sub, tokens.refresh_token, exp_at)
  end

  -- First-user-wins bootstrap: if no Zanzibar admins exist yet, the
  -- person currently signing in becomes admin. No-op once admins exist.
  bootstrap.maybe_grant_first_admin(claims)

  local cookie_val = ctx.session_signer:issue({
    sub   = claims.sub,
    email = claims.email,
    name  = claims.name,
  })
  return {
    status  = 302,
    headers = {
      Location     = pending.return_to or "/",
      ["Set-Cookie"] = cookie_attrs(
        ctx.session_signer.cookie_name,
        cookie_val,
        ctx.session_signer.ttl_seconds
      ),
    },
  }
end

return M
