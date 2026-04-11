## assay.certmanager

cert-manager certificate lifecycle. Certificates, issuers, ACME orders and challenges.
Client: `certmanager.client(url, token)`.

### Certificates

- `c.certificates:list(namespace)` -> `{items}` -- List certificates in namespace
- `c.certificates:get(namespace, name)` -> cert|nil -- Get certificate by name
- `c.certificates:status(namespace, name)` -> `{ready, not_after, not_before, renewal_time, revision, conditions}` -- Get status
- `c.certificates:is_ready(namespace, name)` -> bool -- Check if certificate has Ready=True condition
- `c.certificates:wait_ready(namespace, name, timeout_secs?)` -> true -- Wait for readiness. Default 300s.
- `c.certificates:all_ready(namespace)` -> `{ready, not_ready, total, not_ready_names}` -- Check all certificates

### Issuers

- `c.issuers:list(namespace)` -> `{items}` -- List issuers in namespace
- `c.issuers:get(namespace, name)` -> issuer|nil -- Get issuer by name
- `c.issuers:is_ready(namespace, name)` -> bool -- Check if issuer is ready
- `c.issuers:all_ready(namespace)` -> `{ready, not_ready, total, not_ready_names}` -- Check all issuers

### ClusterIssuers

- `c.cluster_issuers:list()` -> `{items}` -- List cluster-scoped issuers
- `c.cluster_issuers:get(name)` -> issuer|nil -- Get cluster issuer by name
- `c.cluster_issuers:is_ready(name)` -> bool -- Check if cluster issuer is ready

### Certificate Requests

- `c.requests:list(namespace)` -> `{items}` -- List certificate requests
- `c.requests:get(namespace, name)` -> request|nil -- Get certificate request
- `c.requests:is_approved(namespace, name)` -> bool -- Check if request is approved

### ACME Orders & Challenges

- `c.orders:list(namespace)` -> `{items}` -- List ACME orders
- `c.orders:get(namespace, name)` -> order|nil -- Get ACME order
- `c.challenges:list(namespace)` -> `{items}` -- List ACME challenges
- `c.challenges:get(namespace, name)` -> challenge|nil -- Get ACME challenge

Example:
```lua
local cm = require("assay.certmanager")
local c = cm.client("https://k8s-api:6443", env.get("K8S_TOKEN"))
c.certificates:wait_ready("default", "my-tls-cert", 600)
local status = c.certificates:all_ready("default")
assert.eq(status.not_ready, 0)
```
