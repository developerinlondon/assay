# Vault / OpenBao compatibility facade

`assay-engine` can serve its KV store in HashiCorp Vault's dialect, so an estate already wired to
Vault or OpenBao adopts assay-vault by repointing a URL instead of rewriting every consumer.

The facade is **read-only**: `data` reads, `metadata` reads, LIST, and `sys/health`. Writes,
rotation, sealing, and token issuance stay on the native `/api/v1/vault/*` surface, where the Lua
client and the audit trail already live.

```text
ESO ExternalSecret ─┐
ansible hashi_vault ├─► GET /v1/{mount}/data/{path} ─► assay-vault KV ─► {"data":{"data":…}}
vault kv get / curl ┘        X-Vault-Token
```

## Enabling it

```toml
[vault.hashicorp_compat]
enabled = true
mount = "secrets"
```

Off by default — serving a second dialect of the secret store at the engine root is a deliberate
act. The routes appear at the server root (`https://engine.example/v1/…`), because Vault clients
hardcode `/v1/…` and cannot be told to use a prefix. Set `VAULT_ADDR` (or an ESO `server:`) to the
engine's base URL, with no path.

Compiled in through the `vault-hashicorp-compat` Cargo feature, which is part of the default `vault`
umbrella; a slim build drops it with `--no-default-features`.

## Endpoints

| Request                              | Answers                                                    |
| ------------------------------------ | ---------------------------------------------------------- |
| `GET /v1/{mount}/data/{path}`        | latest version; `?version=N` pins one                      |
| `GET /v1/{mount}/metadata/{path}`    | path-level metadata                                        |
| `LIST /v1/{mount}/metadata/{prefix}` | immediate children; `GET …?list=true` is equivalent        |
| `GET /v1/sys/health`                 | `200` unsealed, `503` sealed — unauthenticated, like Vault |

Any other method on a facade route answers `405 {"errors":["unsupported operation"]}`. Any other
`/v1/...` route is a plain 404: there is no `sys/mounts`, no `sys/seal-status`, no `auth/*`, no
token endpoint. Seal state is reported by `sys/health`, whose `sealed` field carries the same fact;
a client that reads `sys/seal-status` (`vault status`, the Lua client's `c.sys:is_sealed()`) uses
the native `/api/v1/vault/sys/seal-status` instead.

## Path mapping

The mount is a **label**, not a namespace. It names the one logical KV2 mount this engine exposes
and is stripped before the lookup; what remains is the assay KV path verbatim.

| Vault request                             | assay KV path       | native equivalent                          |
| ----------------------------------------- | ------------------- | ------------------------------------------ |
| `GET /v1/secrets/data/platform/postgres`  | `platform/postgres` | `GET /api/v1/vault/kv/platform/postgres`   |
| `GET /v1/secrets/metadata/platform/redis` | `platform/redis`    | `GET /api/v1/vault/kv-meta/platform/redis` |
| `LIST /v1/secrets/metadata/platform`      | prefix `platform/`  | `GET /api/v1/vault/kv-list/platform/`      |

A request naming any other mount is a 404 — one engine serves one mount, and quietly serving a
different one would let a typo read the wrong secret.

## Payload mapping

assay KV stores one opaque UTF-8 payload per version; KV2 hands back a JSON object under
`data.data`. The facade bridges them by parsing the stored payload:

- a JSON **object** is served as `data.data` verbatim, field by field
- anything else — a bare token, a number, an array — is served as `{"value": "<payload>"}`

So a secret written as `{"username":"app","password":"…"}` satisfies an `ExternalSecret` that names
`property: password`, and a single-string secret is still reachable at the `value` key.

## Semantics worth knowing

- **Auth.** A Vault token IS an assay token. `X-Vault-Token: <t>` is presented to the engine's
  existing admin-bearer / trusted-JWT gate as `Authorization: Bearer <t>`; there is no second token
  store, no policies, no leases. An `Authorization` header you set yourself wins.
- **Rejections speak Vault.** A missing or wrong token is `403 {"errors":["permission denied"]}`
  (Vault answers 403 for both), a missing path is `404 {"errors":[]}`.
- **Deleted versions.** A soft-deleted version answers 404 with `data.data: null` and a populated
  `deletion_time`, so a caller can tell "deleted at T" from "never existed".
- **Sealed engine.** Reads answer `503 {"errors":["Vault is sealed"]}` and `sys/health` reports
  `sealed: true` with a 503.
- **Version history.** assay records history at the path level, so a metadata read describes the
  current version only in its `versions` map. `max_versions: 0`, `cas_required: false`, and
  `delete_version_after: "0s"` are Vault's own defaults for a mount configured with none of them,
  which is exactly this one.
- **Reported version.** `sys/health` reports the `assay-vault` crate version, not a Vault version. A
  client that gates features on the Vault version string needs that check disabled (the Terraform
  provider's `skip_get_vault_version`, for instance).

## Consumers

### External Secrets Operator

```yaml
apiVersion: external-secrets.io/v1
kind: ClusterSecretStore
metadata:
  name: assay
spec:
  provider:
    vault:
      server: https://engine.example.com
      path: secrets
      version: v2
      auth:
        tokenSecretRef:
          name: assay-engine-token
          namespace: secrets
          key: token
```

Existing `ExternalSecret` objects keep their `remoteRef.key: secrets/data/platform/postgres` and
`property:` selectors unchanged; only the store's `server` moves.

### ansible

```yaml
- name: Read a credential
  ansible.builtin.set_fact:
    db: "{{ lookup('community.hashi_vault.vault_kv2_get',
      'platform/postgres',
      engine_mount_point='secrets',
      url='https://engine.example.com',
      token=assay_token).secret }}"
```

### curl

```sh
curl -sS -H "X-Vault-Token: $TOKEN" https://engine.example.com/v1/secrets/data/platform/postgres
curl -sS -H "X-Vault-Token: $TOKEN" -X LIST https://engine.example.com/v1/secrets/metadata/platform
curl -sS https://engine.example.com/v1/sys/health
```

### The assay Lua client

`assay.hashicorp.vault` talks to the facade unchanged, which makes a migration testable from the
same script that manages the old OpenBao:

```lua
local vault = require("assay.hashicorp.vault")
local c = vault.client("https://engine.example.com", token)
local secret = c.kv:get("secrets", "platform/postgres")
```
