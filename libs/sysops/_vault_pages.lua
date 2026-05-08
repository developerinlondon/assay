-- libs/sysops/_vault_pages.lua
--
-- Vault page registration. mount.lua loads this when the consumer opts
-- into active_modules = { "vault" }.

local M = {}

M.handlers = {
  vault_index              = require("pages.vault.index").page,
  vault_kv                 = require("pages.vault.kv").page,
  vault_kv_put             = require("pages.vault.kv").put,
  vault_kv_delete          = require("pages.vault.kv").delete,
  vault_transit            = require("pages.vault.transit").page,
  vault_transit_create     = require("pages.vault.transit").create,
  vault_transit_rotate     = require("pages.vault.transit").rotate,
  vault_transit_encrypt    = require("pages.vault.transit").encrypt,
  vault_transit_decrypt    = require("pages.vault.transit").decrypt,
  vault_sealing            = require("pages.vault.sealing").page,
  vault_seal               = require("pages.vault.sealing").seal,
  vault_unseal             = require("pages.vault.sealing").unseal,
  vault_init               = require("pages.vault.sealing").init,
  vault_dynamic            = require("pages.vault.dynamic").page,
  vault_dynamic_lease      = require("pages.vault.dynamic").lease,
  vault_dynamic_revoke     = require("pages.vault.dynamic").revoke,
  vault_share              = require("pages.vault.share").page,
  vault_share_mint         = require("pages.vault.share").mint,
  vault_share_revoke       = require("pages.vault.share").revoke,
  vault_me                 = require("pages.vault.me").page,
  vault_collections        = require("pages.vault.collections").page,
  vault_collections_create = require("pages.vault.collections").create,
}

function M.register(routes, url)
  local h = M.handlers
  routes.GET  = routes.GET  or {}
  routes.POST = routes.POST or {}

  routes.GET[url("/vault")]                          = h.vault_index
  routes.GET[url("/vault/kv")]                       = h.vault_kv
  routes.POST[url("/vault/kv/put")]                  = h.vault_kv_put
  routes.POST[url("/vault/kv/delete")]               = h.vault_kv_delete
  routes.GET[url("/vault/transit")]                  = h.vault_transit
  routes.POST[url("/vault/transit/create")]          = h.vault_transit_create
  routes.POST[url("/vault/transit/rotate")]          = h.vault_transit_rotate
  routes.POST[url("/vault/transit/encrypt")]         = h.vault_transit_encrypt
  routes.POST[url("/vault/transit/decrypt")]         = h.vault_transit_decrypt
  routes.GET[url("/vault/sealing")]                  = h.vault_sealing
  routes.POST[url("/vault/seal")]                    = h.vault_seal
  routes.POST[url("/vault/unseal")]                  = h.vault_unseal
  routes.POST[url("/vault/init")]                    = h.vault_init
  routes.GET[url("/vault/dynamic")]                  = h.vault_dynamic
  routes.POST[url("/vault/dynamic/lease")]           = h.vault_dynamic_lease
  -- Trailing-wildcard dispatcher (the runtime URL matcher doesn't
  -- support mid-path wildcards like `/vault/dynamic/leases/*/revoke`).
  routes.POST[url("/vault/dynamic/leases/*")]        = function(req)
    local p = (req and req.path) or ""
    if p:match("/revoke$") then return h.vault_dynamic_revoke(req) end
    return { status = 404, body = "not found" }
  end
  routes.GET[url("/vault/share")]                    = h.vault_share
  routes.POST[url("/vault/share")]                   = h.vault_share_mint
  routes.POST[url("/vault/share/revoke")]            = h.vault_share_revoke
  routes.GET[url("/vault/me")]                       = h.vault_me
  routes.GET[url("/vault/collections")]              = h.vault_collections
  routes.POST[url("/vault/collections")]             = h.vault_collections_create
end

return M
