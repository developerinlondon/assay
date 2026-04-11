## assay.vault

HashiCorp Vault secrets management. KV v2, policies, auth methods, transit encryption, PKI certificates, tokens.
Client: `vault.client(url, token)`. Module helpers: `M.wait()`, `M.authenticated_client()`, `M.ensure_credentials()`, `M.assert_secret()`.

### Client Methods

- `c:read(path)` → data|nil — Read secret at path (raw Vault API path without `/v1/`)
- `c:write(path, payload)` → data|nil — Write secret to path
- `c:delete(path)` → nil — Delete secret at path
- `c:list(path)` → [string] — List keys at path

### KV v2 Secrets

- `c:kv_get(mount, key)` → `{data}`|nil — Read KV v2 secret. `mount` = engine mount (e.g. `"secrets"`)
- `c:kv_put(mount, key, data)` → result — Write KV v2 secret. `data` is a table.
- `c:kv_delete(mount, key)` → nil — Delete KV v2 secret
- `c:kv_list(mount, prefix?)` → [string] — List KV v2 keys under prefix
- `c:kv_metadata(mount, key)` → metadata|nil — Get KV v2 secret metadata

### Health & Status

- `c:health()` → `{initialized, sealed, version, ...}` — Get Vault health (works even when sealed)
- `c:seal_status()` → `{sealed, initialized, ...}` — Get seal status
- `c:is_sealed()` → bool — Check if Vault is sealed
- `c:is_initialized()` → bool — Check if Vault is initialized

### ACL Policies

- `c:policy_get(name)` → policy|nil — Get ACL policy
- `c:policy_put(name, rules)` → nil — Create or update ACL policy
- `c:policy_delete(name)` → nil — Delete ACL policy
- `c:policy_list()` → [string] — List ACL policies

### Auth Methods

- `c:auth_enable(path, type, opts?)` → nil — Enable auth method. `opts`: `{description, config}`
- `c:auth_disable(path)` → nil — Disable auth method
- `c:auth_list()` → `{path: config}` — List enabled auth methods
- `c:auth_config(path, config)` → nil — Configure auth method
- `c:auth_create_role(path, role_name, config)` → nil — Create auth role
- `c:auth_read_role(path, role_name)` → role|nil — Read auth role
- `c:auth_list_roles(path)` → [string] — List auth roles

### Secrets Engines

- `c:engine_enable(path, type, opts?)` → nil — Enable secrets engine. `opts`: `{description, config, options}`
- `c:engine_disable(path)` → nil — Disable secrets engine
- `c:engine_list()` → `{path: config}` — List enabled secrets engines
- `c:engine_tune(path, config)` → nil — Tune secrets engine configuration

### Token Management

- `c:token_create(opts?)` → `{client_token, ...}` — Create new token. `opts`: `{policies, ttl, ...}`
- `c:token_lookup(token)` → token_info|nil — Lookup token details
- `c:token_lookup_self()` → token_info|nil — Lookup current token
- `c:token_revoke(token)` → nil — Revoke a token
- `c:token_revoke_self()` → nil — Revoke current token

### Transit Encryption

- `c:transit_encrypt(key_name, plaintext)` → ciphertext|nil — Encrypt with transit engine (auto base64 encodes)
- `c:transit_decrypt(key_name, ciphertext)` → plaintext|nil — Decrypt with transit engine (auto base64 decodes)
- `c:transit_create_key(key_name, opts?)` → nil — Create transit encryption key
- `c:transit_list_keys()` → [string] — List transit keys

### PKI Certificates

- `c:pki_issue(mount, role_name, opts?)` → cert|nil — Issue certificate. `opts`: `{common_name, ttl, ...}`
- `c:pki_ca_cert(mount?)` → string — Get CA certificate PEM. `mount` defaults to `"pki"`.
- `c:pki_create_role(mount, role_name, opts?)` → nil — Create PKI role

### Module Helpers

- `M.wait(url, opts?)` → true — Wait for Vault to become healthy. `opts`: `{timeout, interval, health_path}`
- `M.authenticated_client(url, opts?)` → client — Create client using K8s secret for token. `opts`: `{secret_namespace, secret_name, secret_key, timeout}`
- `M.ensure_credentials(client, path, check_key, generator)` → creds — Check if creds exist at KV path, generate if missing
- `M.assert_secret(client, path, expected_keys)` → data — Assert secret exists with all expected keys

Example:
```lua
local vault = require("assay.vault")
local c = vault.authenticated_client("http://vault:8200")
c:kv_put("secrets", "myapp/db", {username = "admin", password = crypto.random(32)})
local creds = c:kv_get("secrets", "myapp/db")
```
