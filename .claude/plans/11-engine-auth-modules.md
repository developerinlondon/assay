# 11 — engine-auth Modules

> **STATUS — REV 2 (2026-04-22):** This plan is authoritative for auth module rationale and
> technology choices. The prior "Zanzibar SurrealDB backend" content is obsolete — the
> `ZanzibarStore` trait stays, implementations are PG18 + SQLite only, both additive features, both
> default. See plan 12 Revision log for the drop rationale (compile cost tripling + no capability
> loss). All "Phase D — Zanzibar SurrealDB impl" hours are removed; those are Phase 6 rolled into
> PG18-native tuple + recursive CTE work per plan 12c.

Add a complete authentication and authorization layer to `assay-engine`: OIDC client, full OIDC
provider (self-hosted IdP) with upstream federation, WebAuthn/passkey, Argon2 password, JWT, Biscuit
capability tokens, session management, and Google Zanzibar semantics over pluggable PG18 / SQLite
storage.

## Motivation

Consumer applications that need auth today face three unhappy options:

- Run a full identity provider (Keycloak, Zitadel, Ory Hydra + Kratos). 5+ containers, heavyweight
  ops.
- Wire up disparate libraries themselves. Every consumer re-implements session management and CSRF.
- Settle for simpler auth (opaque bearer tokens, ad-hoc authz). Works until permissions get
  relational.

`assay-engine` already ships workflow + HTTP + JWT + pluggable storage traits. Adding auth
primitives gives consumer apps a single-binary path to OIDC-delegated login plus Zanzibar-style
fine-grained authz — replacing Keycloak + SpiceDB with ~15 MB of native Rust.

## Scope — V1

Eight modules, all engine-resident (not runtime stdlib). Own IdP (provider role) with upstream
federation to Google / Apple / GitHub / any OIDC-compliant provider, plus client-side primitives.
Consumer apps get a complete self-hosted identity stack — they authenticate against _this_ IdP and
never talk to upstream providers directly.

| Module               | Crate / deps                     | Purpose                                              |
| -------------------- | -------------------------------- | ---------------------------------------------------- |
| `auth.oidc`          | `openidconnect` 4                | OIDC client — discovery, PKCE, callback, refresh     |
| `auth.oidc.provider` | `oxide-auth` + custom            | OIDC provider — IdP endpoints, consent, SSO sessions |
| `auth.passkey`       | `webauthn-rs` 0.5                | WebAuthn / FIDO2 registration and authentication     |
| `auth.password`      | `argon2` + `password-hash`       | Argon2id hashing with sensible defaults              |
| `auth.jwt`           | `jsonwebtoken` 10 (already used) | JWT issue and verify, JWKS fetch + rotation          |
| `auth.biscuit`       | `biscuit-auth` 6                 | Capability tokens — offline-verifiable, attenuable   |
| `auth.session`       | custom                           | Session cookies, CSRF, rotating IDs                  |
| `auth.zanzibar`      | custom — trait + backend impls   | Zanzibar semantics on pluggable storage              |

All eight live in `crates/assay-auth` (per plan 10). Consumer apps reach them via:

- `assay-engine` crate — in-process Rust API.
- `assay-engine` binary — HTTP + REST + SSE.
- Lua scripts in `assay` runtime — HTTP calls to an engine service, via thin wrapper modules in the
  runtime stdlib.

Not in V1: SCIM user provisioning, SAML, MFA other than passkeys (TOTP, SMS), end-user admin UI. The
admin _API_ for client and upstream- provider registration IS in scope — it's what makes the IdP
operable.

## Positioning — Ory stack equivalent, single binary

`assay-auth` targets feature parity with the Ory stack (the de-facto open-source identity + authz
reference today: Hydra + Kratos + Keto + Oathkeeper) plus capability tokens that Ory itself doesn't
ship. Same job, one binary, Rust.

| Capability                       | Ory stack                | assay-auth                                         |
| -------------------------------- | ------------------------ | -------------------------------------------------- |
| OIDC / OAuth2 provider (OP)      | Hydra                    | `auth.oidc.provider`                               |
| OIDC client (RP)                 | (app-level lib)          | `auth.oidc`                                        |
| Identity mgmt + login flow       | Kratos                   | `auth.password` + `auth.passkey` + `auth.session`  |
| Passkey / WebAuthn (FIDO2)       | Kratos                   | `auth.passkey`                                     |
| Password hashing (Argon2id)      | Kratos                   | `auth.password`                                    |
| Session mgmt + CSRF              | Kratos                   | `auth.session`                                     |
| Federated upstream (Google, etc) | Kratos                   | `auth.oidc` + IdP registry                         |
| Zanzibar ReBAC engine            | Keto                     | `auth.zanzibar` — full `check`/`expand`/`lookup_*` |
| Relation schema DSL              | Keto (custom DSL)        | SpiceDB-compatible subset parser                   |
| Consistency tokens (zookies)     | Keto                     | `auth.zanzibar` zookies                            |
| API policy enforcement gateway   | Oathkeeper               | out of scope — use Axum middleware                 |
| Capability tokens w/ attenuation | — (not provided)         | `auth.biscuit` ✨                                  |
| SSO across client apps           | Hydra session store      | `auth.oidc.provider` session registry              |
| MFA beyond passkey (TOTP, SMS)   | Kratos                   | V2                                                 |
| SCIM user provisioning           | Kratos                   | V2 or never                                        |
| SAML                             | (paid add-on / external) | V2 if asked for                                    |
| Admin UI                         | provided                 | V2 (primitives + HTTP admin API in V1)             |

**Deployment footprint:**

|                     | Ory                                   | assay-auth in assay-engine    |
| ------------------- | ------------------------------------- | ----------------------------- |
| Services to run     | 3–4 (Hydra, Kratos, Keto, Oathkeeper) | 1 (engine binary)             |
| Databases           | 1–3 (each service has own schema)     | 1 (shared Postgres or SQLite) |
| Image / binary size | ~300–450 MB (4 containers combined)   | ~30–38 MB single binary       |
| Inter-service auth  | needed (HTTP hop between services)    | in-process function calls     |
| Language / runtime  | Go                                    | Rust, single static binary    |

**Where V1 lags Ory:** MFA-beyond-passkey, SCIM, SAML, end-user admin UI. **Where V1 leads Ory:**
capability tokens (Biscuit), single-binary ops, PG18 skip-scan composite index + recursive CTEs for
Zanzibar reachability, Rust.

## Own IdP with upstream federation

Consumer apps authenticate against _this_ IdP. The IdP is the canonical identity source; upstream
providers are one authentication method among several — convenience, not dependency.

```
                ┌─────────────────────────────────────────┐
                │         Consumer applications           │
                │      (any OIDC-compliant client)        │
                └────────────────────┬────────────────────┘
                                     │  OIDC auth code flow + PKCE
                                     ▼
┌──────────────────────────────────────────────────────────────────┐
│                     assay-engine IdP                             │
│                                                                  │
│  /.well-known/openid-configuration                               │
│  /jwks.json        (rotates every N days, keeps history)         │
│                                                                  │
│  /authorize   → consent screen, user picks auth method:          │
│                 ┌────────────────┐                               │
│                 │  Local         │ → password / passkey          │
│                 └────────────────┘                               │
│                 ┌────────────────┐                               │
│                 │  Federated     │ → Google / Apple / GitHub /   │
│                 └────────────────┘   any upstream OIDC provider  │
│                                                                  │
│  /token            (authorization_code, refresh_token,           │
│                     client_credentials grants)                   │
│  /userinfo                                                       │
│  /revoke                                                         │
│  /logout + back-channel logout                                   │
│                                                                  │
│  Internal state (via UserStore / SessionStore):                  │
│    - client registry   (admin-managed: id, secret, redirect_uris)│
│    - user registry     (unified across local + federated)        │
│    - session registry  (SSO across clients, revocable)           │
│    - refresh-token registry (revocable)                          │
│    - JWKS history      (rotation without invalidating old tokens)│
└──────────────────────────────────────────────────────────────────┘
```

When a user signs in via Google, Google authenticates them; the IdP creates its own user record
linked to the upstream Google identity and issues **its own** ID token to the consumer app. Consumer
apps never see Google directly.

## Why Zanzibar

Permissions in relationship-heavy applications (documents inheriting from folders, members
inheriting from groups, viewers inheriting from owners) fit Google Zanzibar's relation-tuple model
directly:

```
can user:alice view document:x?
  ← alice is owner of document:x? or
  ← alice is viewer of folder containing document:x? or
  ← alice is member of a group that owns document:x? or ...
```

Role-based or attribute-based models require ad-hoc code for each inheritance rule. Zanzibar makes
them declarative with a uniform `check` operation.

## Pluggable Zanzibar backends

Zanzibar tuples are naturally graph-shaped:

```
object # relation @ subject [# subject_relation]
  e.g.  tree:ahmed # viewer @ user:alice
        tree:ahmed # viewer @ circle:immediate # member
```

Tuples are rows; the permission graph is walked with recursive CTEs — the pattern SpiceDB uses.
`ZanzibarStore` is a trait in `assay-domain`; implementations live in `assay-auth` behind the
`backend-postgres` and `backend-sqlite` features (both additive, both default).

### Postgres backend — SpiceDB-proven, PG18-optimised

```sql
CREATE TABLE zanzibar_tuple (
  id            UUID PRIMARY KEY DEFAULT uuidv7(),          -- PG18 built-in
  object_type   TEXT NOT NULL,
  object_id     TEXT NOT NULL,
  relation      TEXT NOT NULL,
  subject_type  TEXT NOT NULL,
  subject_id    TEXT NOT NULL,
  subject_rel   TEXT,            -- NULL = direct, set = userset
  created_at    TIMESTAMPTZ NOT NULL DEFAULT NOW(),
  UNIQUE (object_type, object_id, relation,
          subject_type, subject_id, subject_rel)
);

-- PG18 skip-scan: one composite serves forward (check) + inverse (lookup) queries
CREATE INDEX zanzibar_tuple_forward ON zanzibar_tuple
  (object_type, object_id, relation, subject_type, subject_id);
CREATE INDEX zanzibar_tuple_reverse ON zanzibar_tuple
  (subject_type, subject_id, relation);

-- check — recursive CTE over the userset DAG, depth-limited
WITH RECURSIVE walk AS (
  SELECT object_type, object_id, relation,
         subject_type, subject_id, subject_rel, 1 AS depth
    FROM zanzibar_tuple
    WHERE object_type = $1 AND object_id = $2 AND relation = $3
  UNION ALL
  SELECT z.object_type, z.object_id, z.relation,
         z.subject_type, z.subject_id, z.subject_rel, walk.depth + 1
    FROM zanzibar_tuple z
    JOIN walk ON walk.subject_type = z.object_type
             AND walk.subject_id   = z.object_id
             AND walk.subject_rel  = z.relation
    WHERE walk.depth < 50
)
SELECT EXISTS (
  SELECT 1 FROM walk
  WHERE subject_type = $4 AND subject_id = $5 AND subject_rel IS NULL
);
```

Battle-tested pattern (SpiceDB's canonical backend). Handles millions of tuples with proper indexes.
1–5 ms checks at typical depth. PG18's skip-scan means the composite index covers both forward
(`check`) and subject-leading (`lookup_*`) queries without needing two separate indexes for leading
columns.

### SQLite backend — same CTE

Recursive CTEs work identically in SQLite. Single-node and test deployments get the full Zanzibar
surface with no extra DB.

## Zanzibar internals

Internal pipeline for `check(object, permission, subject)`:

```
                 check(document:x, view, user:bob)
                        │
                        ▼
              resolve permission "view" in namespace "document"
                        │         view = owner + viewer
                        ▼
         ┌──────────── union ─────────────┐
         ▼                                ▼
check(document:x, owner, user:bob)   check(document:x, viewer, user:bob)
         │                                │
   direct tuple?                     direct tuple? or subject-set?
         ▼                                ▼
     false                       user:bob ∈ group:g1#member?
                                          │
                                          ▼
                                check(group:g1, member, user:bob) → true
         ┌──────────────────────────────────┘
         ▼
       true
```

Cycle detection via visited-set per check call. Depth limit configurable, default 50 (returns
`Err(CheckLimitExceeded)` rather than silently false).

### Consistency

Zookie tokens encode the commit time of the last write the caller observed. A subsequent `check`
with the zookie guarantees it sees at least that write. Backed by Postgres `xmin` /
`pg_snapshot_xmin` (PG) or a monotonic revision counter maintained by the write path (SQLite).
Default consistency = "minimum" (best-effort). Opt in to `at_exact_snapshot` via zookie when the
caller needs read-your-writes.

## Module API sketches

### `auth.oidc` (Rust)

```rust
let google = auth::oidc::Provider::discover("https://accounts.google.com").await?;
let (login_url, pkce) = google.authorize(AuthorizeRequest {
    client_id: env::var("GOOGLE_CLIENT_ID")?,
    redirect_uri: "https://app.example.com/callback".into(),
    scopes: vec!["openid", "email", "profile"],
})?;
// persist pkce.verifier against session, redirect user to login_url.
// on callback:
let tokens = google.exchange_code(query.code, &pkce.verifier,
                                  &env::var("GOOGLE_CLIENT_SECRET")?).await?;
let user = google.userinfo(&tokens.access_token).await?;
```

### `auth.zanzibar` (Rust)

```rust
// namespace schema (SpiceDB-compatible subset)
zanzibar.define_namespace(r#"
  definition user {}
  definition group { relation member: user }
  definition document {
    relation owner: user
    relation viewer: user | group#member
    permission view = owner + viewer
    permission edit = owner
  }
"#).await?;

zanzibar.write_tuple(Tuple::new("document:x", "owner",  "user:alice")).await?;
zanzibar.write_tuple(Tuple::new("group:g1",   "member", "user:bob")).await?;
zanzibar.write_tuple(Tuple::new("document:x", "viewer", "group:g1#member")).await?;

let allowed = zanzibar.check("document:x", "view", "user:bob",
                             Consistency::Minimum).await?;
// → true (bob is member of g1, which has viewer on document:x)
```

### Lua runtime wrappers

`assay` runtime stdlib exposes `auth.*` modules that wrap engine REST:

```lua
-- HTTP call to engine under the hood
local allowed = auth.zanzibar.check("document:x", "view", "user:bob")
```

Engine URL configured via `ASSAY_ENGINE_URL` env var or `assay.toml`. Connection pooling via reused
HTTP/2 connection. Typical call latency on localhost: 0.5–2 ms.

## Size, memory, and build cost

Engine-embedded configurations:

| Config                                      | Engine binary add |
| ------------------------------------------- | ----------------- |
| Full `auth` feature, PG18 + SQLite backends | +11–15 MB         |
| `auth` without `auth.oidc.provider`         | –3 MB             |
| `auth` without `auth.passkey`               | –2–3 MB           |
| `auth` without `auth.biscuit`               | –2–3 MB           |

Runtime memory with full auth + IdP + Zanzibar: ~35–45 MB RSS under typical load.

**`assay-engine` binary with plan 10 defaults + plan 11 full auth (PG18 + SQLite backends):** ~28–33
MB compressed, ~40 MB on disk, 50–70 MB RSS. Still a single binary. Still small compared to Keycloak
(150+ MB container), Zitadel (80+ MB image plus DB), or the Ory stack (four services).

## Phased plan

### Phase A — Foundation (~8.5 h)

| Task                                                         | Hours |
| ------------------------------------------------------------ | ----- |
| `crates/assay-auth` scaffolding, Cargo features              | 0.5   |
| `auth.session` (cookie jar, CSRF, rotating IDs)              | 2     |
| `auth.password` (Argon2id wrapper)                           | 1     |
| `auth.jwt` (extract + JWKS cache + rotation)                 | 2     |
| `auth.biscuit` (root keygen, issue, attenuate, verify)       | 2     |
| Schema migrations (user / session / credential / oidc_state) | 1     |

### Phase B — Identity flows (~9 h)

| Task                                                       | Hours |
| ---------------------------------------------------------- | ----- |
| `auth.oidc` — discovery, PKCE, callback, exchange, refresh | 3     |
| `auth.oidc` — multi-provider registry + claim mapping      | 1     |
| `auth.passkey` — start/finish register, start/finish auth  | 3     |
| Runtime-side Lua HTTP wrappers for `auth.*`                | 2     |

### Phase C — Zanzibar core (~9 h)

| Task                                             | Hours |
| ------------------------------------------------ | ----- |
| `ZanzibarStore` trait + namespace schema parser  | 2     |
| Postgres impl (recursive CTE + indexes)          | 1.5   |
| `check` with userset expansion + cycle detection | 2.5   |
| `expand` (return userset tree)                   | 1     |
| `lookup_resources` + `lookup_subjects`           | 1.5   |
| Zookies / consistency tokens                     | 0.5   |

### Phase D — REMOVED per plan 12 rev 2

Was "Zanzibar SurrealDB impl." Dropped — see plan 12 Revision log.

### Phase E — OIDC Provider (~25.5 h)

| Task                                                     | Hours |
| -------------------------------------------------------- | ----- |
| Discovery + JWKS endpoint + key rotation                 | 2     |
| `/authorize` + server-rendered consent screen            | 3     |
| `/token` (auth-code, refresh, client-credentials grants) | 3     |
| `/userinfo` + `/revoke`                                  | 1.5   |
| Client registry (admin HTTP + CLI)                       | 3     |
| Upstream federation flow (reuses `auth.oidc`)            | 2     |
| Session registry + SSO across client apps                | 3     |
| Back-channel logout                                      | 2     |
| PKCE enforcement, replay protection, rate limits         | 2     |
| Conformance test pass (OpenID Foundation where feasible) | 4     |

### Phase F — Polish (~9 h)

| Task                                                               | Hours |
| ------------------------------------------------------------------ | ----- |
| Integration tests (mock IdP, WebAuthn vectors, Zanzibar canonical) | 4     |
| Dashboard auth views (client registry, users, sessions, tuples)    | 2     |
| README feature matrix, CHANGELOG, llms.txt                         | 1     |
| Security self-pass (timing, replay, CSRF, open-redirect)           | 2     |

### Total

**~65 continuous agent-hours ≈ 8 agent-days solo.**

Parallelism: A sequential (foundation). B + C + D on separate branches after A. E begins once B's
OIDC client is done. F polishes at the end. With three concurrent agents, calendar ≈ 3 days.

## Dependencies

```toml
# crates/assay-auth/Cargo.toml

[dependencies]
openidconnect = { version = "4", optional = true }
oxide-auth = { version = "0.6", optional = true }
askama = { version = "0.15", optional = true }
webauthn-rs = { version = "0.5", optional = true }
argon2 = { version = "0.5", optional = true } # RustCrypto stable pair with password-hash 0.5; track 0.6 still RC as of Apr 2026
password-hash = { version = "0.5", optional = true }
biscuit-auth = { version = "6", optional = true }
# jsonwebtoken, sqlx come from workspace

[features]
auth = [
  "auth-oidc",
  "auth-oidc-provider",
  "auth-passkey",
  "auth-password",
  "auth-jwt",
  "auth-biscuit",
  "auth-session",
  "auth-zanzibar",
]

auth-oidc = ["dep:openidconnect"]
auth-oidc-provider = ["auth-oidc", "auth-session", "dep:oxide-auth", "dep:askama"]
auth-passkey = ["dep:webauthn-rs"]
auth-password = ["dep:argon2", "dep:password-hash"]
auth-biscuit = ["dep:biscuit-auth"]
auth-zanzibar = [] # backend selected via engine's backend-* feature

backend-postgres = ["dep:sqlx", "sqlx/postgres"]
backend-sqlite = ["dep:sqlx", "sqlx/sqlite"]
```

Backends are additive (both in default) — runtime selects one via `EngineConfig.backend`. See plan
12 Principle 3.

## Prerequisite: plan 10 · executed via plan 12

Plan 10 (assay-engine architecture) lands first. It establishes the `assay-auth` crate scaffold,
shared `assay-domain` traits, and the engine binary that wires auth modules in. Plan 10 also
documents the **`FromRef` state composition pattern** (see § "State composition") — `assay-auth`
exports `pub struct AuthCtx` and `pub fn router() -> Router<AuthCtx>`; the engine composes both into
`EngineState` via `FromRef`. Every module in this plan follows that shape.

Plan 12 (v0.13.0 execution) is the authoritative task list that sequences plans 10 + 11 into one
release. Specifically:

- Phase 4 (plan 12c) delivers auth primitives: session, password, JWT, Biscuit.
- Phase 5 (plan 12c) delivers identity flows: OIDC client, passkey, Lua runtime wrappers.
- Phase 6 (plan 12c) delivers Zanzibar core across PG18 + SQLite backends.
- Phase 7 (plan 12d) delivers the full OIDC provider.

Phase-level hour estimates in this doc (Phase A–F) are conceptual; plan 12's sub-plans reorder them
into executable task units.

## Out of scope for V1

- SCIM user provisioning — V2 or never.
- SAML — only if a consumer app specifically asks.
- MFA other than passkeys (TOTP, SMS) — V2.
- End-user admin UI — can be built on top of the primitives (the admin _API_ for client and
  upstream-provider registration IS in scope).
- Hardware security module (HSM) integration for JWKS signing keys — V2.
- Device authorization grant (`urn:ietf:params:oauth:grant-type:device_code`) — add when a consumer
  app needs TV / CLI login.

## Open decisions

1. **Engine-resident, not runtime stdlib.** Lua scripts access auth over HTTP via thin wrappers in
   the runtime stdlib. Accepted.

2. **Zanzibar backends: PG18 + SQLite.** Both compiled in by default, additive features, runtime
   selection via `EngineConfig.backend`. PG18 is the SpiceDB-proven default; SQLite gets the same
   recursive CTE semantics for embedded / dev / test deployments. SurrealDB was evaluated and
   dropped in rev 2 — see plan 12 Revision log.

3. **Session storage.** Opaque session ID + DB lookup per request, not encrypted JWE. Revocation
   matters for an auth stdlib.

4. **Biscuit vs Macaroons vs Paseto for capability tokens.** Biscuit.

   - **Macaroons** (Google Research, 2014) — "cookies with caveats." A token holds an appendable
     list of restrictions, verified via an HMAC chain. Originated the attenuation-by-appending idea.
     Weakness: HMAC-based verification means every verifier either shares the root secret with the
     issuer or calls a discharge service — no offline third-party verification.
   - **Biscuit** (Clever Cloud / Eclipse, 2021+) — the modern successor. Public-key signed, so
     third-party verifiers don't need a shared secret. Restrictions are expressed in a Datalog
     dialect (strictly more expressive than Macaroon caveats). Offline verification is the default.
   - **Paseto** — "safer JWT." No attenuation, no policy language — just a cleaner token format.
     Doesn't address the capability-delegation problem.

   Biscuit wins on verifier ergonomics (public-key, offline) and policy expressiveness (Datalog).

5. **Bundled dev-mode IdP.** Feature-gated behind `auth-dev-provider`, warns loudly if enabled in
   release builds. Lets consumers skip Google/Apple client registration during early development.

6. **Password flow kept.** Low cost, unlocks seed admin accounts and local-only deployments.

---

_Prerequisite: 10-assay-engine-architecture.md._ _Consumed by: the jeebonV3 plan (imports
`assay-engine` crate with `features = ["workflow", "auth", "backend-postgres", "backend-sqlite"]`)._
