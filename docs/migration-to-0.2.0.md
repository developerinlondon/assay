# Migration guide — assay-engine 0.1.x → 0.2.0

`assay-engine v0.2.0` is the **umbrella release** that turns assay into a one-binary **Temporal +
Ory replacement**. Workflows were already in v0.1.x; v0.2.0 adds a complete identity provider —
passkey, OIDC client + provider, JWT/JWKS rotation, biscuit capability tokens, Zanzibar ReBAC,
session management, an admin HTTP API, and dashboard panes for everything new.

This doc covers every required change for binary users, embedders, Lua-script consumers, and
operators upgrading from v0.1.x.

If something here isn't clear, open an issue.

## TL;DR

- **One binary still runs everything.** `assay-engine serve --config engine.toml` now serves
  workflow + auth + dashboard on a single HTTP port.
- **Storage layout changed.** PG uses per-module schemas (`engine`, `workflow`, `auth`); SQLite uses
  per-module attached files (`./data/engine.db`, `./data/workflow.db`, `./data/auth.db`).
- **Modules are runtime-controlled.** `engine.modules` (a row per module: `name`, `enabled`,
  `version`, `config`) decides what's active at boot. Compile features still control linking.
- **Auth is opt-in on upgrade.** v0.1.x deployments get the new `engine.modules` table on first boot
  of v0.2.0 but auth stays disabled until you flip it (see Upgrade path below).
- **Biscuit is built in.** Capability tokens with Datalog attenuation ship as a non-optional feature
  of `assay-auth` — no Cargo flag, always available.
- **Dashboard auth panes appear when auth is enabled.** Users / Sessions / OIDC clients / Upstream
  providers / Zanzibar / JWKS / Biscuit / Audit log.

## Scenario 1 — you ran `assay-engine` against v0.1.x storage (PG)

The schema/attach refactor (originally v0.1.2, rolled into v0.2.0) moves engine + workflow tables
into dedicated PG schemas. **The migration is idempotent:** boot v0.2.0 against your existing PG
database and the engine ALTERs the `assay`-prefixed tables into the `engine` and `workflow` schemas
on first connect. No data is rewritten; only the schema namespace changes.

```bash
# Stop the v0.1.x engine.
systemctl stop assay-engine

# Pull v0.2.0.
cargo install assay-engine --version 0.2.0
# or: docker pull ghcr.io/developerinlondon/assay-engine:0.2.0

# Boot v0.2.0 against the same PG URL — migration runs automatically.
assay-engine serve --config engine.toml
```

Auth tables (`auth.*`) are **not** created until you opt into the auth module. To turn it on:

```bash
# One-time enable via SQL (or use the admin CLI in a follow-up release).
psql "$DATABASE_URL" -c "
  INSERT INTO engine.modules (name, enabled, version, config)
  VALUES ('auth', TRUE, '0.2.0', '{}'::jsonb)
  ON CONFLICT (name) DO UPDATE SET enabled = TRUE;
"

# Restart the engine — auth schema is created and migrations run on next boot.
systemctl restart assay-engine
```

Or use the new `auto_enable_modules` config knob to make boot do this for you (handy for fresh
installs and dev):

```toml
# engine.toml
auto_enable_modules = ["auth"]
```

## Scenario 2 — you ran the v0.1.x engine against SQLite

**Active development convention: delete `./data/` and start fresh.**

```bash
rm -rf ./data/
assay-engine serve --config engine.toml
```

The v0.2.0 engine creates a fresh `./data/engine.db` and (if `auto_enable_modules = ["auth"]`)
attaches `./data/auth.db` and `./data/workflow.db` automatically.

There is no in-place migration tool for SQLite v0.1.x → v0.2.0 because the old layout used a single
file with `_assay_*` prefixes; v0.2.0 attaches one file per module and uses `<module>.<table>`
qualification. The conversion is mechanically possible but, for the active-dev cohort that's the
only reported SQLite user today, not worth the engineering cost. If you have production SQLite data
you need to preserve, file an issue.

## Scenario 3 — you embed `assay-engine` as a crate

Update your `Cargo.toml`:

```toml
[dependencies]
assay-engine = { version = "0.2", default-features = false, features = ["backend-postgres", "backend-sqlite", "auth" # NEW — pulls in assay-auth with the full feature set
  # "server",      # enable for the clap-based binary entrypoint
] }
```

The `auth` feature pulls in every auth sub-feature of `assay-auth`: `auth-session`, `auth-password`,
`auth-jwt`, `auth-oidc`, `auth-oidc-provider`, `auth-passkey`, `auth-zanzibar`. Biscuit is always
on; there is no `auth-biscuit` feature flag.

`AuthCtx` composes into your existing axum state via `axum::extract::FromRef` — same pattern as
`WorkflowCtx`. The engine's own `EngineState<S>` shows the recipe; copy that or use
`assay_engine::run` (the top-level entrypoint) to skip the wiring entirely.

## Scenario 4 — you use the assay (Lua) runtime against the engine

Drop the new auth surface in via the `assay.auth` stdlib module (ships in the v0.14.x runtime
binary):

```lua
local auth = require("assay.auth")
local c = auth.client({ engine_url = "http://localhost:3000" })

-- Password login
local sess = c:login("alice@example.com", "hunter2")

-- Whoami via session cookie
local c2 = auth.client({
  engine_url = "http://localhost:3000",
  session_cookie = sess.session_id,
})
local me = c2:whoami()

-- Zanzibar permission check
local can_read = c.zanzibar:check("doc", "doc-42", "read", "user", me.id)

-- Issue a biscuit capability token (admin endpoint)
local pem = c.biscuit:public_pem()  -- engine root key, cache locally
```

The full surface mirrors plan 12c §"v0.2.0 alignment". See `crates/assay/stdlib/auth.lua` for the
canonical wrapper.

## New auth surface (HTTP)

All routes mount under `/auth` on the engine. Highlights:

| Route                                   | Purpose                                               |
| --------------------------------------- | ----------------------------------------------------- |
| `POST /auth/login`                      | Password login → session cookie + CSRF token          |
| `DELETE /auth/session`                  | Revoke current session (logout)                       |
| `GET /auth/whoami`                      | Resolve current session → User                        |
| `POST /auth/passkey/register/start`     | Begin WebAuthn registration ceremony                  |
| `POST /auth/passkey/register/finish`    | Complete WebAuthn registration                        |
| `POST /auth/passkey/auth/start`         | Begin WebAuthn authentication ceremony                |
| `POST /auth/passkey/auth/finish`        | Complete WebAuthn authentication                      |
| `GET /.well-known/openid-configuration` | OIDC discovery (the engine is a conformant OP)        |
| `GET /.well-known/jwks.json`            | JSON Web Key Set — rotated key material               |
| `GET /auth/authorize`                   | OIDC authorization endpoint (Hydra equivalent)        |
| `POST /auth/token`                      | OIDC token endpoint (auth-code + refresh)             |
| `GET /auth/userinfo`                    | OIDC userinfo endpoint                                |
| `POST /auth/oidc/start/{slug}`          | Federated SSO via upstream provider                   |
| `POST /auth/oidc/callback/{slug}`       | Complete federated SSO                                |
| `POST /auth/biscuit/issue`              | Mint a biscuit capability token from facts + checks   |
| `POST /auth/zanzibar/check`             | Permission check (`subject ∈ relation(object)`)       |
| `POST /auth/zanzibar/expand`            | Userset tree                                          |
| `POST /auth/zanzibar/write`             | Write a relation tuple (admin)                        |
| `GET /auth/admin/auth/users`            | Admin: paginated user list                            |
| `GET /auth/admin/auth/sessions`         | Admin: paginated session list                         |
| `GET /auth/admin/auth/biscuit`          | Admin: active root key (kid + public PEM)             |
| `GET /auth/admin/auth/jwks`             | Admin: JWKS proxy                                     |
| `GET /auth/admin/auth/zanzibar/...`     | Admin: namespace browser, tuple inspector, check eval |

Admin routes require an admin api-key (configured via `auth.admin_api_keys` in `engine.toml`,
constant-time compared). User-facing routes use session cookies + CSRF.

## New tables

### `engine` schema (v0.1.2 already; rolled into v0.2.0)

| Table               | Purpose                                                           |
| ------------------- | ----------------------------------------------------------------- |
| `engine.modules`    | Per-module enable/disable + version + config blob                 |
| `engine.migrations` | Applied migrations per module (`module`, `version`, `applied_at`) |
| `engine.audit`      | Engine-level operations audit log                                 |
| `engine.instances`  | One row per running engine process — heartbeats + leader election |
| `engine.events`     | Outbox for engine-level realtime events (SSE replay)              |

### `workflow` schema (unchanged content; namespace-qualified now)

`workflow.workflows`, `workflow.history`, `workflow.tasks`, `workflow.timers`, `workflow.signals`,
`workflow.namespaces`, `workflow.workers`, `workflow.queues`, … — same shape as v0.1.x, just under
the `workflow` schema.

### `auth` schema (NEW)

| Table                           | Purpose                                                       |
| ------------------------------- | ------------------------------------------------------------- |
| `auth.users`                    | Authoritative user records                                    |
| `auth.sessions`                 | Opaque session ids + CSRF tokens + expiry                     |
| `auth.passkeys`                 | WebAuthn credentials per user                                 |
| `auth.user_upstream`            | Federated identity links (provider/subject → user_id)         |
| `auth.audit`                    | Append-only compliance log (security-restricted retention)    |
| `auth.jwks_keys`                | Rotated JWT signing keys (active + history)                   |
| `auth.biscuit_root_keys`        | Biscuit capability-token root key bootstrap (always-on)       |
| `auth.zanzibar_tuples`          | ReBAC relation tuples — Keto/SpiceDB-equivalent data model    |
| `auth.zanzibar_namespaces`      | Per-namespace schema definitions                              |
| `auth.oidc_clients`             | Registered consumer apps (the engine is the OP)               |
| `auth.upstream_providers`       | Federated identity providers (Google / Apple / GitHub / etc.) |
| `auth.oidc_authorization_codes` | Single-use codes issued at `/authorize`                       |
| `auth.oidc_refresh_tokens`      | SHA-256-hashed long-lived bearer tokens                       |
| `auth.oidc_sessions`            | SSO session registry (one row per issued id_token)            |
| `auth.oidc_consents`            | Per-(user, client) consent grants                             |
| `auth.oidc_upstream_states`     | Short-lived per-login federation state                        |

## `engine.toml` additions

```toml
# v0.2.0 — auth section + module enablement

# Flip these compiled-in modules from disabled→enabled on first boot.
# Empty by default so existing v0.1.x deployments don't get unexpected
# auth migrations on upgrade. Local-dev convenience: ["auth"].
auto_enable_modules = ["auth"]

[server]
bind_addr = "0.0.0.0:3000"
# Required for OIDC `iss`, biscuit issuer, passkey origin, federation
# callbacks. Production deployments MUST override the localhost default.
public_url = "https://auth.example.com"

[auth]
# JWT issuer + OIDC `iss` claim. Defaults to `<public_url>/auth`.
issuer = "https://auth.example.com/auth"
# JWT audience list — also used by the OIDC provider when minting access
# tokens. Defaults to [issuer].
audience = ["https://auth.example.com/auth"]
# Admin api-keys — bearer tokens that grant access to /admin/* routes.
# Empty list locks admin routes entirely.
admin_api_keys = ["sk_admin_...replace..."]

[auth.session]
# Session lifetime in seconds. Default: 30 days.
ttl_seconds = 2592000

[auth.passkey]
# Relying-party id (host, no scheme/port). Defaults to host of public_url.
rp_id = "auth.example.com"
# Human-readable label browsers show.
rp_name = "Acme Identity"

[auth.oidc_provider]
enabled = true
# Override the issuer URL. Defaults to AuthConfig::issuer.
issuer_override = "https://auth.example.com"
```

## Module enablement model

Three layers compose:

1. **Compile features (Cargo)** — decide whether the module's code is _linked_ into the binary.
   `assay-engine` defaults compile workflow + dashboard; add `--features auth` for the IdP.
2. **`engine.modules` row (DB)** — decides whether the module is _active_ at runtime. Set
   `enabled = TRUE` to run the module's migrations + mount its routes + render its dashboard panes.
3. **`engine.toml` config** — decides how the active module is _configured_ (issuer URL, session
   TTL, OIDC provider toggle, …).

Boot sequence:

1. Open engine storage; CREATE SCHEMA / open file for `engine`; run engine schema migrations.
2. `SELECT name, version, config FROM engine.modules WHERE enabled = TRUE`.
3. For each enabled module: PG `CREATE SCHEMA IF NOT EXISTS <m>` / SQLite
   `ATTACH DATABASE 'data/<m>.db' AS <m>`; run pending migrations recorded in `engine.migrations`.
4. Wire trait routing per module; mount HTTP routes; start scheduler/workers.

This makes module rollouts safe: ship the binary with the new module compiled in but disabled,
verify in staging, then flip `engine.modules.enabled` in production with no redeploy.

## Replaces what?

| Component             | Replaces                  | Notes                                           |
| --------------------- | ------------------------- | ----------------------------------------------- |
| `assay-workflow`      | Temporal                  | Same Lua/Rust API since v0.1.x                  |
| `assay-auth` session  | Ory Kratos (sessions)     | Cookie + CSRF + Argon2 password                 |
| `assay-auth` passkey  | Ory Kratos (WebAuthn)     | `webauthn-rs`-backed                            |
| `assay-auth` OIDC OP  | Ory Hydra                 | RFC 7009 revoke, RFC 7662 introspect, JWKS rot  |
| `assay-auth` Zanzibar | Ory Keto / SpiceDB        | Recursive-CTE walk on PG18 + SQLite             |
| `assay-auth` biscuit  | (Ory has nothing)         | Datalog-attenuable capability tokens — built-in |
| `assay-dashboard`     | Ory Console + Temporal UI | Single SPA, auth panes appear when auth is on   |

## Rollback

Pin to v0.1.x for the engine binary if v0.2.0 doesn't work for you:

```bash
cargo install assay-engine --version 0.1.2
# or: docker pull ghcr.io/developerinlondon/assay-engine:0.1.2
```

The schema/attach refactor was already in v0.1.2, so rolling back from v0.2.0 to v0.1.2 keeps the
new storage layout — only the auth module disappears. To roll back further (to v0.1.1 with the
single-file SQLite layout / pre-schema PG layout), file an issue; that path needs case-by-case
guidance.

## Questions

File issues against [github.com/developerinlondon/assay](https://github.com/developerinlondon/assay)
with the `migration` label. Include your old `engine.toml`, your backend type, and which scenario
above you fall under.
