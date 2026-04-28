# Migration guide — assay-engine 0.2.x → 0.3.0

`assay-engine v0.3.0` adds the **vault module** (plan 17). One binary that previously ran workflow +
identity now also handles secrets storage, transit encryption, dynamic credentials,
biscuit-attenuated share links, sealing (Shamir / Cloud KMS), audit forwarding, and a
Bitwarden-protocol compatibility shim. New crate: `assay-vault`.

This doc covers every required change for binary users, embedders, Lua-script consumers, and
operators upgrading from v0.2.x.

## TL;DR

- **New module: `vault`.** Default-enabled when compiled in. Mounted under `/api/v1/vault/*`.
  Exposes KV v2, transit, collections, share, dynamic creds, sys (sealing).
- **New schema namespace: `vault.*`.** PG: `vault` schema with 14 tables. SQLite: attached
  `./data/vault.db`. Engine boot ATTACHes + migrates automatically.
- **Default-enabled compile feature `vault`.** Slim builds opt out with
  `--no-default-features --features "..."` (excluding `vault`).
- **Master KEK** is generated on first boot and stored in `vault.kek_metadata` with
  `sealing_method = 'plaintext'` (Phase-1 placeholder). Migrate to Shamir or Cloud KMS sealing
  before production via `POST /api/v1/vault/sys/init`.
- **HA failover tightened** to ~10s — `engine.instances` heartbeat drops from 15s to 3s, stale
  cutoff from 60s to 10s. Existing PG deployments take effect on next boot; no migration needed.

## What's new in `assay-engine` 0.3.0

### `assay-vault` crate (new)

| Surface                                                 | Plan ref | Status                                                 |
| ------------------------------------------------------- | -------- | ------------------------------------------------------ |
| KV v2 — versioned, server-decryptable secrets           | §S1      | ✅ shipped                                             |
| Transit — encrypt/decrypt without exposing key material | §S2      | ✅ shipped                                             |
| Personal vaults + collections + items + folders (E2E)   | §S4      | ✅ shipped                                             |
| Biscuit-attenuated share links (mint/redeem/revoke)     | §S5      | ✅ shipped                                             |
| Sealing — Shamir SSS init unseal                        | §S7      | ✅ shipped                                             |
| Sealing — Cloud KMS auto-unseal                         | §S7      | trait shape; AWS sigv4 + GCP JWT impls in v0.3.x       |
| Sealing — HSM via PKCS#11                               | §S7      | reserved feature flag (`vault-sealing-hsm`); opt-in    |
| Audit forwarding — webhook                              | §S8      | ✅ shipped                                             |
| Audit forwarding — syslog / S3                          | §S8      | shape ready, sinks land in v0.3.x                      |
| Dynamic credentials — Postgres                          | §S3a     | ✅ shipped                                             |
| Dynamic credentials — AWS / GCP / Kubernetes            | §S3b-d   | trait shape; AWS sigv4 + GCP JWT + K8s impls in v0.3.x |
| HA leader-lease tightening (10s failover)               | §S9      | ✅ shipped                                             |
| Bitwarden-protocol compat shim                          | §S6      | shape ready; mobile/browser/CLI compat lands in v0.3.x |

### HTTP routes mounted under `/api/v1/vault/*`

```
PUT     /kv/{path}                 store new version
GET     /kv/{path}?version=N       read latest or specific
DELETE  /kv/{path}?version=N       soft-delete
POST    /kv-destroy/{path}?version=N
POST    /kv-undelete/{path}?version=N
GET     /kv-list/{prefix}
GET     /kv-meta/{path}

POST    /transit/keys/{name}       create
GET     /transit/keys              list
POST    /transit/keys/{name}/rotate
POST    /transit/encrypt/{name}    body: { plaintext_b64 }
POST    /transit/decrypt/{name}    body: { ciphertext }

POST    /me/{user_id}              ensure personal vault
GET     /me/{user_id}              read by owner
POST    /me/{user_id}/items        create
GET     /me/{user_id}/items        list

POST    /collections               create with org_id, name, created_by
GET     /collections?org_id=…
GET     /collections/{id}
DELETE  /collections/{id}          cascades to members + items

POST    /collections/{id}/members  upsert with wrapped_key + role
GET     /collections/{id}/members
DELETE  /collections/{id}/members/{user_id}

POST    /collections/{id}/items
GET     /collections/{id}/items
GET     /items/{id}
PUT     /items/{id}
DELETE  /items/{id}

POST    /folders                   body: { vault_id XOR collection_id, name }
GET     /folders/{id}
PUT     /folders/{id}              rename
DELETE  /folders/{id}

POST    /share                     mint { target_kind, target_id, ttl_secs }
GET     /share/{token}             public — redeem
POST    /share/revoke              { revocation_id, reason }

GET     /sys/seal-status
POST    /sys/seal
POST    /sys/unseal                { share_b64 }
POST    /sys/init                  { threshold, shares_count }

POST    /dynamic/{provider}/{role}/lease  body: { ttl_secs? }
GET     /dynamic/leases?provider=…
DELETE  /dynamic/leases/{id}
```

All routes admin-key gated for v0.3.0 except `GET /share/{token}` (public — the biscuit + revocation
table ARE the access controls).

### Lua stdlib

```lua
local vault = require("assay.vault")
local c = vault.client({ engine_url = "...", admin_key = "..." })

-- KV v2
c.kv:put("api/stripe", "sk_live_xxx")
c.kv:get("api/stripe")
c.kv:list("api/")
c.kv:delete("api/stripe", 2)
c.kv:destroy("api/stripe", 2)

-- Transit
c.transit:create("logs")
local ct = c.transit:encrypt("logs", "anything")
local pt = c.transit:decrypt("logs", ct)
c.transit:rotate("logs")
```

The pre-existing HashiCorp Vault / OpenBao client moved to `require("assay.hashicorp.vault")`
(originally landed as `assay.hashicorp_vault` in 0.3.0; renamed in 0.15.4 for namespace
consistency with `assay.ory.*`). The `assay.openbao` alias still loads through the renamed module.

## Required changes

### Binary users

```toml
# engine.toml — no required edits. Vault module is enabled by default.
```

PG deployments: the `vault` schema is created automatically. SQLite deployments: `./data/vault.db`
is created automatically (or whatever `backend.path` points at).

### Embedders

`Cargo.toml`:

```toml
[dependencies]
assay-engine = "0.3"
# Default features include "vault". Opt out with:
# assay-engine = { version = "0.3", default-features = false, features = ["..."] }
```

`EngineState<S>` gains an optional `vault: Option<VaultCtx>` field behind
`#[cfg(feature = "vault")]`. Use `axum::extract::FromRef` to extract `VaultCtx` in your own handlers
— same pattern as `AuthCtx`.

### Lua-script consumers

If your scripts called `require("assay.vault")` against HashiCorp Vault / OpenBao, switch to
`require("assay.hashicorp.vault")`. The new `assay.vault` (now `assay.engine.vault`) module talks
to the assay-engine's own vault surface.

### Operators

#### Master KEK

On first v0.3.0 boot, the engine generates a fresh 32-byte KEK and persists it in
`vault.kek_metadata` with `sealing_method =
'plaintext'`. The plaintext stance is a Phase-1
placeholder; engine boot logs a WARN.

To migrate to Shamir Secret Sharing:

```bash
curl -X POST -H "Authorization: Bearer $ADMIN_KEY" \
  -d '{"threshold":3,"shares_count":5}' \
  http://engine/api/v1/vault/sys/init
# Response: { kid, shares_b64: ["...", "...", ...], threshold, shares_count }
# Distribute the shares to operators. The engine does NOT retain a copy.
```

After init, restart the engine. It will boot sealed; submit shares:

```bash
curl -X POST -H "Authorization: Bearer $ADMIN_KEY" \
  -d '{"share_b64":"..."}' \
  http://engine/api/v1/vault/sys/unseal
# Repeat with threshold shares. The last submission completes the unseal.
```

Verify:

```bash
curl -H "Authorization: Bearer $ADMIN_KEY" \
  http://engine/api/v1/vault/sys/seal-status
# { "sealed": false, "method": "shamir", ... }
```

Cloud-KMS auto-unseal (AWS / GCP) lands in v0.3.x; the trait shape is reserved. Until then, Shamir
is the production path.

#### HA failover playbook (plan §S9)

`v0.3.0` tightens the heartbeat / stale cutoff to enable sub-10s failover. The mechanism:

- Every engine pod heartbeats its row in `engine.instances` every 3s.
- The leader holds either a PG advisory lock (PG backend) or the single-row `engine.lock` (SQLite
  backend, single-tenant).
- A background sweep (also every 3s) DELETEs rows whose `last_heartbeat` is older than 10s.
- When the leader's row goes stale, the next pod that polls picks up the advisory lock and becomes
  leader.

Operator failover steps when an engine pod crashes:

1. **Detection** — observe the `engine.instances` table; the dead pod's row will disappear within
   10s. The dashboard's `/api/v1/engine/core/active-modules` endpoint reports the live set.
2. **Verify takeover** — confirm a different `instance_id` is now serving. Workflow + auth + vault
   all run on every replica.
3. **Operator action** — none required for the engine itself. If the failed pod was holding running
   workflow tasks, the next leader picks them up via the existing dispatch-wakeup loop. The vault
   module is fully active on every replica that has a `VaultCtx`; sealing state is per-instance
   memory and a fresh replica needs to be unsealed (Shamir shares re-submitted) on first boot if
   running with shamir sealing.

For PG hot-follower replication: external responsibility. Document the read-replica +
failover-to-leader procedure that matches your PG topology (Patroni, Stolon, etc.) — engine-level HA
does not manage the database itself.

## Tradeoffs locked in v0.3.0

- **No KEK rotation flow ships with v0.3.0.** The trait + storage shape support it (every wrapped
  DEK records its `kek_kid`); the re-wrap operation that sweeps every existing DEK and rewrites it
  under a new KEK is reserved for v0.3.x.
- **Bitwarden-compat shim is shape-ready, not feature-complete.** Phase 7 of plan 17 — stock BW
  mobile / browser / CLI clients speaking to assay-engine — lands in subsequent v0.3.x releases.
- **Dynamic creds: PG provider only in 0.3.0.** AWS sigv4 / GCP JWT / K8s projected-token providers
  ride on the same trait once their deps land.

## Out of scope (stays as-is)

- `assay-auth` — unchanged. Same OIDC, JWT, passkey, biscuit, Zanzibar surface as v0.2.x.
- `assay-workflow` — unchanged.
- Per-module schema layout — the `engine` / `workflow` / `auth` schemas (PG) and attachments
  (SQLite) shipped in v0.2.0 stay. `vault` joins them.
