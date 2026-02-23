--- @module assay.openbao
--- @description OpenBao secrets management (vault-compatible). Alias for assay.vault.
--- @keywords openbao, vault, secrets, kv, policies, auth, transit, pki, encryption, decryption, certificate, seal, initialization, authentication, secret-engine, password, rotation

-- OpenBao alias: OpenBao is API-compatible with HashiCorp Vault.
-- Both tools can use the same client via require("assay.vault") or require("assay.openbao").
return require("assay.vault")
