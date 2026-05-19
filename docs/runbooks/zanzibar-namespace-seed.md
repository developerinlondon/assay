# Zanzibar namespace seed + bootstrap admin

Engine boot reads `*.json` namespace schemas from a config dir and upserts them via
`ZanzibarStore::define_namespace`. Bootstrap admin tuples are written for a named user. Both phases
idempotent on every restart.

## Flow

```
/etc/<engine>/zanzibar/*.json
     │
     ▼
engine boot ─── reads schemas
     │
     ▼
ZanzibarStore::define_namespace(&schema)   (idempotent: define replaces)
     │
     ▼
auth.zanzibar_namespaces                   (row per namespace)
     │
     ▼
if bootstrap_admin_email set:
     get_user_by_email → user.id
     write_tuple  auth:<sys>#admin → user:<id>             (engine /admin/*)
     write_tuple  workflow:main#access → user:<id>          (workflow API)
                                                            (ON CONFLICT DO NOTHING)
```

## Config (`engine.toml`)

```toml
[auth.zanzibar]
namespace_seed_dir = "/etc/gondor-engine/zanzibar"
bootstrap_admin_email = "operator@example.com" # one-time; can clear after
system_object_id = "system" # default; auth:<system_object_id>
```

| Field                   | Effect                                 | Default                |
| ----------------------- | -------------------------------------- | ---------------------- |
| `namespace_seed_dir`    | scan dir for `*.json`, upsert each     | unset → no seed        |
| `bootstrap_admin_email` | grant the bootstrap user full powers   | unset → no tuple write |
| `system_object_id`      | object id for `auth:<sys>#admin` tuple | `"system"`             |

## Reference schemas

5 ship in `crates/assay-engine/examples/zanzibar/` — copy them to your seed dir, edit if needed.

```
01-group.json          group#member
02-auth.json           auth#admin, auth#user_admin, auth#server_viewer
03-vault-path.json     vault_path#reader, vault_path#writer
04-workflow.json       workflow#triggerer, workflow#viewer
05-workflow-step.json  workflow_step#approver
```

## What ends up gated by the bootstrap tuples

```
auth:system#admin       → require_role_for("auth", "system", "admin")
                          on every /api/v1/engine/auth/admin/* route
                          (users, sessions, oidc clients, zanzibar,
                           biscuit, jwks, audit)

workflow:main#access    → workflow_gate_middleware on every
                          /api/v1/engine/workflow/* route
                          (namespace="main" default; override with
                           ?namespace= query param + matching tuple)
```

Other modules (vault paths, workflow triggers, step approvals) use their own zanzibar relations —
those tuples are operator-written via `POST /admin/zanzibar/tuples` or the sysops `/zanzibar/tuples`
page; the bootstrap doesn't pre-seed them.

## Bootstrap chicken-and-egg

If `auth.users` is empty (e.g. invite-only + first boot), the bootstrap will log a warning and skip
— the user must exist first. Two ways to break the cycle:

```
A. flip auto_provision=true once
     sign in via Google → user row created → flip back to false → restart
B. invite via admin API directly
     curl -X POST /api/v1/engine/auth/admin/users \
       -H "Authorization: Bearer $ADMIN_KEY" \
       -d '{"email":"operator@example.com","display_name":"Op"}'
     restart engine
```

After either, the bootstrap tuple lands on the next boot.

## Verifying

```bash
sqlite3 /var/lib/<engine>/auth.db \
  'SELECT name FROM zanzibar_namespaces'
# → auth, group, vault_path, workflow, workflow_step (+ vault crate's set)

sqlite3 /var/lib/<engine>/auth.db \
  'SELECT object_type, object_id, relation, subject_id FROM zanzibar_tuples'
# → auth     | system | admin  | usr_…
# → workflow | main   | access | usr_…
```

## Removing / rotating the bootstrap user

Bootstrap is one-shot per `(email, namespace, object, relation)`. To revoke admin:

```bash
curl -X DELETE /api/v1/engine/auth/admin/zanzibar/tuples \
  -H "Authorization: Bearer $ADMIN_KEY" \
  -d '{"object_type":"auth","object_id":"system","relation":"admin",
       "subject_type":"user","subject_id":"usr_<id>"}'
```

Clearing `bootstrap_admin_email` from config prevents future bootstrap runs from re-writing the
tuple. Existing tuples persist until deleted.
