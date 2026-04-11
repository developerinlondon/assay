## assay.certmanager

cert-manager certificate lifecycle. Certificates, issuers, ACME orders and challenges.
Client: `certmanager.client(url, token)`.

### Certificates

- `c:certificates(namespace)` → `{items}` — List certificates in namespace
- `c:certificate(namespace, name)` → cert|nil — Get certificate by name
- `c:certificate_status(namespace, name)` → `{ready, not_after, not_before, renewal_time, revision, conditions}` — Get status
- `c:is_certificate_ready(namespace, name)` → bool — Check if certificate has Ready=True condition
- `c:wait_certificate_ready(namespace, name, timeout_secs?)` → true — Wait for readiness. Default 300s.

### Issuers

- `c:issuers(namespace)` → `{items}` — List issuers in namespace
- `c:issuer(namespace, name)` → issuer|nil — Get issuer by name
- `c:is_issuer_ready(namespace, name)` → bool — Check if issuer is ready

### ClusterIssuers

- `c:cluster_issuers()` → `{items}` — List cluster-scoped issuers
- `c:cluster_issuer(name)` → issuer|nil — Get cluster issuer by name
- `c:is_cluster_issuer_ready(name)` → bool — Check if cluster issuer is ready

### Certificate Requests

- `c:certificate_requests(namespace)` → `{items}` — List certificate requests
- `c:certificate_request(namespace, name)` → request|nil — Get certificate request
- `c:is_request_approved(namespace, name)` → bool — Check if request is approved

### ACME Orders & Challenges

- `c:orders(namespace)` → `{items}` — List ACME orders
- `c:order(namespace, name)` → order|nil — Get ACME order
- `c:challenges(namespace)` → `{items}` — List ACME challenges
- `c:challenge(namespace, name)` → challenge|nil — Get ACME challenge

### Utilities

- `c:all_certificates_ready(namespace)` → `{ready, not_ready, total, not_ready_names}` — Check all certificates
- `c:all_issuers_ready(namespace)` → `{ready, not_ready, total, not_ready_names}` — Check all issuers

Example:
```lua
local cm = require("assay.certmanager")
local c = cm.client("https://k8s-api:6443", env.get("K8S_TOKEN"))
c:wait_certificate_ready("default", "my-tls-cert", 600)
local status = c:all_certificates_ready("default")
assert.eq(status.not_ready, 0)
```
