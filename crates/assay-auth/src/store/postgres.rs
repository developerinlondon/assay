//! Postgres implementations of [`UserStore`] and [`SessionStore`].
//!
//! Tables live in the `auth` schema (created/migrated by
//! [`crate::schema::migrate_postgres`]). Queries are schema-qualified
//! (`auth.users`, …) so the connection's `search_path` is irrelevant —
//! matches the rest of the codebase's "explicit > implicit" stance.

use std::sync::Arc;

use anyhow::{Context, Result};
use sqlx::{PgPool, Row};

use super::types::{PasskeyCred, Session, User};
use super::{SessionStore, UserStore};

/// User store backed by a shared `PgPool`. Cheap to clone (the
/// underlying pool is `Arc`d already).
#[derive(Clone)]
pub struct PostgresUserStore {
    pool: PgPool,
}

impl PostgresUserStore {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Wrap into an `Arc<dyn UserStore>` for [`crate::ctx::AuthCtx`].
    pub fn into_dyn(self) -> Arc<dyn UserStore> {
        Arc::new(self)
    }
}

#[async_trait::async_trait]
impl UserStore for PostgresUserStore {
    async fn create_user(&self, user: &User) -> Result<()> {
        sqlx::query(
            "INSERT INTO auth.users
                 (id, email, email_verified, display_name, password_hash, created_at)
             VALUES ($1, $2, $3, $4, NULL, $5)",
        )
        .bind(&user.id)
        .bind(&user.email)
        .bind(user.email_verified)
        .bind(&user.display_name)
        .bind(user.created_at)
        .execute(&self.pool)
        .await
        .context("auth.users insert")?;
        Ok(())
    }

    async fn get_user_by_id(&self, id: &str) -> Result<Option<User>> {
        let row = sqlx::query(
            "SELECT id, email, email_verified, display_name, created_at
             FROM auth.users WHERE id = $1",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .context("auth.users select by id")?;
        Ok(row.map(map_user_row_pg))
    }

    async fn get_user_by_email(&self, email: &str) -> Result<Option<User>> {
        let row = sqlx::query(
            "SELECT id, email, email_verified, display_name, created_at
             FROM auth.users WHERE email = $1",
        )
        .bind(email)
        .fetch_optional(&self.pool)
        .await
        .context("auth.users select by email")?;
        Ok(row.map(map_user_row_pg))
    }

    async fn update_user(&self, user: &User) -> Result<()> {
        sqlx::query(
            "UPDATE auth.users
             SET email = $2,
                 email_verified = $3,
                 display_name = $4
             WHERE id = $1",
        )
        .bind(&user.id)
        .bind(&user.email)
        .bind(user.email_verified)
        .bind(&user.display_name)
        .execute(&self.pool)
        .await
        .context("auth.users update")?;
        Ok(())
    }

    async fn set_password_hash(&self, user_id: &str, hash: &str) -> Result<()> {
        sqlx::query("UPDATE auth.users SET password_hash = $2 WHERE id = $1")
            .bind(user_id)
            .bind(hash)
            .execute(&self.pool)
            .await
            .context("auth.users set password_hash")?;
        Ok(())
    }

    async fn get_password_hash(&self, user_id: &str) -> Result<Option<String>> {
        let row: Option<(Option<String>,)> =
            sqlx::query_as("SELECT password_hash FROM auth.users WHERE id = $1")
                .bind(user_id)
                .fetch_optional(&self.pool)
                .await
                .context("auth.users select password_hash")?;
        Ok(row.and_then(|r| r.0))
    }

    async fn list_passkeys(&self, user_id: &str) -> Result<Vec<PasskeyCred>> {
        let rows = sqlx::query(
            "SELECT credential_id, public_key, sign_count, transports, created_at
             FROM auth.passkeys WHERE user_id = $1
             ORDER BY created_at",
        )
        .bind(user_id)
        .fetch_all(&self.pool)
        .await
        .context("auth.passkeys list")?;
        Ok(rows.into_iter().map(map_passkey_row_pg).collect())
    }

    async fn add_passkey(&self, user_id: &str, cred: &PasskeyCred) -> Result<()> {
        sqlx::query(
            "INSERT INTO auth.passkeys
                 (credential_id, user_id, public_key, sign_count, transports, created_at)
             VALUES ($1, $2, $3, $4, $5, $6)",
        )
        .bind(&cred.credential_id)
        .bind(user_id)
        .bind(&cred.public_key)
        .bind(cred.sign_count as i32)
        .bind(cred.transports.join(","))
        .bind(cred.created_at)
        .execute(&self.pool)
        .await
        .context("auth.passkeys insert")?;
        Ok(())
    }

    async fn remove_passkey(&self, credential_id: &[u8]) -> Result<bool> {
        let res = sqlx::query("DELETE FROM auth.passkeys WHERE credential_id = $1")
            .bind(credential_id)
            .execute(&self.pool)
            .await
            .context("auth.passkeys delete")?;
        Ok(res.rows_affected() > 0)
    }

    async fn link_upstream(&self, user_id: &str, provider: &str, subject: &str) -> Result<()> {
        sqlx::query(
            "INSERT INTO auth.user_upstream (provider, subject, user_id)
             VALUES ($1, $2, $3)
             ON CONFLICT (provider, subject) DO UPDATE SET user_id = EXCLUDED.user_id",
        )
        .bind(provider)
        .bind(subject)
        .bind(user_id)
        .execute(&self.pool)
        .await
        .context("auth.user_upstream upsert")?;
        Ok(())
    }

    async fn get_user_by_upstream(
        &self,
        provider: &str,
        subject: &str,
    ) -> Result<Option<User>> {
        let row = sqlx::query(
            "SELECT u.id, u.email, u.email_verified, u.display_name, u.created_at
             FROM auth.users u
             JOIN auth.user_upstream l ON l.user_id = u.id
             WHERE l.provider = $1 AND l.subject = $2",
        )
        .bind(provider)
        .bind(subject)
        .fetch_optional(&self.pool)
        .await
        .context("auth.user_upstream lookup")?;
        Ok(row.map(map_user_row_pg))
    }
}

/// Session store backed by `auth.sessions`. Independent struct from
/// [`PostgresUserStore`] because they're independently mockable in
/// tests and the engine may swap one without the other.
#[derive(Clone)]
pub struct PostgresSessionStore {
    pool: PgPool,
}

impl PostgresSessionStore {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    pub fn into_dyn(self) -> Arc<dyn SessionStore> {
        Arc::new(self)
    }
}

#[async_trait::async_trait]
impl SessionStore for PostgresSessionStore {
    async fn create(&self, session: &Session) -> Result<()> {
        sqlx::query(
            "INSERT INTO auth.sessions
                 (id, user_id, csrf_token, created_at, expires_at, ip_hash, user_agent_hash)
             VALUES ($1, $2, $3, $4, $5, $6, $7)",
        )
        .bind(&session.id)
        .bind(&session.user_id)
        .bind(&session.csrf_token)
        .bind(session.created_at)
        .bind(session.expires_at)
        .bind(&session.ip_hash)
        .bind(&session.user_agent_hash)
        .execute(&self.pool)
        .await
        .context("auth.sessions insert")?;
        Ok(())
    }

    async fn get(&self, id: &str) -> Result<Option<Session>> {
        let row = sqlx::query(
            "SELECT id, user_id, csrf_token, created_at, expires_at, ip_hash, user_agent_hash
             FROM auth.sessions WHERE id = $1",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .context("auth.sessions select")?;
        Ok(row.map(map_session_row_pg))
    }

    async fn delete(&self, id: &str) -> Result<bool> {
        let res = sqlx::query("DELETE FROM auth.sessions WHERE id = $1")
            .bind(id)
            .execute(&self.pool)
            .await
            .context("auth.sessions delete")?;
        Ok(res.rows_affected() > 0)
    }

    async fn list_for_user(&self, user_id: &str) -> Result<Vec<Session>> {
        let rows = sqlx::query(
            "SELECT id, user_id, csrf_token, created_at, expires_at, ip_hash, user_agent_hash
             FROM auth.sessions WHERE user_id = $1 ORDER BY created_at DESC",
        )
        .bind(user_id)
        .fetch_all(&self.pool)
        .await
        .context("auth.sessions list_for_user")?;
        Ok(rows.into_iter().map(map_session_row_pg).collect())
    }

    async fn delete_for_user(&self, user_id: &str) -> Result<u64> {
        let res = sqlx::query("DELETE FROM auth.sessions WHERE user_id = $1")
            .bind(user_id)
            .execute(&self.pool)
            .await
            .context("auth.sessions delete_for_user")?;
        Ok(res.rows_affected())
    }

    async fn purge_expired(&self, now: f64) -> Result<u64> {
        let res = sqlx::query("DELETE FROM auth.sessions WHERE expires_at <= $1")
            .bind(now)
            .execute(&self.pool)
            .await
            .context("auth.sessions purge_expired")?;
        Ok(res.rows_affected())
    }
}

fn map_user_row_pg(row: sqlx::postgres::PgRow) -> User {
    User {
        id: row.get("id"),
        email: row.get("email"),
        email_verified: row.get("email_verified"),
        display_name: row.get("display_name"),
        created_at: row.get("created_at"),
    }
}

fn map_session_row_pg(row: sqlx::postgres::PgRow) -> Session {
    Session {
        id: row.get("id"),
        user_id: row.get("user_id"),
        csrf_token: row.get("csrf_token"),
        created_at: row.get("created_at"),
        expires_at: row.get("expires_at"),
        ip_hash: row.get("ip_hash"),
        user_agent_hash: row.get("user_agent_hash"),
    }
}

fn map_passkey_row_pg(row: sqlx::postgres::PgRow) -> PasskeyCred {
    let transports: String = row.get("transports");
    let sign_count: i32 = row.get("sign_count");
    PasskeyCred {
        credential_id: row.get("credential_id"),
        public_key: row.get("public_key"),
        sign_count: sign_count.max(0) as u32,
        transports: if transports.is_empty() {
            Vec::new()
        } else {
            transports.split(',').map(|s| s.to_string()).collect()
        },
        created_at: row.get("created_at"),
    }
}
