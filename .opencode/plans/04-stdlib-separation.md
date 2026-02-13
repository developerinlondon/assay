# Plan 04: Stdlib Separation — Assay as a Generic Runtime Platform

**Status**: Draft — awaiting review
**Created**: 2026-02-12
**Context**: Assay is intended to be a key element for multiple products. The current architecture
embeds domain-specific stdlibs (k8s, vault, argocd, etc.) in the Rust binary, coupling runtime
releases to library changes and preventing independent versioning.

## Problem

1. K8s-specific code (`build_http_client()` with SA CA cert) was added to the Rust runtime in
   v0.3.3 to fix TLS errors — this violates the "generic runtime" principle
2. All 15+ stdlibs are compiled into the binary — can't version independently
3. Contributors need Rust knowledge to modify Lua libraries
4. Every stdlib change requires a full Rust rebuild + Docker image push
5. Products can't cherry-pick only the stdlibs they need

## Goal

Make Assay a **pure generic Lua 5.5 runtime** with no domain-specific knowledge. Standard libraries
live in a separate repo, are versioned independently, and are distributed as OCI artifacts.

## Architecture

```
+--------------------------------------------------------------+
| REPO 1: assay (Runtime)                                      |
| github.com/developerinlondon/assay                           |
|                                                              |
| Pure generic Lua 5.5 runtime. Zero domain knowledge.         |
|                                                              |
| Builtins:                                                    |
|   http.client({ca_cert=, timeout=, ...})   <-- NEW           |
|   http.get/post/put/patch/delete                             |
|   json, yaml, toml, regex, base64                            |
|   crypto (hash, random, jwt, hmac)                           |
|   fs (read, write, exists, glob)                             |
|   db (connect, query, execute)                               |
|   ws (websocket)                                             |
|   template (jinja-style)                                     |
|   env, log, sleep, time, assert                              |
|   require() with fs-loader                                   |
|                                                              |
| Ships as:                                                    |
|   - Static binary (~5MB, scratch image)                      |
|   - ghcr.io/developerinlondon/assay:vX.Y                     |
|   - crates.io (Rust library)                                 |
|   - GitHub releases (Linux + macOS binaries)                 |
|                                                              |
| Does NOT contain: k8s, vault, argocd, grafana, etc.          |
+--------------------------------------------------------------+
                          |
                          | require("assay.k8s")
                          | loaded from ASSAY_LIB_PATH
                          v
+--------------------------------------------------------------+
| REPO 2: assay-stdlib (Standard Libraries)                    |
| github.com/developerinlondon/assay-stdlib                    |
|                                                              |
| Pure Lua. No Rust. Independent versioning.                   |
|                                                              |
| stdlib/                                                      |
|   +-- k8s/                                                   |
|   |   +-- init.lua          K8s API, handles SA CA cert      |
|   |   +-- resources.lua     resource path registry            |
|   |   +-- wait.lua          wait_ready, polling               |
|   +-- vault/                                                 |
|   |   +-- init.lua          client, kv, policy                |
|   |   +-- pki.lua           PKI engine                        |
|   |   +-- transit.lua       transit engine                    |
|   +-- argocd/                                                |
|   |   +-- init.lua                                           |
|   +-- kargo/                                                 |
|   |   +-- init.lua                                           |
|   +-- prometheus/                                            |
|   |   +-- init.lua                                           |
|   +-- grafana/                                               |
|   |   +-- init.lua                                           |
|   +-- loki/                                                  |
|   |   +-- init.lua                                           |
|   +-- traefik/                                               |
|   |   +-- init.lua                                           |
|   +-- temporal/                                              |
|   |   +-- init.lua                                           |
|   +-- certmanager/                                           |
|   |   +-- init.lua                                           |
|   +-- crossplane/                                            |
|   |   +-- init.lua                                           |
|   +-- healthcheck/                                           |
|   |   +-- init.lua                                           |
|   +-- alertmanager/                                          |
|   |   +-- init.lua                                           |
|   +-- harbor/                                                |
|   |   +-- init.lua                                           |
|   +-- velero/                                                |
|   |   +-- init.lua                                           |
|   +-- eso/                                                   |
|   |   +-- init.lua          External Secrets Operator         |
|   +-- dex/                                                   |
|       +-- init.lua                                           |
|                                                              |
| Ships as:                                                    |
|   - OCI image (just .lua files, ~50KB)                       |
|   - ghcr.io/developerinlondon/assay-stdlib:vX.Y              |
|   - GitHub releases (tarball)                                |
|   - (Future) assay package registry                          |
+--------------------------------------------------------------+
```

## Key Design Decisions

### 1. `http.client()` — the enabler

The single most important runtime change. Without it, stdlibs cannot configure TLS independently.

```lua
-- Runtime provides:
local client = http.client({
  ca_cert = "/path/to/ca.crt",   -- optional: add root CA
  timeout = 30,                  -- optional: request timeout
})
client:get(url, { headers = { ... } })
client:post(url, body, { headers = { ... } })
client:patch(url, body, { headers = { ... } })

-- k8s/init.lua uses it internally:
local sa_ca = "/var/run/secrets/kubernetes.io/serviceaccount/ca.crt"
local function create_client()
  local opts = { timeout = 30 }
  if fs.exists(sa_ca) then
    opts.ca_cert = sa_ca
  end
  return http.client(opts)
end
local client = create_client()
```

The runtime provides the building block. The stdlib uses it for its domain.

### 2. No embedded stdlibs in the runtime binary

`require("assay.k8s")` resolves ONLY via `ASSAY_LIB_PATH`. The binary contains zero Lua files.

Why:

- Runtime can release without touching libs
- Libs can release without rebuilding Rust
- A K8s bug fix doesn't need a Rust recompile + Docker rebuild
- Contributors who know Lua but not Rust can contribute stdlibs

### 3. Stdlib inter-dependencies

Most stdlibs are independent. Only CRD-related ones depend on k8s:

```
argocd.lua      --> http.client() directly (no k8s dependency)
kargo.lua       --> http.client() directly
vault.lua       --> http.client() directly
prometheus.lua  --> http.client() directly
grafana.lua     --> http.client() directly
loki.lua        --> http.client() directly
temporal.lua    --> http.client() directly
traefik.lua     --> http.client() directly
harbor.lua      --> http.client() directly

k8s.lua         --> http.client() with SA CA cert
crossplane.lua  --> requires k8s (CRD operations)
eso.lua         --> requires k8s (ExternalSecret CRDs)
certmanager.lua --> requires k8s (Certificate CRDs)
```

### 4. k8s.patch Content-Type fix

The current `k8s.patch()` sends `application/json` which K8s rejects for PATCH operations.
With the stdlib separation, this is fixed properly:

```lua
-- k8s/init.lua
function M.patch(path, body, opts)
  opts = opts or {}
  local hdrs = auth_headers(opts.token)
  hdrs["Content-Type"] = opts.content_type or "application/merge-patch+json"
  local encoded = type(body) == "table" and json.encode(body) or body
  local resp = client:patch(url, encoded, { headers = hdrs })
  -- ...
end
```

### 5. Versioning and compatibility

```
assay runtime v0.4+    added http.client()
assay-stdlib v0.1+     requires runtime v0.4+ (uses http.client)
```

Stdlib repo declares minimum runtime version:

```yaml
# assay-stdlib/manifest.yaml
name: assay-stdlib
version: 0.1.0
requires:
  assay: ">=0.4.0"
libraries:
  - name: k8s
    description: Kubernetes API client
    requires_builtins: [http.client, fs, json, base64]
  - name: vault
    description: HashiCorp Vault / OpenBao client
    requires_builtins: [http.client, json]
  - name: argocd
    description: ArgoCD API client
    requires_builtins: [http.client, json]
```

## Distribution — Docker Image Composition

### "Batteries included" (official convenience image)

```dockerfile
# Published as ghcr.io/developerinlondon/assay-full:v1.0
FROM ghcr.io/developerinlondon/assay:v1.0 AS runtime
FROM ghcr.io/developerinlondon/assay-stdlib:v0.1.0 AS stdlib
FROM scratch
COPY --from=runtime /assay /usr/local/bin/assay
COPY --from=stdlib /lib/assay/ /lib/assay/
ENV ASSAY_LIB_PATH=/lib/assay
ENTRYPOINT ["assay"]
```

### "Cherry-pick" (product-specific)

```dockerfile
# Product only needs k8s + vault
FROM ghcr.io/developerinlondon/assay:v1.0
COPY --from=ghcr.io/developerinlondon/assay-stdlib:v0.1.0 /lib/assay/k8s /lib/assay/k8s
COPY --from=ghcr.io/developerinlondon/assay-stdlib:v0.1.0 /lib/assay/vault /lib/assay/vault
ENV ASSAY_LIB_PATH=/lib/assay
ENTRYPOINT ["assay"]
```

### "User-defined" (enterprise/community)

```dockerfile
FROM ghcr.io/developerinlondon/assay-full:v1.0
COPY ./acme-corp-lib/ /lib/assay/acme/
# Scripts can now: require("acme.deploy"), require("acme.rollback")
```

### Jeebon specifically

```yaml
# K8s Job — uses assay-full, no changes to scripts
containers:
  - name: bootstrap
    image: ghcr.io/developerinlondon/assay-full:v1.0
    command: ["assay", "/scripts/bootstrap.lua"]
```

## Testing Strategy

### Runtime repo (assay) — Rust integration tests

Tests only cover builtins. No domain-specific tests.

```
assay/tests/
  +-- http.rs           test http.client(), http.get, TLS config
  +-- json.rs           test json.parse, json.encode
  +-- crypto.rs         test hash, random, jwt
  +-- fs.rs             test read, write, exists
  +-- db.rs             test connect, query, execute
  +-- fs_require.rs     test require() with ASSAY_LIB_PATH
  +-- ...
```

### Stdlib repo (assay-stdlib) — Lua integration tests

Tests run via `assay` binary, use `http.serve()` for mock servers.

```
assay-stdlib/tests/
  +-- common/
  |   +-- mock_server.lua     uses http.serve() for test mocks
  +-- k8s/
  |   +-- test_get_secret.lua
  |   +-- test_patch.lua
  |   +-- test_wait_ready.lua
  +-- vault/
  |   +-- test_kv.lua
  |   +-- test_policy.lua
  +-- argocd/
  |   +-- test_sync.lua
  +-- ...
```

CI for stdlib repo:

```yaml
# .github/workflows/test.yml
jobs:
  test:
    steps:
      - uses: actions/checkout@v4
      - name: Download assay binary
        run: |
          gh release download --repo developerinlondon/assay -p 'assay-linux-x86_64'
          chmod +x assay-linux-x86_64
      - name: Run tests
        env:
          ASSAY_LIB_PATH: ./stdlib
        run: |
          for test in tests/**/*.lua; do
            ./assay-linux-x86_64 "$test"
          done
```

## Trade-offs

| Concern                      | Mitigation                                                    |
| ---------------------------- | ------------------------------------------------------------- |
| Two repos = more overhead    | Stdlib is pure Lua -- CI is fast, no build step               |
| Breaking runtime changes     | Manifest declares min version; CI tests against multiple      |
| Users need two images        | Publish assay-full image; most users never think about it     |
| Discovery of available libs  | README catalog; future: `assay search <name>`                 |
| Stdlib size in containers    | All 15+ stdlibs are ~50KB of Lua. Negligible.                 |
| Cold start (loading from fs) | Lua files are tiny; fs.read + compile is <1ms per file        |
| Stdlib inter-dependencies    | Shallow tree -- most only need http.client(), not other libs  |

## Module Resolution

```
require("assay.k8s")
  1. Check ASSAY_LIB_PATH/k8s/init.lua     (directory module)
  2. Check ASSAY_LIB_PATH/k8s.lua           (single file module)
  3. Error: module not found

require("assay.vault.pki")
  1. Check ASSAY_LIB_PATH/vault/pki.lua     (sub-module)
  2. Error: module not found
```

The `assay.` prefix is stripped, then resolved relative to `ASSAY_LIB_PATH`. This matches the
existing fs-require implementation.

## Implementation Phases

### Phase 1: Runtime changes (assay repo)

1. Add `http.client({ca_cert, timeout, ...})` builtin to Rust
2. Revert K8s CA cert auto-loading from `main.rs` (`build_http_client()`)
3. Remove all embedded stdlibs from `src/lua/mod.rs`
4. Keep `require("assay.*")` resolving ONLY via `ASSAY_LIB_PATH`
5. Update Dockerfile to NOT embed stdlibs
6. Move existing stdlib Rust tests to a `compat/` directory (they validate the builtins the stdlibs
   need, but don't test the stdlibs themselves)
7. Tag as v0.4.0 (breaking: stdlibs no longer embedded)

### Phase 2: Stdlib repo (new repo)

1. Create `github.com/developerinlondon/assay-stdlib`
2. Move all `.lua` files from `assay/stdlib/` to `assay-stdlib/stdlib/`
3. Update `k8s/init.lua` to use `http.client()` with SA CA cert
4. Fix `k8s.patch()` Content-Type (application/merge-patch+json)
5. Split large stdlibs into sub-modules (vault/pki.lua, vault/transit.lua)
6. Add `manifest.yaml` with version and dependency declarations
7. Set up CI: download assay binary, run Lua tests
8. Set up OCI image build: publish `ghcr.io/developerinlondon/assay-stdlib:v0.1.0`
9. Tag as v0.1.0

### Phase 3: Distribution images

1. Create `assay-full` Dockerfile that combines runtime + stdlib
2. Publish `ghcr.io/developerinlondon/assay-full:v0.4.0-stdlib0.1.0`
3. Update jeebon to use `assay-full` image
4. Verify all 9 converted jobs still work
5. Document image composition patterns for other products

### Phase 4: Future (not now)

1. `assay init` command to scaffold a project with stdlib
2. Package registry for community stdlibs
3. `assay install <package>` to download libs
4. Documentation site with stdlib API reference
5. Sub-module lazy loading for large stdlibs
