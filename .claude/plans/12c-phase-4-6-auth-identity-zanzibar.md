# 12c — Phases 4 + 5 + 6 — Auth primitives, identity flows, Zanzibar core

> Sub-plan of [12-v0.13.0-execution.md](./12-v0.13.0-execution.md). Prerequisites: Phases 0–2 from
> [12a](./12a-phase-0-1-workspace-and-state.md). Can run in parallel with Phase 3 in
> [12b](./12b-phase-2-3-workflow-engine.md).
>
> **Source of truth for module rationale:**
> [11-engine-auth-modules.md](./11-engine-auth-modules.md). This sub-plan is the _task list_; plan
> 11 explains _why_ each choice (Biscuit vs Macaroons, `openidconnect` vs hand-rolled, Zanzibar
> backend selection, etc.).

## v0.1.2 alignment (read first)

Plan 12 was drafted before assay-engine v0.1.2 introduced the schema/attach storage model
([14-v0.13.2-engine-schemas.md](./14-v0.13.2-engine-schemas.md)). The DDL below is now
schema-qualified — all auth tables live in the `auth` schema (PG) / attached `auth` database
(SQLite, file `data/auth.db` by default). Carry these conventions through every task in this plan:

- **Schema-qualified naming**: `auth.users`, `auth.sessions`, `auth.zanzibar_tuples` (plural),
  `auth.zanzibar_namespaces` (plural), `auth.passkeys`, `auth.user_upstream`, `auth.audit`,
  `auth.jwks_keys`. No `_assay_` prefix.
- **Migration tracker**: one shared `engine.migrations` table
  (`module TEXT, version INTEGER,
  applied_at TIMESTAMPTZ`). Auth records its migrations under
  `module = 'auth'`. Replaces the per-module `_assay_auth_migrations` references in earlier drafts.
- **Boot lifecycle**: schema/file is created at boot when `engine.modules` shows auth enabled
  (`SELECT enabled FROM engine.modules WHERE name = 'auth'`). Compile-time features still control
  whether auth code is _linked_; runtime `engine.modules` controls whether it's _active_.
- **Compliance audit log**: new `auth.audit` table (append-only, long retention,
  security-restricted). Distinct from `engine.audit` (engine-level operations) and from any auth
  real-time event stream.
- **Auth does NOT write to `engine.events`.** Auth is independent at the event level. If real-time
  dashboard visibility into auth activity is wanted, auth uses its own NOTIFY channel on
  `auth.audit` (or a small `auth.outbox` table introduced if needed).
- **Biscuit (task 4.5) becomes opt-in**, not in the default `auth` meta-feature. Cargo flag
  `auth-biscuit` exists but isn't pulled in by default. Saves ~2h in phase 4. Re-enable later if a
  use case actually needs offline verification or attenuation. (jeebon's planned use cases — share
  links, delegated upload, worker caps, cross-app delegation — are equivalently served by a
  `share_links` table + scoped JWTs from `auth.jwt` + OAuth scopes from the OIDC provider in phase
  7.)
- **Atomic transactions across schemas/attached DBs are preserved** by the v0.1.2 model. Signup
  atomically inserts `auth.users` + `auth.passkeys` + initial `auth.zanzibar_tuples` in one
  transaction. Cross-module FKs (`workflow.workflows.created_by` REFERENCES `auth.users(id)`) work
  since both schemas live in one DB.
- **Default SQLite path**: `./data/auth.db`. Configurable via `[backend].data_dir` in engine.toml.
- **Prerequisite**: assay-engine v0.1.2 must ship before phase 4 starts.

**Phase 4 goal:** Auth foundations built — session cookie jar + CSRF, Argon2 password hashing, JWT
issue/verify with JWKS rotation, Biscuit capability tokens. Each is a small, focused module in
`assay-auth`. No HTTP handlers yet; just library surface.

**Phase 5 goal:** Identity flows work end-to-end against mock providers — OIDC client (login with
Google-like upstream), WebAuthn/passkey register + auth, Lua runtime wrappers that call
`assay-engine` over HTTP.

**Phase 6 goal:** Zanzibar trait defined in `assay-domain`; PG18 + SQLite backends operational
(recursive-CTE walk); `check`, `expand`, `lookup_resources`, `lookup_subjects` all correct on both
backends. Both backends are additive features (default includes both) with runtime selection via
`EngineConfig.backend`.

---

## Phase 4 — Auth primitives

### Task 4.1: `assay-auth` Cargo setup + feature matrix

**Files:**

- Modify: `crates/assay-auth/Cargo.toml`
- Modify: `crates/assay-auth/src/lib.rs`

- [ ] **Step 1: Replace scaffold Cargo.toml**

```toml
[package]
name = "assay-auth"
version = "0.1.0"
edition = "2024"
license = "Apache-2.0"
repository = "https://github.com/developerinlondon/assay"
description = "Authentication, OIDC (client + provider), passkey, Argon2, JWT, Biscuit capability tokens, session management, and Zanzibar-style authorization for assay-engine."
categories = ["authentication", "cryptography", "asynchronous"]
keywords = ["oidc", "zanzibar", "passkey", "biscuit", "auth"]

[features]
default = [
  "auth",
  "backend-postgres",
  "backend-sqlite",
]

# Meta-feature pulling every module in.
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

auth-oidc = ["dep:openidconnect", "auth-session"]
auth-oidc-provider = ["auth-oidc", "dep:oxide-auth", "dep:askama"]
auth-passkey = ["dep:webauthn-rs", "auth-session"]
auth-password = ["dep:argon2", "dep:password-hash"]
auth-jwt = ["dep:jsonwebtoken"]
auth-biscuit = ["dep:biscuit-auth"]
auth-session = []
auth-zanzibar = []

backend-postgres = ["dep:sqlx", "sqlx/postgres"]
backend-sqlite = ["dep:sqlx", "sqlx/sqlite"]

[dependencies]
assay-domain = { path = "../assay-domain", version = "0.1" }

# Shared
serde = { version = "1", features = ["derive"] }
serde_json = "1"
anyhow = "1"
thiserror = "2"
tracing = "0.1"
tokio = { version = "1", features = ["sync", "time", "rt"] }
async-trait = "0.1"
chrono = { version = "0.4", default-features = false, features = ["std", "clock", "serde"] }
rand = "0.9"
data-encoding = "2"
url = "2"
utoipa = "5"
axum = "0.8"
cookie = "0.18"

# Feature-gated
openidconnect = { version = "4", optional = true }
oxide-auth = { version = "0.6", optional = true }
askama = { version = "0.12", optional = true }
webauthn-rs = { version = "0.5", optional = true }
argon2 = { version = "0.5", optional = true }
password-hash = { version = "0.5", optional = true }
jsonwebtoken = { version = "10", optional = true, features = ["rust_crypto"] }
biscuit-auth = { version = "6", optional = true }

sqlx = { version = "0.8", features = ["runtime-tokio-rustls", "any"], optional = true }

# HTTP client for OIDC discovery / userinfo / JWKS fetch
reqwest = { version = "0.13", default-features = false, features = ["json", "rustls"] }

[dev-dependencies]
rstest = "0.26"
testcontainers = "0.27"
testcontainers-modules = { version = "0.15", features = ["postgres"] }
tempfile = "3"
tokio = { version = "1", features = ["full"] }
wiremock = "0.6"
```

- [ ] **Step 2: Stub lib.rs with module layout**

```rust
// crates/assay-auth/src/lib.rs
//! Auth layer for assay-engine — OIDC client + provider, passkey,
//! Argon2 password, JWT, Biscuit capability tokens, session mgmt,
//! and Zanzibar-style ReBAC.
//!
//! Module boundaries and rationale: see plan 11.

pub mod error;

#[cfg(feature = "auth-session")]
pub mod session;

#[cfg(feature = "auth-password")]
pub mod password;

#[cfg(feature = "auth-jwt")]
pub mod jwt;

#[cfg(feature = "auth-biscuit")]
pub mod biscuit;

#[cfg(feature = "auth-oidc")]
pub mod oidc;

#[cfg(feature = "auth-oidc-provider")]
pub mod oidc_provider;

#[cfg(feature = "auth-passkey")]
pub mod passkey;

#[cfg(feature = "auth-zanzibar")]
pub mod zanzibar;

pub mod store;
pub mod ctx;
pub mod router;

pub use ctx::AuthCtx;
pub use router::router;
```

- [ ] **Step 3: Define `AuthCtx` skeleton**

```rust
// crates/assay-auth/src/ctx.rs
use std::sync::Arc;

use crate::store::{SessionStore, UserStore};
#[cfg(feature = "auth-zanzibar")]
use crate::store::ZanzibarStore;
#[cfg(feature = "auth-jwt")]
use crate::jwt::JwtConfig;

#[derive(Clone)]
pub struct AuthCtx {
    pub users: Arc<dyn UserStore>,
    pub sessions: Arc<dyn SessionStore>,
    #[cfg(feature = "auth-zanzibar")]
    pub zanzibar: Arc<dyn ZanzibarStore>,
    #[cfg(feature = "auth-jwt")]
    pub jwt: JwtConfig,
    // oidc provider fields added in task 5.1
}
```

- [ ] **Step 4: Define error enum**

```rust
// crates/assay-auth/src/error.rs
#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error("invalid credentials")]
    InvalidCredentials,
    #[error("session not found or expired")]
    SessionNotFound,
    #[error("csrf token mismatch")]
    CsrfMismatch,
    #[error("jwt verification failed: {0}")]
    Jwt(String),
    #[error("zanzibar depth limit exceeded")]
    ZanzibarDepth,
    #[error("zanzibar cycle detected")]
    ZanzibarCycle,
    #[error("oidc error: {0}")]
    Oidc(String),
    #[error("passkey error: {0}")]
    Passkey(String),
    #[error("backend: {0}")]
    Backend(#[from] anyhow::Error),
}

pub type Result<T> = std::result::Result<T, Error>;
```

- [ ] **Step 5: Router stub**

```rust
// crates/assay-auth/src/router.rs
use axum::Router;
use crate::AuthCtx;

/// Auth routes. Mounted at engine root by assay-engine.
/// Routes land as task 4.* / 5.* / Phase 7 complete their flows.
pub fn router() -> Router<AuthCtx> {
    Router::new()
    // Individual module routers merged as they're implemented.
}
```

- [ ] **Step 6: Store traits stub**

```rust
// crates/assay-auth/src/store/mod.rs
pub mod types;
pub use types::*;

#[async_trait::async_trait]
pub trait UserStore: Send + Sync + 'static {
    async fn create_user(&self, user: &User) -> anyhow::Result<()>;
    async fn get_user_by_id(&self, id: &str) -> anyhow::Result<Option<User>>;
    async fn get_user_by_email(&self, email: &str) -> anyhow::Result<Option<User>>;
    async fn update_user(&self, user: &User) -> anyhow::Result<()>;
    // Password credentials
    async fn set_password_hash(&self, user_id: &str, hash: &str) -> anyhow::Result<()>;
    async fn get_password_hash(&self, user_id: &str) -> anyhow::Result<Option<String>>;
    // Passkey credentials
    async fn list_passkeys(&self, user_id: &str) -> anyhow::Result<Vec<PasskeyCred>>;
    async fn add_passkey(&self, user_id: &str, cred: &PasskeyCred) -> anyhow::Result<()>;
    async fn remove_passkey(&self, credential_id: &[u8]) -> anyhow::Result<bool>;
    // Upstream federation links
    async fn link_upstream(&self, user_id: &str, provider: &str, subject: &str) -> anyhow::Result<()>;
    async fn get_user_by_upstream(&self, provider: &str, subject: &str) -> anyhow::Result<Option<User>>;
}

#[async_trait::async_trait]
pub trait SessionStore: Send + Sync + 'static {
    async fn create(&self, session: &Session) -> anyhow::Result<()>;
    async fn get(&self, id: &str) -> anyhow::Result<Option<Session>>;
    async fn delete(&self, id: &str) -> anyhow::Result<bool>;
    async fn list_for_user(&self, user_id: &str) -> anyhow::Result<Vec<Session>>;
    async fn delete_for_user(&self, user_id: &str) -> anyhow::Result<u64>;
    async fn purge_expired(&self, now: f64) -> anyhow::Result<u64>;
}

#[cfg(feature = "auth-zanzibar")]
pub use crate::zanzibar::ZanzibarStore;
```

```rust
// crates/assay-auth/src/store/types.rs
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct User {
    pub id: String,
    pub email: Option<String>,
    pub email_verified: bool,
    pub display_name: Option<String>,
    pub created_at: f64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PasskeyCred {
    pub credential_id: Vec<u8>,
    pub public_key: Vec<u8>,
    pub sign_count: u32,
    pub transports: Vec<String>,
    pub created_at: f64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Session {
    pub id: String,
    pub user_id: String,
    pub csrf_token: String,
    pub created_at: f64,
    pub expires_at: f64,
    pub ip_hash: Option<String>,
    pub user_agent_hash: Option<String>,
}
```

- [ ] **Step 7: Verify per-feature build**

```bash
cargo check -p assay-auth --no-default-features
cargo check -p assay-auth --no-default-features --features auth
cargo check -p assay-auth --no-default-features --features "auth-session auth-password"
cargo check -p assay-auth --no-default-features --features "auth backend-postgres"
cargo check -p assay-auth
```

- [ ] **Step 8: Commit**

```bash
git add -A
git commit -m "feat(auth): Cargo feature matrix + traits + ctx skeleton"
```

---

### Task 4.2: Session module

**Files:** `crates/assay-auth/src/session.rs` + tests.

Plan 11 reference: "auth.session" — opaque session ID + DB lookup per request, not encrypted JWE.
Revocation matters for an auth stdlib.

- [ ] **Step 1: Session API sketch**

```rust
// crates/assay-auth/src/session.rs
use rand::RngCore;
use std::sync::Arc;
use crate::store::{Session, SessionStore};
use crate::error::{Error, Result};

pub const SESSION_COOKIE: &str = "assay_session";
pub const CSRF_COOKIE: &str = "assay_csrf";
pub const SESSION_TTL_SECS: u64 = 60 * 60 * 24 * 7; // 7 days

pub struct SessionManager {
    pub store: Arc<dyn SessionStore>,
    pub ttl_secs: u64,
}

impl SessionManager {
    pub fn new(store: Arc<dyn SessionStore>) -> Self {
        Self { store, ttl_secs: SESSION_TTL_SECS }
    }

    pub async fn create_for_user(&self, user_id: &str) -> Result<Session> {
        let id = random_id(32);
        let csrf = random_id(32);
        let now = now_secs();
        let session = Session {
            id: id.clone(),
            user_id: user_id.to_string(),
            csrf_token: csrf,
            created_at: now,
            expires_at: now + self.ttl_secs as f64,
            ip_hash: None,
            user_agent_hash: None,
        };
        self.store.create(&session).await?;
        Ok(session)
    }

    pub async fn validate(&self, session_id: &str) -> Result<Session> {
        let s = self.store.get(session_id).await?
            .ok_or(Error::SessionNotFound)?;
        if s.expires_at < now_secs() {
            self.store.delete(session_id).await?;
            return Err(Error::SessionNotFound);
        }
        Ok(s)
    }

    pub fn assert_csrf(&self, session: &Session, provided: &str) -> Result<()> {
        if !ct_eq(session.csrf_token.as_bytes(), provided.as_bytes()) {
            return Err(Error::CsrfMismatch);
        }
        Ok(())
    }

    pub async fn revoke(&self, session_id: &str) -> Result<bool> {
        Ok(self.store.delete(session_id).await?)
    }

    pub async fn revoke_all_for_user(&self, user_id: &str) -> Result<u64> {
        Ok(self.store.delete_for_user(user_id).await?)
    }

    pub async fn rotate(&self, old_session_id: &str) -> Result<Session> {
        let old = self.validate(old_session_id).await?;
        let new = self.create_for_user(&old.user_id).await?;
        self.store.delete(old_session_id).await?;
        Ok(new)
    }
}

fn random_id(byte_len: usize) -> String {
    let mut buf = vec![0u8; byte_len];
    rand::rng().fill_bytes(&mut buf);
    data_encoding::BASE64URL_NOPAD.encode(&buf)
}

fn now_secs() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64()
}

fn ct_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() { return false; }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b) { diff |= x ^ y; }
    diff == 0
}
```

- [ ] **Step 2: Cookie extractor + middleware**

`crates/assay-auth/src/session/middleware.rs` — axum middleware that extracts `Session` from cookies
into a request extension. Rotation on privilege escalation is the caller's responsibility.

- [ ] **Step 3: Write unit tests**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};
    use std::collections::HashMap;

    struct MemStore(Mutex<HashMap<String, Session>>);
    #[async_trait::async_trait]
    impl SessionStore for MemStore {
        async fn create(&self, s: &Session) -> anyhow::Result<()> {
            self.0.lock().unwrap().insert(s.id.clone(), s.clone()); Ok(())
        }
        async fn get(&self, id: &str) -> anyhow::Result<Option<Session>> {
            Ok(self.0.lock().unwrap().get(id).cloned())
        }
        async fn delete(&self, id: &str) -> anyhow::Result<bool> {
            Ok(self.0.lock().unwrap().remove(id).is_some())
        }
        async fn list_for_user(&self, _u: &str) -> anyhow::Result<Vec<Session>> { Ok(vec![]) }
        async fn delete_for_user(&self, _u: &str) -> anyhow::Result<u64> { Ok(0) }
        async fn purge_expired(&self, _n: f64) -> anyhow::Result<u64> { Ok(0) }
    }

    #[tokio::test]
    async fn create_validate_revoke_roundtrip() {
        let store = Arc::new(MemStore(Default::default()));
        let mgr = SessionManager::new(store);
        let s = mgr.create_for_user("u1").await.unwrap();
        let v = mgr.validate(&s.id).await.unwrap();
        assert_eq!(v.user_id, "u1");
        mgr.assert_csrf(&v, &s.csrf_token).unwrap();
        assert!(mgr.revoke(&s.id).await.unwrap());
        assert!(matches!(mgr.validate(&s.id).await, Err(Error::SessionNotFound)));
    }
}
```

- [ ] **Step 4: Commit**

```bash
git add -A
git commit -m "feat(auth/session): session manager with CSRF + rotation"
```

---

### Task 4.3: Password module (Argon2id)

**Files:** `crates/assay-auth/src/password.rs` + tests.

Plan 11 reference: Argon2id with sensible defaults.

- [ ] **Step 1: Write unit tests first**

```rust
// crates/assay-auth/src/password.rs
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_verify_roundtrip() {
        let h = Password::default();
        let hash = h.hash("hunter2").unwrap();
        assert!(h.verify("hunter2", &hash).unwrap());
        assert!(!h.verify("wrong", &hash).unwrap());
    }

    #[test]
    fn rejects_empty_passwords() {
        let h = Password::default();
        assert!(h.hash("").is_err());
    }

    #[test]
    fn hashes_differ_between_same_password() {
        let h = Password::default();
        let h1 = h.hash("same").unwrap();
        let h2 = h.hash("same").unwrap();
        assert_ne!(h1, h2); // salt differs
    }
}
```

- [ ] **Step 2: Implementation**

```rust
use argon2::{Argon2, Algorithm, Version, Params};
use password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use rand::rngs::OsRng;
use crate::error::{Error, Result};

pub struct Password { hasher: Argon2<'static> }

impl Default for Password {
    fn default() -> Self {
        // OWASP-recommended Argon2id defaults as of 2024.
        let params = Params::new(19_456, 2, 1, None).unwrap();
        Self { hasher: Argon2::new(Algorithm::Argon2id, Version::V0x13, params) }
    }
}

impl Password {
    pub fn hash(&self, plain: &str) -> Result<String> {
        if plain.is_empty() {
            return Err(Error::InvalidCredentials);
        }
        let salt = SaltString::generate(&mut OsRng);
        let hash = self.hasher
            .hash_password(plain.as_bytes(), &salt)
            .map_err(|e| Error::Backend(anyhow::anyhow!("argon2: {e}")))?;
        Ok(hash.to_string())
    }

    pub fn verify(&self, plain: &str, hash: &str) -> Result<bool> {
        let parsed = PasswordHash::new(hash)
            .map_err(|e| Error::Backend(anyhow::anyhow!("argon2: {e}")))?;
        Ok(self.hasher.verify_password(plain.as_bytes(), &parsed).is_ok())
    }
}
```

- [ ] **Step 3: Run tests, commit**

```bash
cargo test -p assay-auth password
git add -A && git commit -m "feat(auth/password): Argon2id hash + verify"
```

---

### Task 4.4: JWT module (issue + verify + JWKS rotation)

**Files:** `crates/assay-auth/src/jwt.rs` + tests.

Plan 11 reference: `jsonwebtoken` 10, JWKS fetch + rotation. History preserved so old tokens verify.

- [ ] **Step 1: Design — key registry + rotation**

```rust
// crates/assay-auth/src/jwt.rs
//!
//! JWT issuance + verification with key rotation.
//!
//! The `JwtConfig` holds:
//!  - Active signing key (used for new tokens)
//!  - Previous keys (used to verify older tokens still in circulation)
//!  - JWKS endpoint URL (for upstream providers — OIDC client side)
//!
//! Rotate with `config.rotate(new_key)` — old key moves to history.
//! History is capped at N entries; age-out expires after token_max_lifetime.

use std::sync::Arc;
use jsonwebtoken::{DecodingKey, EncodingKey, Algorithm, Header, Validation, TokenData};
use serde::{Serialize, de::DeserializeOwned};

pub struct JwtKeyPair {
    pub kid: String,
    pub encoding: EncodingKey,
    pub decoding: DecodingKey,
    pub created_at: f64,
}

#[derive(Clone)]
pub struct JwtConfig(Arc<parking_lot::RwLock<JwtConfigInner>>);

struct JwtConfigInner {
    active: JwtKeyPair,
    history: Vec<JwtKeyPair>,  // previous keys, still valid for verify
    issuer: String,
    audience: Vec<String>,
    algorithm: Algorithm,
    max_history: usize,
}

impl JwtConfig {
    pub fn new_rs256(active: JwtKeyPair, issuer: String, audience: Vec<String>) -> Self {
        Self(Arc::new(parking_lot::RwLock::new(JwtConfigInner {
            active, history: vec![], issuer, audience,
            algorithm: Algorithm::RS256, max_history: 3,
        })))
    }

    pub fn issue<C: Serialize>(&self, claims: &C) -> crate::Result<String> {
        let inner = self.0.read();
        let mut header = Header::new(inner.algorithm);
        header.kid = Some(inner.active.kid.clone());
        jsonwebtoken::encode(&header, claims, &inner.active.encoding)
            .map_err(|e| crate::Error::Jwt(e.to_string()))
    }

    pub fn verify<C: DeserializeOwned>(&self, token: &str) -> crate::Result<TokenData<C>> {
        let header = jsonwebtoken::decode_header(token)
            .map_err(|e| crate::Error::Jwt(e.to_string()))?;
        let inner = self.0.read();
        let mut v = Validation::new(inner.algorithm);
        v.set_issuer(&[inner.issuer.clone()]);
        v.set_audience(&inner.audience);
        let kid = header.kid.as_deref().unwrap_or(&inner.active.kid);
        let key = if kid == inner.active.kid {
            &inner.active.decoding
        } else {
            inner.history.iter()
                .find(|k| k.kid == kid)
                .map(|k| &k.decoding)
                .ok_or_else(|| crate::Error::Jwt(format!("unknown kid {kid}")))?
        };
        jsonwebtoken::decode::<C>(token, key, &v)
            .map_err(|e| crate::Error::Jwt(e.to_string()))
    }

    pub fn rotate(&self, new_active: JwtKeyPair) {
        let mut inner = self.0.write();
        let old = std::mem::replace(&mut inner.active, new_active);
        inner.history.insert(0, old);
        let cap = inner.max_history;
        inner.history.truncate(cap);
    }

    pub fn jwks_json(&self) -> serde_json::Value {
        // Render active + history keys as a JWKS.
        // Implementation: for each key, emit {kid, kty, alg, use, n, e}.
        // Task detail: match plan 11's plan for "/jwks.json" endpoint.
        todo!("task 4.4 step 3")
    }
}
```

- [ ] **Step 2: JWKS JSON rendering**

Implement `jwks_json` — serialise each RSA key as a JWK entry.

- [ ] **Step 3: Upstream JWKS fetcher (client side)**

```rust
pub struct UpstreamJwks {
    url: String,
    cached: parking_lot::RwLock<Option<(std::time::Instant, Vec<JwtKeyPair>)>>,
    ttl: std::time::Duration,
}

impl UpstreamJwks {
    pub fn new(url: String) -> Self { Self { url, cached: Default::default(), ttl: std::time::Duration::from_secs(3600) } }

    pub async fn fetch(&self) -> crate::Result<Vec<JwtKeyPair>> {
        // HTTP GET url, parse JWKS, populate cache. Return cached on
        // refresh failure if cache is non-empty.
        todo!("task 4.4 step 4")
    }
}
```

- [ ] **Step 4: Unit tests**

- Round-trip issue → verify same payload.
- Rotate: old token still verifies after rotation (from history).
- Expiry: `exp` claim enforced.
- Unknown kid: rejected.
- Wrong audience: rejected.

- [ ] **Step 5: Commit**

```bash
git commit -m "feat(auth/jwt): issue + verify with kid rotation + JWKS rendering"
```

---

### Task 4.5: Biscuit module (capability tokens)

**Files:** `crates/assay-auth/src/biscuit.rs` + tests.

Plan 11 reference: Biscuit 6 — public-key signed, Datalog policy, offline verifiable, attenuable.

- [ ] **Step 1: API sketch**

```rust
use biscuit_auth::{Biscuit, KeyPair, PublicKey, builder::*};

pub struct BiscuitIssuer {
    keypair: KeyPair,
}

impl BiscuitIssuer {
    pub fn generate() -> Self { Self { keypair: KeyPair::new() } }

    pub fn from_pem(_pem: &str) -> crate::Result<Self> { todo!() }

    pub fn public_key(&self) -> PublicKey { self.keypair.public() }

    pub fn issue(&self, facts: impl IntoIterator<Item = String>) -> crate::Result<Biscuit> {
        let mut builder = Biscuit::builder();
        for f in facts { builder.add_fact(f.as_str()).map_err(|e| crate::Error::Backend(e.into()))?; }
        builder.build(&self.keypair).map_err(|e| crate::Error::Backend(e.into()))
    }

    pub fn verify(&self, token: &[u8], checks: Vec<String>) -> crate::Result<()> {
        let biscuit = Biscuit::from(token, self.keypair.public())
            .map_err(|e| crate::Error::Backend(e.into()))?;
        let mut authorizer = biscuit.authorizer().map_err(|e| crate::Error::Backend(e.into()))?;
        for c in checks { authorizer.add_check(c.as_str()).map_err(|e| crate::Error::Backend(e.into()))?; }
        authorizer.authorize().map_err(|e| crate::Error::Backend(anyhow::anyhow!("biscuit deny: {e}")))?;
        Ok(())
    }
}

pub fn attenuate(token: &[u8], pub_key: PublicKey, caveats: Vec<String>) -> crate::Result<Vec<u8>> {
    // Append a block with the caveats and re-serialise.
    todo!()
}
```

- [ ] **Step 2: Tests**

- Issue → verify round-trip with facts `{role("admin"), user("alice")}` + check `user("alice")` →
  OK.
- Attenuation: issue + attenuate with `check if time($now), $now < 2026-06-01` → verify at future
  time → deny.
- Tamper detection: flip a byte → verify fails.

- [ ] **Step 3: Commit**

```bash
git commit -m "feat(auth/biscuit): issue / verify / attenuate"
```

---

### Task 4.6: Store backend impls for User + Session (PG, SQLite)

**Files:** `crates/assay-auth/src/store/postgres.rs`, `crates/assay-auth/src/store/sqlite.rs`,
`crates/assay-auth/migrations/{postgres,sqlite}/01_auth.sql`.

- [ ] **Step 1: Schema migration**

```sql
-- migrations/postgres/01_auth.sql
-- All auth tables live in the `auth` schema (created at boot via the
-- engine.modules-driven attach/create flow from v0.1.2). SQLite mirrors
-- this layout in the attached `auth` database (data/auth.db).

CREATE TABLE auth.users (
    id TEXT PRIMARY KEY,
    email TEXT UNIQUE,
    email_verified BOOLEAN NOT NULL DEFAULT FALSE,
    display_name TEXT,
    password_hash TEXT,
    created_at DOUBLE PRECISION NOT NULL
);

CREATE TABLE auth.user_upstream (
    provider TEXT NOT NULL,
    subject TEXT NOT NULL,
    user_id TEXT NOT NULL REFERENCES auth.users(id) ON DELETE CASCADE,
    PRIMARY KEY (provider, subject)
);
CREATE INDEX user_upstream_user ON auth.user_upstream (user_id);

CREATE TABLE auth.passkeys (
    credential_id BYTEA PRIMARY KEY,
    user_id TEXT NOT NULL REFERENCES auth.users(id) ON DELETE CASCADE,
    public_key BYTEA NOT NULL,
    sign_count INTEGER NOT NULL DEFAULT 0,
    transports TEXT NOT NULL,  -- csv
    created_at DOUBLE PRECISION NOT NULL
);
CREATE INDEX passkeys_user ON auth.passkeys (user_id);

CREATE TABLE auth.sessions (
    id TEXT PRIMARY KEY,
    user_id TEXT NOT NULL REFERENCES auth.users(id) ON DELETE CASCADE,
    csrf_token TEXT NOT NULL,
    created_at DOUBLE PRECISION NOT NULL,
    expires_at DOUBLE PRECISION NOT NULL,
    ip_hash TEXT,
    user_agent_hash TEXT
);
CREATE INDEX sessions_user ON auth.sessions (user_id);
CREATE INDEX sessions_expires ON auth.sessions (expires_at);

-- JWKS rotation (per task 4.4). Keys persisted, rotated by the JWKS
-- workflow in jeebon (or by an internal scheduler when assay-engine
-- runs standalone).
CREATE TABLE auth.jwks_keys (
    kid TEXT PRIMARY KEY,
    alg TEXT NOT NULL,                  -- e.g. EdDSA, RS256
    public_jwk JSONB NOT NULL,
    private_pem_encrypted BYTEA,        -- nullable for verifier-only nodes
    created_at DOUBLE PRECISION NOT NULL,
    rotated_at DOUBLE PRECISION,
    expires_at DOUBLE PRECISION
);
CREATE INDEX jwks_keys_active ON auth.jwks_keys (rotated_at) WHERE rotated_at IS NULL;

-- Compliance audit log: append-only, security-restricted access,
-- long retention (1+ year per jeebon plan). Distinct from engine.audit
-- (engine-level operations) and from auth's real-time event stream.
CREATE TABLE auth.audit (
    id UUID PRIMARY KEY DEFAULT uuidv7(),
    ts TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    actor TEXT,                         -- user_id, "system", or NULL for unauthenticated
    action TEXT NOT NULL,               -- e.g. "login.success", "passkey.register", "session.revoke"
    target TEXT,                        -- subject of the action (user_id, session_id, etc.)
    ip_hash TEXT,
    user_agent_hash TEXT,
    details JSONB NOT NULL DEFAULT '{}'::jsonb
);
CREATE INDEX audit_ts ON auth.audit (ts DESC);
CREATE INDEX audit_actor ON auth.audit (actor, ts DESC);
CREATE INDEX audit_action ON auth.audit (action, ts DESC);
```

SQLite version uses `BLOB` for `BYTEA` and `REAL` for `DOUBLE PRECISION`, plus a Rust-generated
UUIDv7 string for `auth.audit.id`. Otherwise identical.

- [ ] **Step 2: Implement `PostgresUserStore` + `PostgresSessionStore`**

Mirror shape of `assay-workflow::PostgresStore` — holds a `PgPool`, each trait method executes an
sqlx query.

- [ ] **Step 3: Implement `SqliteUserStore` + `SqliteSessionStore`**

Same pattern; sqlx dialect differences minimal here.

- [ ] **Step 4: Parametrised integration tests**

Mirror the Phase 2 workflow harness pattern — rstest cases for `backend-postgres` +
`backend-sqlite`.

- [ ] **Step 5: Commit**

```bash
git commit -m "feat(auth/stores): PG + SQLite impls for User + Session"
```

---

### Phase 4 exit criteria

- `cargo test -p assay-auth --features 'auth-session auth-password auth-jwt auth-biscuit'` green.
- `cargo test -p assay-auth --features 'auth backend-postgres'` green (against testcontainers PG).
- `cargo test -p assay-auth --features 'auth backend-sqlite'` green.
- No compile-time or clippy warnings on `cargo clippy -p assay-auth --all-features`.

---

## Phase 5 — Identity flows

### Task 5.1: `auth.oidc` — OIDC client (discovery, PKCE, callback)

**Files:** `crates/assay-auth/src/oidc/` (discovery, client, router).

Plan 11 reference: `openidconnect` 4; discovery, PKCE, callback, token exchange, refresh, userinfo.

- [ ] **Step 1: Client wrapper**

```rust
// crates/assay-auth/src/oidc/client.rs
use openidconnect::{
    core::{CoreClient, CoreProviderMetadata, CoreResponseType},
    AuthenticationFlow, CsrfToken, Nonce, PkceCodeChallenge, Scope,
    AuthUrl, ClientId, ClientSecret, IssuerUrl, RedirectUrl, TokenResponse,
};

pub struct OidcClient {
    inner: CoreClient,
    redirect: RedirectUrl,
}

impl OidcClient {
    pub async fn discover(
        issuer: &str, client_id: &str, client_secret: &str, redirect: &str,
    ) -> crate::Result<Self> {
        let issuer_url = IssuerUrl::new(issuer.into())
            .map_err(|e| crate::Error::Oidc(e.to_string()))?;
        let metadata = CoreProviderMetadata::discover_async(
            issuer_url, openidconnect::reqwest::async_http_client,
        ).await.map_err(|e| crate::Error::Oidc(e.to_string()))?;
        let redirect = RedirectUrl::new(redirect.into())
            .map_err(|e| crate::Error::Oidc(e.to_string()))?;
        let inner = CoreClient::from_provider_metadata(
            metadata,
            ClientId::new(client_id.into()),
            Some(ClientSecret::new(client_secret.into())),
        ).set_redirect_uri(redirect.clone());
        Ok(Self { inner, redirect })
    }

    pub fn authorize(&self, scopes: &[&str]) -> (url::Url, CsrfToken, Nonce, PkceCodeChallenge) {
        let (pkce_challenge, pkce_verifier) = PkceCodeChallenge::new_random_sha256();
        let mut auth = self.inner.authorize_url(
            AuthenticationFlow::<CoreResponseType>::AuthorizationCode,
            CsrfToken::new_random, Nonce::new_random,
        );
        for s in scopes { auth = auth.add_scope(Scope::new((*s).into())); }
        let (url, csrf, nonce) = auth.set_pkce_challenge(pkce_challenge).url();
        (url, csrf, nonce, pkce_verifier)  // caller persists verifier against session
    }

    pub async fn exchange_code(
        &self, code: &str, pkce_verifier: &str,
    ) -> crate::Result<IdTokenClaims> {
        todo!("exchange + verify id_token; return claims incl. sub + email")
    }

    pub async fn userinfo(&self, access_token: &str) -> crate::Result<serde_json::Value> {
        todo!()
    }
}
```

- [ ] **Step 2: Multi-provider registry**

`AuthCtx` gains a `providers: Arc<HashMap<String, OidcClient>>` field keyed by provider slug
(`google`, `apple`, `github`, …). Admin registers providers via CLI or HTTP.

- [ ] **Step 3: Client router**

Routes:

- `GET /login/{provider}` → 302 to authorize URL; session stores CSRF, PKCE verifier, nonce.
- `GET /login/{provider}/callback` → validates CSRF, exchanges code, creates user + session.

- [ ] **Step 4: Integration test**

`wiremock` stands up a fake discovery endpoint + token endpoint; assert the flow round-trips to a
user row.

- [ ] **Step 5: Commit**

```bash
git commit -m "feat(auth/oidc): client with discovery, PKCE, callback, multi-provider"
```

---

### Task 5.2: `auth.passkey` — WebAuthn register + authenticate

**Files:** `crates/assay-auth/src/passkey.rs` + router.

Plan 11 reference: `webauthn-rs` 0.5; start/finish register, start/finish auth.

- [ ] **Step 1: Passkey manager wrapper**

```rust
pub struct PasskeyManager {
    webauthn: Arc<webauthn_rs::Webauthn>,
    users: Arc<dyn UserStore>,
}

impl PasskeyManager {
    pub fn new(rp_id: &str, rp_origin: &str, users: Arc<dyn UserStore>) -> crate::Result<Self> {
        let webauthn = webauthn_rs::WebauthnBuilder::new(rp_id, &url::Url::parse(rp_origin)?)
            .map_err(|e| crate::Error::Passkey(e.to_string()))?
            .build()
            .map_err(|e| crate::Error::Passkey(e.to_string()))?;
        Ok(Self { webauthn: Arc::new(webauthn), users })
    }

    pub fn start_register(&self, user_id: &str, user_name: &str) -> crate::Result<(PublicKeyCredentialCreationOptions, RegistrationState)> { todo!() }
    pub async fn finish_register(&self, user_id: &str, state: RegistrationState, resp: RegisterPublicKeyCredential) -> crate::Result<()> { todo!() }
    pub fn start_authenticate(&self) -> crate::Result<(RequestChallengeResponse, AuthenticationState)> { todo!() }
    pub async fn finish_authenticate(&self, state: AuthenticationState, resp: PublicKeyCredential) -> crate::Result<String /* user_id */> { todo!() }
}
```

- [ ] **Step 2: Router**

```
POST /passkey/register/start → options + state
POST /passkey/register/finish → store cred
POST /passkey/auth/start → challenge + state
POST /passkey/auth/finish → session cookie
```

Register + authenticate state is held in session scratch keyed by session ID.

- [ ] **Step 3: Test with webauthn-rs fixtures**

The library ships test vectors (`webauthn-rs-proto` test assets). Use them to build an end-to-end
test without a browser.

- [ ] **Step 4: Commit**

```bash
git commit -m "feat(auth/passkey): WebAuthn register + auth flows"
```

---

### Task 5.3: Auth module routes wiring

**Files:** `crates/assay-auth/src/router.rs`.

- [ ] **Step 1: Compose module routers**

```rust
pub fn router() -> Router<AuthCtx> {
    let mut r = Router::new();

    #[cfg(feature = "auth-oidc")]
    { r = r.merge(crate::oidc::router::router()); }

    #[cfg(feature = "auth-passkey")]
    { r = r.merge(crate::passkey::router::router()); }

    #[cfg(feature = "auth-password")]
    { r = r.merge(crate::password::router::router()); }

    #[cfg(feature = "auth-session")]
    { r = r.merge(crate::session::router::router()); }  // /logout, /whoami

    // OIDC provider routes (Phase 7) merged separately via oidc_provider feature.

    r
}
```

- [ ] **Step 2: Integration test — cold sign-up via passkey**

Spin up an in-memory auth stack (all mock stores), run: `POST /passkey/register/start` → `/finish` →
`POST /passkey/auth/start` → `/finish` → `GET /whoami` returns the user. Assert.

- [ ] **Step 3: Commit**

```bash
git commit -m "feat(auth): wire module routers into AuthCtx router"
```

---

### Task 5.4: Runtime Lua wrappers for auth.*

**Files:** `crates/assay/stdlib/auth/*.lua`, `crates/assay/src/lua/auth.rs`.

Plan 11 reference: thin Lua wrappers call engine over HTTP. `ASSAY_ENGINE_URL` env var or
`assay.toml`. HTTP/2 connection reuse, 0.5–2ms localhost.

- [ ] **Step 1: Engine URL discovery**

Runtime config reads `ASSAY_ENGINE_URL` env var, else looks for `[engine] url = "..."` in
assay.toml.

- [ ] **Step 2: `auth.login`, `auth.whoami`, `auth.zanzibar.check` wrappers**

```lua
-- stdlib/auth/api.lua
local auth = {}

function auth.login(method, credentials)
  return http.post(engine_url() .. "/login/" .. method, credentials)
end

function auth.whoami(session_token)
  return http.get(engine_url() .. "/whoami", { headers = { cookie = "assay_session=" .. session_token } })
end

auth.zanzibar = {}
function auth.zanzibar.check(object, permission, subject)
  local body = { object = object, permission = permission, subject = subject }
  return http.post(engine_url() .. "/api/v1/zanzibar/check", body)
end

return auth
```

- [ ] **Step 3: Integration test against a running engine**

Start `assay-engine` in a subprocess, run a Lua script that calls `auth.login`, asserts
`auth.whoami` returns the expected user.

- [ ] **Step 4: Commit**

```bash
git commit -m "feat(runtime): auth.* Lua wrappers calling assay-engine"
```

---

### Phase 5 exit criteria

- OIDC client flow against mock upstream: green.
- Passkey register/auth against webauthn-rs fixtures: green.
- Lua `auth.*` wrappers call a running engine, tests pass.

---

## Phase 6 — Zanzibar core

Plan 11 lines 136–260 cover Zanzibar rationale, backend selection, check pipeline, and consistency.
This phase delivers the `ZanzibarStore` trait (in `assay-domain`), PG18 + SQLite impls (recursive
CTE), `check`/`expand`/`lookup_*` algorithms, and zookies. PG18 specifics: UUIDv7 PKs via
`uuidv7()`, skip-scan composite index on
`(object_type, object_id, relation, subject_type,
subject_id)` serving both forward and inverse
queries.

### Task 6.1: `ZanzibarStore` trait + schema parser

**Files:** `crates/assay-auth/src/zanzibar/{mod.rs, trait.rs, schema.rs, types.rs}`.

- [ ] **Step 1: Types + trait**

```rust
// crates/assay-auth/src/zanzibar/types.rs
#[derive(Clone, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct Object { pub r#type: String, pub id: String }

#[derive(Clone, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct Subject {
    pub r#type: String,
    pub id: String,
    pub relation: Option<String>,  // None = direct user; Some = userset
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct Tuple {
    pub object: Object,
    pub relation: String,
    pub subject: Subject,
    pub created_at: f64,
}

#[derive(Clone, Copy, Debug)]
pub enum Consistency { Minimum, AtExactSnapshot(u64) }

#[derive(Clone, Debug)]
pub enum CheckResult { Permitted, Denied }
```

```rust
// crates/assay-auth/src/zanzibar/trait.rs
#[async_trait::async_trait]
pub trait ZanzibarStore: Send + Sync + 'static {
    async fn write_tuple(&self, t: &Tuple) -> anyhow::Result<()>;
    async fn delete_tuple(&self, t: &Tuple) -> anyhow::Result<bool>;
    async fn check(&self, object: &Object, permission: &str, subject: &Subject, consistency: Consistency) -> anyhow::Result<CheckResult>;
    async fn expand(&self, object: &Object, permission: &str) -> anyhow::Result<UsersetTree>;
    async fn lookup_resources(&self, subject: &Subject, permission: &str, object_type: &str) -> anyhow::Result<Vec<Object>>;
    async fn lookup_subjects(&self, object: &Object, permission: &str, subject_type: &str) -> anyhow::Result<Vec<Subject>>;
    async fn define_namespace(&self, schema: &NamespaceSchema) -> anyhow::Result<()>;
    async fn get_namespace(&self, name: &str) -> anyhow::Result<Option<NamespaceSchema>>;
}
```

- [ ] **Step 2: Schema parser (SpiceDB-compatible subset)**

```rust
// crates/assay-auth/src/zanzibar/schema.rs
// parses:
//   definition user {}
//   definition group { relation member: user }
//   definition document {
//     relation owner: user
//     relation viewer: user | group#member
//     permission view = owner + viewer
//     permission edit = owner
//   }
//
// Output: NamespaceSchema with relations + permissions as algebraic
// expressions. Permission expressions compose via union (+), intersection
// (&), and exclusion (-).
```

- [ ] **Step 3: Unit tests for the parser**

Round-trip: parse → pretty-print → parse again → structures equal.

- [ ] **Step 4: Commit**

```bash
git commit -m "feat(auth/zanzibar): trait + types + SpiceDB-compat schema parser"
```

---

### Task 6.2: PG backend — tuple table + recursive CTE

**Files:** `crates/assay-auth/src/zanzibar/postgres.rs`, migrations.

Plan 11 lines 181–217: schema + recursive CTE walk, depth limit 50, cycle detection.

- [ ] **Step 1: Migration**

```sql
-- migrations/postgres/02_zanzibar.sql
CREATE TABLE auth.zanzibar_tuples (
    object_type  TEXT NOT NULL,
    object_id    TEXT NOT NULL,
    relation     TEXT NOT NULL,
    subject_type TEXT NOT NULL,
    subject_id   TEXT NOT NULL,
    subject_rel  TEXT,
    created_at   DOUBLE PRECISION NOT NULL DEFAULT EXTRACT(EPOCH FROM NOW()),
    PRIMARY KEY (object_type, object_id, relation, subject_type, subject_id, subject_rel)
);
CREATE INDEX zanzibar_tuples_rev ON auth.zanzibar_tuples
    (subject_type, subject_id, relation);

CREATE TABLE auth.zanzibar_namespaces (
    name TEXT PRIMARY KEY,
    schema_json JSONB NOT NULL,
    updated_at DOUBLE PRECISION NOT NULL
);
```

- [ ] **Step 2: `check` via recursive CTE**

```rust
impl ZanzibarStore for PostgresZanzibarStore {
    async fn check(&self, object: &Object, permission: &str, subject: &Subject, _c: Consistency)
        -> anyhow::Result<CheckResult>
    {
        // Resolve the permission expression via the namespace schema
        // (task 6.1 parser). For a simple case `view = owner + viewer`,
        // run the CTE for `owner`, then if not permitted run for `viewer`.
        //
        // The CTE walks the tuple DAG from the object side.
        let granted: bool = sqlx::query_scalar!(
            r#"
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
            ) AS granted
            "#,
            object.r#type, object.id, permission,
            subject.r#type, subject.id,
        )
        .fetch_one(&self.pool).await?.unwrap_or(false);

        Ok(if granted { CheckResult::Permitted } else { CheckResult::Denied })
    }
}
```

- [ ] **Step 3: `write_tuple` + `delete_tuple`**

Straightforward INSERT/DELETE against the table.

- [ ] **Step 4: Tests — the plan 11 example**

```
zanzibar.define_namespace(ns_schema).await;
zanzibar.write_tuple(Tuple::direct("document:x", "owner", "user:alice")).await;
zanzibar.write_tuple(Tuple::direct("group:g1",   "member", "user:bob")).await;
zanzibar.write_tuple(Tuple::userset("document:x", "viewer", "group:g1#member")).await;

assert_eq!(check("document:x", "view", "user:alice"), Permitted);
assert_eq!(check("document:x", "view", "user:bob"),   Permitted);
assert_eq!(check("document:x", "view", "user:carol"), Denied);
assert_eq!(check("document:x", "edit", "user:bob"),   Denied);
```

- [ ] **Step 5: Commit**

```bash
git commit -m "feat(auth/zanzibar): PG backend with recursive-CTE check"
```

---

### Task 6.3: SQLite backend — same CTE

- [ ] **Step 1: Migration** mirrors PG, `JSONB` becomes `TEXT`, `DOUBLE PRECISION` becomes `REAL`.
- [ ] **Step 2: `check` uses identical CTE** (SQLite's recursive CTE syntax is compatible).
- [ ] **Step 3: Tests** — extend harness with SQLite case.
- [ ] **Step 4: Commit** — `feat(auth/zanzibar): SQLite backend`.

---

### Task 6.4: `expand`, `lookup_resources`, `lookup_subjects`

- [ ] **Step 1: `expand`** returns the userset tree for a permission — essentially the CTE result
      without the final membership check, structured as a tree.
- [ ] **Step 2: `lookup_resources`** — walk from the subject side (uses the reverse index
      `zanzibar_tuple_rev`).
- [ ] **Step 3: `lookup_subjects`** — walk from the object side, collect all terminal subjects.
- [ ] **Step 4: Tests** on the plan 11 example — `expand(document:x, view)` returns a tree with
      `owner -> alice` and `viewer -> group:g1#member -> bob`.
- [ ] **Step 5: Commit** — `feat(auth/zanzibar): expand + lookup_resources + lookup_subjects`.

---

### Task 6.5: Consistency tokens (zookies)

- [ ] **Step 1: Design** — zookies encode the commit time of the last observed write. PG: use
      `pg_current_wal_lsn()` at write time. SQLite: use a monotonic counter stored in a metadata
      table.
- [ ] **Step 2: Implementation**
  - `write_tuple` returns a zookie.
  - `check(consistency: Consistency::AtExactSnapshot(zookie))` blocks (or errors) if the store's
    current position is before the zookie.
- [ ] **Step 3: Tests** — write, extract zookie, check at zookie vs. check at zero; behaviour
      matches spec.
- [ ] **Step 4: Commit** — `feat(auth/zanzibar): zookies / consistency tokens`.

---

### Task 6.6: Cycle detection + depth limit

- [ ] **Step 1: Depth** — the CTE already limits to 50. Surface the limit as
      `crate::Error::ZanzibarDepth` when exceeded.
- [ ] **Step 2: Cycle** — within `check`, track a visited-set `HashSet<Object+Relation>` per call;
      if a node repeats, return `Err(Error::ZanzibarCycle)`.
- [ ] **Step 3: Tests** — build a self-referential tuple (`group:g member group:g#member`) and
      assert `Error::ZanzibarCycle`.
- [ ] **Step 4: Commit** — `feat(auth/zanzibar): cycle detection + depth limit surfacing`.

---

### Task 6.7 — REMOVED per plan 12 rev 2

Was "SurrealDB Zanzibar backend (native RELATE)". Dropped — SurrealDB removed from v0.13.0 entirely.
See plan 12 Revision log for rationale. `ZanzibarStore` trait abstraction stays, so a third backend
can be added by a future plan without touching Phase 6 architecture.

---

### Phase 6 exit criteria

- `check`, `expand`, `lookup_resources`, `lookup_subjects` all correct against the plan 11 reference
  example on both PG18 and SQLite.
- Zookies functional; depth limit + cycle detection reject pathological inputs.
- Integration tests parametrised over both backends green.
- A script: `zanzibar.define_namespace(...)`, write ~1000 tuples, run 100 checks → both backends
  report identical results.
- PG18-specific sanity: `EXPLAIN (ANALYZE)` confirms the composite skip-scan index is chosen for
  both forward (`check`) and reverse (`lookup_*`) queries.

---

## What's next

**[12d](./12d-phase-7-oidc-provider.md)** — Phase 7, the full OIDC provider (IdP endpoints, consent,
SSO, upstream federation). This is the largest single phase at ~25h; most of Phase 5's `auth.oidc`
client code gets reused in "federated upstream" mode.
