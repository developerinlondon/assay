## assay.eso

External Secrets Operator. ExternalSecrets, SecretStores, ClusterSecretStores sync status.
Client: `eso.client(url, token)`.

### ExternalSecrets

- `c:external_secrets(namespace)` → `{items}` — List ExternalSecrets
- `c:external_secret(namespace, name)` → es|nil — Get ExternalSecret by name
- `c:external_secret_status(namespace, name)` → `{ready, status, sync_hash, conditions}` — Get sync status
- `c:is_secret_synced(namespace, name)` → bool — Check if ExternalSecret is synced (Ready=True)
- `c:wait_secret_synced(namespace, name, timeout_secs?)` → true — Wait for sync. Default 60s.

### SecretStores

- `c:secret_stores(namespace)` → `{items}` — List SecretStores in namespace
- `c:secret_store(namespace, name)` → store|nil — Get SecretStore by name
- `c:secret_store_status(namespace, name)` → `{ready, conditions}` — Get store status
- `c:is_store_ready(namespace, name)` → bool — Check if SecretStore is ready

### ClusterSecretStores

- `c:cluster_secret_stores()` → `{items}` — List cluster-scoped SecretStores
- `c:cluster_secret_store(name)` → store|nil — Get ClusterSecretStore by name
- `c:is_cluster_store_ready(name)` → bool — Check if ClusterSecretStore is ready

### ClusterExternalSecrets

- `c:cluster_external_secrets()` → `{items}` — List ClusterExternalSecrets
- `c:cluster_external_secret(name)` → es|nil — Get ClusterExternalSecret by name

### Utilities

- `c:all_secrets_synced(namespace)` → `{synced, failed, total, failed_names}` — Check all ExternalSecrets
- `c:all_stores_ready(namespace)` → `{ready, not_ready, total, not_ready_names}` — Check all SecretStores

Example:
```lua
local eso = require("assay.eso")
local c = eso.client("https://k8s-api:6443", env.get("K8S_TOKEN"))
c:wait_secret_synced("default", "my-external-secret", 120)
local status = c:all_secrets_synced("default")
assert.eq(status.failed, 0)
```
