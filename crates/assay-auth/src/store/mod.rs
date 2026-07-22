//! Storage traits + concrete backends for the auth crate.
//!
//! The traits ([`UserStore`], [`SessionStore`]) are object-safe so
//! [`crate::ctx::AuthCtx`] can hold `Arc<dyn UserStore>`. Backend
//! implementations live behind their respective Cargo features —
//! `backend-postgres` and `backend-sqlite` — so a slim downstream
//! build (`--no-default-features --features auth-jwt`) compiles
//! without sqlx.
//!
//! All concrete impls assume the auth schema/attached database has
//! already been migrated by [`crate::schema`]. Engine boot runs the
//! migration before constructing the stores.

pub mod types;

#[cfg(feature = "backend-postgres")]
pub mod postgres;
#[cfg(feature = "backend-sqlite")]
pub mod sqlite;

pub use types::*;

#[cfg(feature = "backend-postgres")]
pub use postgres::{PostgresSessionStore, PostgresUserStore};
#[cfg(feature = "backend-sqlite")]
pub use sqlite::{SqliteSessionStore, SqliteUserStore};

/// CRUD over `auth.users`, `auth.user_upstream`, `auth.passkeys`.
///
/// Methods that touch passwords go through this trait too — the
/// password module hashes the plaintext, then asks the store to
/// persist the resulting hash.
#[async_trait::async_trait]
pub trait UserStore: Send + Sync + 'static {
    async fn create_user(&self, user: &User) -> anyhow::Result<()>;
    async fn get_user_by_id(&self, id: &str) -> anyhow::Result<Option<User>>;
    async fn get_user_by_email(&self, email: &str) -> anyhow::Result<Option<User>>;
    async fn update_user(&self, user: &User) -> anyhow::Result<()>;

    /// Admin: paginated user list. `limit` is clamped by the impl;
    /// `offset` may be 0. `search` is an optional case-insensitive
    /// substring match on `email` (or `display_name` when email is
    /// NULL). Returns rows sorted by `created_at DESC`.
    async fn list_users(
        &self,
        limit: i64,
        offset: i64,
        search: Option<&str>,
    ) -> anyhow::Result<Vec<User>>;

    /// Admin: total user count (after applying `search` if provided).
    /// Used by the dashboard's pagination + the Lua wrapper.
    async fn count_users(&self, search: Option<&str>) -> anyhow::Result<i64>;

    /// Admin: hard-delete a user row + cascade dependents. Returns
    /// `Ok(true)` iff a row was removed. The schema's
    /// `ON DELETE CASCADE` foreign keys handle the dependents
    /// (`auth.passkeys`, `auth.sessions`, `auth.user_upstream`).
    async fn delete_user(&self, id: &str) -> anyhow::Result<bool>;

    // Password credentials — stored as Argon2id PHC strings on `auth.users`.
    async fn set_password_hash(&self, user_id: &str, hash: &str) -> anyhow::Result<()>;
    async fn get_password_hash(&self, user_id: &str) -> anyhow::Result<Option<String>>;

    // Passkey credentials — `auth.passkeys`.
    async fn list_passkeys(&self, user_id: &str) -> anyhow::Result<Vec<PasskeyCred>>;
    async fn add_passkey(&self, user_id: &str, cred: &PasskeyCred) -> anyhow::Result<()>;
    async fn remove_passkey(&self, credential_id: &[u8]) -> anyhow::Result<bool>;

    /// Fetch a single stored credential by its raw credential id, plus
    /// the `user_id` that owns it. Returns `Ok(None)` when no row
    /// matches. The authentication ceremony uses this to resolve the
    /// owning user from the credential the authenticator asserted —
    /// server-side, never trusting a client-supplied identity.
    async fn get_passkey(
        &self,
        credential_id: &[u8],
    ) -> anyhow::Result<Option<(String, PasskeyCred)>>;

    /// Persist the post-authentication sign-count bump (and the refreshed
    /// serialised blob, which carries the same counter + any backup-state
    /// changes) for the credential. Keyed on `credential_id`. Returns
    /// `Ok(true)` iff a row was updated.
    async fn update_passkey_counter(
        &self,
        credential_id: &[u8],
        sign_count: u32,
        passkey_json: &str,
    ) -> anyhow::Result<bool>;

    // Federated upstream links — `auth.user_upstream`.
    async fn link_upstream(
        &self,
        user_id: &str,
        provider: &str,
        subject: &str,
    ) -> anyhow::Result<()>;
    async fn get_user_by_upstream(
        &self,
        provider: &str,
        subject: &str,
    ) -> anyhow::Result<Option<User>>;

    /// Admin: list every (provider, subject) link for a user. Used by
    /// the dashboard's user-detail pane to show federated identities.
    async fn list_upstream_for_user(&self, user_id: &str) -> anyhow::Result<Vec<(String, String)>>;
}

/// CRUD over `auth.sessions`. The session manager
/// ([`crate::session::SessionManager`]) is the primary caller.
#[async_trait::async_trait]
pub trait SessionStore: Send + Sync + 'static {
    async fn create(&self, session: &Session) -> anyhow::Result<()>;
    async fn get(&self, id: &str) -> anyhow::Result<Option<Session>>;
    async fn delete(&self, id: &str) -> anyhow::Result<bool>;
    async fn list_for_user(&self, user_id: &str) -> anyhow::Result<Vec<Session>>;
    async fn delete_for_user(&self, user_id: &str) -> anyhow::Result<u64>;
    /// Drop every session whose `expires_at <= now`. Returns the row
    /// count for visibility (logging / metrics).
    async fn purge_expired(&self, now: f64) -> anyhow::Result<u64>;

    /// Admin: paginated global session list. `user_filter` narrows to
    /// a single user when provided. Returns rows sorted by
    /// `created_at DESC`. Used by the dashboard's Sessions pane and
    /// the Lua wrapper.
    async fn list_all(
        &self,
        limit: i64,
        offset: i64,
        user_filter: Option<&str>,
    ) -> anyhow::Result<Vec<Session>>;

    /// Admin: total session count (optionally filtered by user).
    async fn count_all(&self, user_filter: Option<&str>) -> anyhow::Result<i64>;
}
