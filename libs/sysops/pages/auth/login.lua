--! /auth/login — kick off the OIDC dance.
--!
--! Generates a fresh PKCE verifier + state, stashes them in
--! ctx.session_store under the state token (one-shot, GC'd after 5 min),
--! and 302s the browser to the IdP's authorize endpoint.

local ctx     = require("sysops.ctx")
local session = require("sysops.session")

local M = {}

function M.page(req)
  if not ctx.oidc_client or not ctx.session_store then
    return { status = 503, body = "auth gateway not configured" }
  end

  local q = (req and req.params) or {}
  -- Clamp return_to to a same-origin relative path. Anything cross-origin,
  -- protocol-relative, or slash-bypassed becomes "/".
  local return_to = session.safe_return_to(q.return_to)

  local state = crypto.random(32)
  local verifier = crypto.random(64)
  ctx.session_store:put_pending(state, {
    verifier  = verifier,
    return_to = return_to,
  })

  local authorize_url = ctx.oidc_client:authorize_url(state, verifier)
  return {
    status  = 302,
    headers = { Location = authorize_url },
  }
end

return M
