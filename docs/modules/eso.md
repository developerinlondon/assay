## assay.eso

External Secrets Operator. ExternalSecrets, SecretStores, ClusterSecretStores sync status.
Client: `eso.client(url, token)`.

### ExternalSecrets

- `c.external_secrets:list(namespace)` -> `{items}` -- List ExternalSecrets
- `c.external_secrets:get(namespace, name)` -> es|nil -- Get ExternalSecret by name
- `c.external_secrets:status(namespace, name)` -> `{ready, status, sync_hash, conditions}` -- Get sync status
- `c.external_secrets:is_synced(namespace, name)` -> bool -- Check if ExternalSecret is synced (Ready=True)
- `c.external_secrets:wait_synced(namespace, name, timeout_secs?)` -> true -- Wait for sync. Default 60s.
- `c.external_secrets:all_synced(namespace)` -> `{synced, failed, total, failed_names}` -- Check all ExternalSecrets

### SecretStores

- `c.secret_stores:list(namespace)` -> `{items}` -- List SecretStores in namespace
- `c.secret_stores:get(namespace, name)` -> store|nil -- Get SecretStore by name
- `c.secret_stores:status(namespace, name)` -> `{ready, conditions}` -- Get store status
- `c.secret_stores:is_ready(namespace, name)` -> bool -- Check if SecretStore is ready
- `c.secret_stores:all_ready(namespace)` -> `{ready, not_ready, total, not_ready_names}` -- Check all SecretStores

### ClusterSecretStores

- `c.cluster_secret_stores:list()` -> `{items}` -- List cluster-scoped SecretStores
- `c.cluster_secret_stores:get(name)` -> store|nil -- Get ClusterSecretStore by name
- `c.cluster_secret_stores:is_ready(name)` -> bool -- Check if ClusterSecretStore is ready

### ClusterExternalSecrets

- `c.cluster_external_secrets:list()` -> `{items}` -- List ClusterExternalSecrets
- `c.cluster_external_secrets:get(name)` -> es|nil -- Get ClusterExternalSecret by name

Example:
```lua
local eso = require("assay.eso")
local c = eso.client("https://k8s-api:6443", env.get("K8S_TOKEN"))
c.external_secrets:wait_synced("default", "my-external-secret", 120)
local status = c.external_secrets:all_synced("default")
assert.eq(status.failed, 0)
```
