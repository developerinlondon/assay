--- @module assay.ory
--- @description Convenience umbrella for the Ory stack submodules (kratos, hydra, keto, rbac). Prefer requiring the individual submodules directly (e.g. assay.ory.kratos) if you only need one.
--- @keywords ory, stack, kratos, hydra, keto, rbac, identity, oauth2, oidc, authz, zanzibar, capability
--- @quickref ory.kratos.client(opts) -> kratos client | Kratos identity management
--- @quickref ory.hydra.client(opts) -> hydra client | Hydra OAuth2 and OIDC
--- @quickref ory.keto.client(read_url, opts?) -> keto client | Keto authorization (Zanzibar-style ReBAC)
--- @quickref ory.rbac.policy(opts) -> policy | Capability-based RBAC engine over Keto
--- @quickref ory.connect(opts) -> {kratos, hydra, keto} | Build all three clients in one call

local kratos = require("assay.ory.kratos")
local hydra = require("assay.ory.hydra")
local keto = require("assay.ory.keto")
local rbac = require("assay.ory.rbac")

local M = {
  kratos = kratos,
  hydra = hydra,
  keto = keto,
  rbac = rbac,
}

-- Convenience: build all three clients from a single options table.
-- opts: {
--   kratos_public, kratos_admin,
--   hydra_public, hydra_admin,
--   keto_read, keto_write,
-- }
-- Any URL left nil produces a client for that service that's limited to the
-- other endpoint (e.g. providing only kratos_public makes a read-only Kratos client).
function M.connect(opts)
  opts = opts or {}
  local result = {}
  if opts.kratos_public or opts.kratos_admin then
    result.kratos = kratos.client({
      public_url = opts.kratos_public,
      admin_url = opts.kratos_admin,
    })
  end
  if opts.hydra_public or opts.hydra_admin then
    result.hydra = hydra.client({
      public_url = opts.hydra_public,
      admin_url = opts.hydra_admin,
    })
  end
  if opts.keto_read then
    result.keto = keto.client(opts.keto_read, { write_url = opts.keto_write })
  end
  return result
end

return M
