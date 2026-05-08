-- Auth + Zanzibar page registration. mount.lua loads this when the
-- consumer opts into active_modules = { "auth" }.

local M = {}

M.handlers = {
  auth_users             = require("pages.auth.users").page,
  auth_users_create      = require("pages.auth.users").create,
  auth_user_edit         = require("pages.auth.user_edit").page,
  auth_user_save         = require("pages.auth.user_edit").save,
  auth_user_delete       = require("pages.auth.user_edit").delete,
  auth_sessions          = require("pages.auth.sessions").page,
  auth_session_revoke    = require("pages.auth.sessions").revoke,
  auth_oidc_clients      = require("pages.auth.oidc_clients").page,
  auth_oidc_client_create = require("pages.auth.oidc_clients").create,
  auth_oidc_client_delete = require("pages.auth.oidc_clients").delete,
  auth_oidc_client_rotate = require("pages.auth.oidc_clients").rotate,
  auth_upstreams         = require("pages.auth.upstreams").page,
  auth_upstream_upsert   = require("pages.auth.upstreams").upsert,
  auth_upstream_delete   = require("pages.auth.upstreams").delete,
  auth_jwks              = require("pages.auth.jwks").page,
  auth_biscuit           = require("pages.auth.biscuit").page,
  auth_audit             = require("pages.auth.audit").page,
  zanzibar_index         = require("pages.zanzibar.index").page,
  zanzibar_tuples        = require("pages.zanzibar.tuples").page,
  zanzibar_tuples_write  = require("pages.zanzibar.tuples").write,
  zanzibar_tuples_delete = require("pages.zanzibar.tuples").delete,
  zanzibar_check         = require("pages.zanzibar.check").page,
  zanzibar_check_run     = require("pages.zanzibar.check").run,
}

function M.register(routes, url)
  local h = M.handlers
  routes.GET  = routes.GET  or {}
  routes.POST = routes.POST or {}

  routes.GET[url("/auth")]                     = function(_req) return { status = 303, headers = { Location = url("/auth/users") }, body = "" } end
  routes.GET[url("/auth/users")]               = h.auth_users
  routes.POST[url("/auth/users")]              = h.auth_users_create
  routes.GET[url("/auth/users/*/edit")]        = h.auth_user_edit
  routes.POST[url("/auth/users/*/edit")]       = h.auth_user_save
  routes.POST[url("/auth/users/*/delete")]     = h.auth_user_delete
  routes.GET[url("/auth/sessions")]            = h.auth_sessions
  routes.POST[url("/auth/sessions/*/revoke")]  = h.auth_session_revoke
  routes.GET[url("/auth/oidc-clients")]   = h.auth_oidc_clients
  routes.POST[url("/auth/oidc-clients")]  = h.auth_oidc_client_create
  routes.POST[url("/auth/oidc-clients/*")] = function(req)
    local path = (req and req.path) or ""
    if path:match("/rotate%-secret$") then
      return h.auth_oidc_client_rotate(req)
    else
      return h.auth_oidc_client_delete(req)
    end
  end
  routes.GET[url("/auth/upstreams")]      = h.auth_upstreams
  routes.POST[url("/auth/upstreams")]     = h.auth_upstream_upsert
  routes.POST[url("/auth/upstreams/*")]   = h.auth_upstream_delete
  routes.GET[url("/auth/jwks")]                = h.auth_jwks
  routes.GET[url("/auth/biscuit")]             = h.auth_biscuit
  routes.GET[url("/auth/audit")]               = h.auth_audit
  routes.GET[url("/zanzibar")]                 = h.zanzibar_index
  routes.GET[url("/zanzibar/tuples")]          = h.zanzibar_tuples
  routes.POST[url("/zanzibar/tuples")]         = h.zanzibar_tuples_write
  routes.POST[url("/zanzibar/tuples/delete")]  = h.zanzibar_tuples_delete
  routes.GET[url("/zanzibar/check")]           = h.zanzibar_check
  routes.POST[url("/zanzibar/check")]          = h.zanzibar_check_run
end

return M
