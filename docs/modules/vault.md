## assay.vault

HashiCorp Vault secrets management. KV v2, policies, auth methods, transit encryption, PKI certificates, tokens.
Client: `vault.client(url, token)`. Module helpers: `M.wait()`, `M.authenticated_client()`, `M.ensure_credentials()`, `M.assert_secret()`.

### Raw API

- `c:read(path)` -> data|nil -- Read secret at path (raw Vault API path without `/v1/`)
- `c:write(path, payload)` -> data|nil -- Write secret to path
- `c:delete(path)` -> nil -- Delete secret at path
- `c:list(path)` -> [string] -- List keys at path

### KV v2 Secrets

- `c.kv:get(mount, key)` -> `{data}`|nil -- Read KV v2 secret. `mount` = engine mount (e.g. `"secrets"`)
- `c.kv:put(mount, key, data)` -> result -- Write KV v2 secret. `data` is a table.
- `c.kv:delete(mount, key)` -> nil -- Delete KV v2 secret
- `c.kv:list(mount, prefix?)` -> [string] -- List KV v2 keys under prefix
- `c.kv:metadata(mount, key)` -> metadata|nil -- Get KV v2 secret metadata

### System / Health

- `c.sys:health()` -> `{initialized, sealed, version, ...}` -- Get Vault health (works even when sealed)
- `c.sys:seal_status()` -> `{sealed, initialized, ...}` -- Get seal status
- `c.sys:is_sealed()` -> bool -- Check if Vault is sealed
- `c.sys:is_initialized()` -> bool -- Check if Vault is initialized

### ACL Policies

- `c.policies:get(name)` -> policy|nil -- Get ACL policy
- `c.policies:create(name, rules)` -> nil -- Create or update ACL policy
- `c.policies:delete(name)` -> nil -- Delete ACL policy
- `c.policies:list()` -> [string] -- List ACL policies

### Auth Methods

- `c.auth:enable(path, type, opts?)` -> nil -- Enable auth method. `opts`: `{description, config}`
- `c.auth:disable(path)` -> nil -- Disable auth method
- `c.auth:methods()` -> `{path: config}` -- List enabled auth methods
- `c.auth:config(path, config)` -> nil -- Configure auth method
- `c.auth:create_role(path, role_name, config)` -> nil -- Create auth role
- `c.auth:get_role(path, role_name)` -> role|nil -- Read auth role
- `c.auth:list_roles(path)` -> [string] -- List auth roles

### Secrets Engines

- `c.engines:enable(path, type, opts?)` -> nil -- Enable secrets engine. `opts`: `{description, config, options}`
- `c.engines:disable(path)` -> nil -- Disable secrets engine
- `c.engines:list()` -> `{path: config}` -- List enabled secrets engines
- `c.engines:tune(path, config)` -> nil -- Tune secrets engine configuration

### Token Management

- `c.token:create(opts?)` -> `{client_token, ...}` -- Create new token. `opts`: `{policies, ttl, ...}`
- `c.token:lookup(token)` -> token_info|nil -- Lookup token details
- `c.token:lookup_self()` -> token_info|nil -- Lookup current token
- `c.token:revoke(token)` -> nil -- Revoke a token
- `c.token:revoke_self()` -> nil -- Revoke current token

### Transit Encryption

- `c.transit:encrypt(key_name, plaintext)` -> ciphertext|nil -- Encrypt with transit engine (auto base64 encodes)
- `c.transit:decrypt(key_name, ciphertext)` -> plaintext|nil -- Decrypt with transit engine (auto base64 decodes)
- `c.transit:create_key(key_name, opts?)` -> nil -- Create transit encryption key
- `c.transit:list_keys()` -> [string] -- List transit keys

### PKI Certificates

- `c.pki:issue(mount, role_name, opts?)` -> cert|nil -- Issue certificate. `opts`: `{common_name, ttl, ...}`
- `c.pki:ca_cert(mount?)` -> string -- Get CA certificate PEM. `mount` defaults to `"pki"`.
- `c.pki:create_role(mount, role_name, opts?)` -> nil -- Create PKI role

### Module Helpers

- `M.wait(url, opts?)` -> true -- Wait for Vault to become healthy. `opts`: `{timeout, interval, health_path}`
- `M.authenticated_client(url, opts?)` -> client -- Create client using K8s secret for token. `opts`: `{secret_namespace, secret_name, secret_key, timeout}`
- `M.ensure_credentials(client, path, check_key, generator)` -> creds -- Check if creds exist at KV path, generate if missing
- `M.assert_secret(client, path, expected_keys)` -> data -- Assert secret exists with all expected keys

Example:
```lua
local vault = require("assay.vault")
local c = vault.authenticated_client("http://vault:8200")
c.kv:put("secrets", "myapp/db", {username = "admin", password = crypto.random(32)})
local creds = c.kv:get("secrets", "myapp/db")
```
