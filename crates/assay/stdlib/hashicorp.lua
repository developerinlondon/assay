--- @module assay.hashicorp
--- @description Convenience umbrella for HashiCorp tooling submodules. Today only `vault` ships; future submodules (consul, nomad, boundary, terraform, packer, waypoint) will register here. Prefer requiring the individual submodules directly (e.g. `assay.hashicorp.vault`) if you only need one.
--- @keywords hashicorp, vault, consul, nomad, boundary, terraform, packer, waypoint, secrets, kv, auth

local vault = require("assay.hashicorp.vault")

return {
  vault = vault,
}
