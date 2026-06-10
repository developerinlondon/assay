//! SQLite implementations of [`UserStore`] and [`SessionStore`].
//!
//! Tables live in the attached `auth` database (engine boot ATTACHes
//! `data/auth.db` AS `auth` before [`crate::schema::migrate_sqlite`]
//! creates them). Queries are schema-qualified (`auth.users`, …) so the
//! syntax matches the PG store exactly — the `auth.` prefix resolves
//! against the ATTACH alias on SQLite and against the schema on PG.

use std::sync::Arc;

use anyhow::{Context, Result};
use sqlx::{Row, SqlitePool};

use super::types::{PasskeyCred, Session, User};
use super::{SessionStore, UserStore};

/// User store backed by a shared `SqlitePool`. Mirrors
/// [`super::postgres::PostgresUserStore`] in shape so callers swap one
/// for the other based on the engine's selected backend.
#[derive(Clone)]
pub struct SqliteUserStore {
    pool: SqlitePool,
}

impl SqliteUserStore {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    /// Wrap into an `Arc<dyn UserStore>` for [`crate::ctx::AuthCtx`].
    pub fn into_dyn(self) -> Arc<dyn UserStore> {
        Arc::new(self)
    }
}

#[async_trait::async_trait]
impl UserStore for SqliteUserStore {
    async fn create_user(&self, user: &User) -> Result<()> {
        sqlx::query(
            "INSERT INTO auth.users
                 (id, email, email_verified, display_name, password_hash, created_at)
             VALUES (?, ?, ?, ?, NULL, ?)",
        )
        .bind(&user.id)
        .bind(&user.email)
        .bind(if user.email_verified { 1i64 } else { 0i64 })
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
             FROM auth.users WHERE id = ?",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .context("auth.users select by id")?;
        Ok(row.map(map_user_row_sqlite))
    }

    async fn get_user_by_email(&self, email: &str) -> Result<Option<User>> {
        let row = sqlx::query(
            "SELECT id, email, email_verified, display_name, created_at
             FROM auth.users WHERE email = ?",
        )
        .bind(email)
        .fetch_optional(&self.pool)
        .await
        .context("auth.users select by email")?;
        Ok(row.map(map_user_row_sqlite))
    }

    async fn update_user(&self, user: &User) -> Result<()> {
        sqlx::query(
            "UPDATE auth.users
             SET email = ?,
                 email_verified = ?,
                 display_name = ?
             WHERE id = ?",
        )
        .bind(&user.email)
        .bind(if user.email_verified { 1i64 } else { 0i64 })
        .bind(&user.display_name)
        .bind(&user.id)
        .execute(&self.pool)
        .await
        .context("auth.users update")?;
        Ok(())
    }

    async fn set_password_hash(&self, user_id: &str, hash: &str) -> Result<()> {
        sqlx::query("UPDATE auth.users SET password_hash = ? WHERE id = ?")
            .bind(hash)
            .bind(user_id)
            .execute(&self.pool)
            .await
            .context("auth.users set password_hash")?;
        Ok(())
    }

    async fn get_password_hash(&self, user_id: &str) -> Result<Option<String>> {
        let row: Option<(Option<String>,)> =
            sqlx::query_as("SELECT password_hash FROM auth.users WHERE id = ?")
                .bind(user_id)
                .fetch_optional(&self.pool)
                .await
                .context("auth.users select password_hash")?;
        Ok(row.and_then(|r| r.0))
    }

    async fn list_passkeys(&self, user_id: &str) -> Result<Vec<PasskeyCred>> {
        let rows = sqlx::query(
            "SELECT credential_id, public_key, sign_count, transports, created_at, passkey_json
             FROM auth.passkeys WHERE user_id = ?
             ORDER BY created_at",
        )
        .bind(user_id)
        .fetch_all(&self.pool)
        .await
        .context("auth.passkeys list")?;
        Ok(rows.into_iter().map(map_passkey_row_sqlite).collect())
    }

    async fn add_passkey(&self, user_id: &str, cred: &PasskeyCred) -> Result<()> {
        sqlx::query(
            "INSERT INTO auth.passkeys
                 (credential_id, user_id, public_key, sign_count, transports, created_at, passkey_json)
             VALUES (?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&cred.credential_id)
        .bind(user_id)
        .bind(&cred.public_key)
        .bind(cred.sign_count as i64)
        .bind(cred.transports.join(","))
        .bind(cred.created_at)
        .bind(cred.passkey_json.as_deref())
        .execute(&self.pool)
        .await
        .context("auth.passkeys insert")?;
        Ok(())
    }

    async fn remove_passkey(&self, credential_id: &[u8]) -> Result<bool> {
        let res = sqlx::query("DELETE FROM auth.passkeys WHERE credential_id = ?")
            .bind(credential_id)
            .execute(&self.pool)
            .await
            .context("auth.passkeys delete")?;
        Ok(res.rows_affected() > 0)
    }

    async fn get_passkey(
        &self,
        credential_id: &[u8],
    ) -> Result<Option<(String, PasskeyCred)>> {
        let row = sqlx::query(
            "SELECT credential_id, user_id, public_key, sign_count, transports, created_at, passkey_json
             FROM auth.passkeys WHERE credential_id = ?",
        )
        .bind(credential_id)
        .fetch_optional(&self.pool)
        .await
        .context("auth.passkeys get")?;
        Ok(row.map(|r| {
            let user_id: String = r.get("user_id");
            (user_id, map_passkey_row_sqlite(r))
        }))
    }

    async fn update_passkey_counter(
        &self,
        credential_id: &[u8],
        sign_count: u32,
        passkey_json: &str,
    ) -> Result<bool> {
        let res = sqlx::query(
            "UPDATE auth.passkeys SET sign_count = ?, passkey_json = ?
             WHERE credential_id = ?",
        )
        .bind(sign_count as i64)
        .bind(passkey_json)
        .bind(credential_id)
        .execute(&self.pool)
        .await
        .context("auth.passkeys counter update")?;
        Ok(res.rows_affected() > 0)
    }

    async fn link_upstream(&self, user_id: &str, provider: &str, subject: &str) -> Result<()> {
        sqlx::query(
            "INSERT INTO auth.user_upstream (provider, subject, user_id)
             VALUES (?, ?, ?)
             ON CONFLICT (provider, subject) DO UPDATE SET user_id = excluded.user_id",
        )
        .bind(provider)
        .bind(subject)
        .bind(user_id)
        .execute(&self.pool)
        .await
        .context("auth.user_upstream upsert")?;
        Ok(())
    }

    async fn get_user_by_upstream(&self, provider: &str, subject: &str) -> Result<Option<User>> {
        let row = sqlx::query(
            "SELECT u.id, u.email, u.email_verified, u.display_name, u.created_at
             FROM auth.users u
             JOIN auth.user_upstream l ON l.user_id = u.id
             WHERE l.provider = ? AND l.subject = ?",
        )
        .bind(provider)
        .bind(subject)
        .fetch_optional(&self.pool)
        .await
        .context("auth.user_upstream lookup")?;
        Ok(row.map(map_user_row_sqlite))
    }

    async fn list_users(&self, limit: i64, offset: i64, search: Option<&str>) -> Result<Vec<User>> {
        let lim = limit.clamp(1, 500);
        let off = offset.max(0);
        let rows = if let Some(needle) = search {
            let pat = format!("%{}%", needle.to_lowercase());
            sqlx::query(
                "SELECT id, email, email_verified, display_name, created_at
                 FROM auth.users
                 WHERE LOWER(COALESCE(email, '')) LIKE ?
                    OR LOWER(COALESCE(display_name, '')) LIKE ?
                 ORDER BY created_at DESC
                 LIMIT ? OFFSET ?",
            )
            .bind(&pat)
            .bind(&pat)
            .bind(lim)
            .bind(off)
            .fetch_all(&self.pool)
            .await
            .context("auth.users list (search)")?
        } else {
            sqlx::query(
                "SELECT id, email, email_verified, display_name, created_at
                 FROM auth.users
                 ORDER BY created_at DESC
                 LIMIT ? OFFSET ?",
            )
            .bind(lim)
            .bind(off)
            .fetch_all(&self.pool)
            .await
            .context("auth.users list")?
        };
        Ok(rows.into_iter().map(map_user_row_sqlite).collect())
    }

    async fn count_users(&self, search: Option<&str>) -> Result<i64> {
        let row: (i64,) = if let Some(needle) = search {
            let pat = format!("%{}%", needle.to_lowercase());
            sqlx::query_as(
                "SELECT COUNT(*) FROM auth.users
                 WHERE LOWER(COALESCE(email, '')) LIKE ?
                    OR LOWER(COALESCE(display_name, '')) LIKE ?",
            )
            .bind(&pat)
            .bind(&pat)
            .fetch_one(&self.pool)
            .await
            .context("auth.users count (search)")?
        } else {
            sqlx::query_as("SELECT COUNT(*) FROM auth.users")
                .fetch_one(&self.pool)
                .await
                .context("auth.users count")?
        };
        Ok(row.0)
    }

    async fn delete_user(&self, id: &str) -> Result<bool> {
        let res = sqlx::query("DELETE FROM auth.users WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await
            .context("auth.users delete")?;
        Ok(res.rows_affected() > 0)
    }

    async fn list_upstream_for_user(&self, user_id: &str) -> Result<Vec<(String, String)>> {
        let rows = sqlx::query(
            "SELECT provider, subject FROM auth.user_upstream
             WHERE user_id = ? ORDER BY provider, subject",
        )
        .bind(user_id)
        .fetch_all(&self.pool)
        .await
        .context("auth.user_upstream list")?;
        Ok(rows
            .into_iter()
            .map(|r| {
                (
                    r.get::<String, _>("provider"),
                    r.get::<String, _>("subject"),
                )
            })
            .collect())
    }
}

/// Session store backed by `auth.sessions`. Independent struct from
/// [`SqliteUserStore`] because they're independently mockable in
/// tests and the engine may swap one without the other.
#[derive(Clone)]
pub struct SqliteSessionStore {
    pool: SqlitePool,
}

impl SqliteSessionStore {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    pub fn into_dyn(self) -> Arc<dyn SessionStore> {
        Arc::new(self)
    }
}

#[async_trait::async_trait]
impl SessionStore for SqliteSessionStore {
    async fn create(&self, session: &Session) -> Result<()> {
        sqlx::query(
            "INSERT INTO auth.sessions
                 (id, user_id, csrf_token, created_at, expires_at, ip_hash, user_agent_hash)
             VALUES (?, ?, ?, ?, ?, ?, ?)",
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
             FROM auth.sessions WHERE id = ?",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .context("auth.sessions select")?;
        Ok(row.map(map_session_row_sqlite))
    }

    async fn delete(&self, id: &str) -> Result<bool> {
        let res = sqlx::query("DELETE FROM auth.sessions WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await
            .context("auth.sessions delete")?;
        Ok(res.rows_affected() > 0)
    }

    async fn list_for_user(&self, user_id: &str) -> Result<Vec<Session>> {
        let rows = sqlx::query(
            "SELECT id, user_id, csrf_token, created_at, expires_at, ip_hash, user_agent_hash
             FROM auth.sessions WHERE user_id = ? ORDER BY created_at DESC",
        )
        .bind(user_id)
        .fetch_all(&self.pool)
        .await
        .context("auth.sessions list_for_user")?;
        Ok(rows.into_iter().map(map_session_row_sqlite).collect())
    }

    async fn delete_for_user(&self, user_id: &str) -> Result<u64> {
        let res = sqlx::query("DELETE FROM auth.sessions WHERE user_id = ?")
            .bind(user_id)
            .execute(&self.pool)
            .await
            .context("auth.sessions delete_for_user")?;
        Ok(res.rows_affected())
    }

    async fn purge_expired(&self, now: f64) -> Result<u64> {
        let res = sqlx::query("DELETE FROM auth.sessions WHERE expires_at <= ?")
            .bind(now)
            .execute(&self.pool)
            .await
            .context("auth.sessions purge_expired")?;
        Ok(res.rows_affected())
    }

    async fn list_all(
        &self,
        limit: i64,
        offset: i64,
        user_filter: Option<&str>,
    ) -> Result<Vec<Session>> {
        let lim = limit.clamp(1, 500);
        let off = offset.max(0);
        let rows = if let Some(uid) = user_filter {
            sqlx::query(
                "SELECT id, user_id, csrf_token, created_at, expires_at, ip_hash, user_agent_hash
                 FROM auth.sessions WHERE user_id = ?
                 ORDER BY created_at DESC
                 LIMIT ? OFFSET ?",
            )
            .bind(uid)
            .bind(lim)
            .bind(off)
            .fetch_all(&self.pool)
            .await
            .context("auth.sessions list_all (user filter)")?
        } else {
            sqlx::query(
                "SELECT id, user_id, csrf_token, created_at, expires_at, ip_hash, user_agent_hash
                 FROM auth.sessions
                 ORDER BY created_at DESC
                 LIMIT ? OFFSET ?",
            )
            .bind(lim)
            .bind(off)
            .fetch_all(&self.pool)
            .await
            .context("auth.sessions list_all")?
        };
        Ok(rows.into_iter().map(map_session_row_sqlite).collect())
    }

    async fn count_all(&self, user_filter: Option<&str>) -> Result<i64> {
        let row: (i64,) = if let Some(uid) = user_filter {
            sqlx::query_as("SELECT COUNT(*) FROM auth.sessions WHERE user_id = ?")
                .bind(uid)
                .fetch_one(&self.pool)
                .await
                .context("auth.sessions count_all (user filter)")?
        } else {
            sqlx::query_as("SELECT COUNT(*) FROM auth.sessions")
                .fetch_one(&self.pool)
                .await
                .context("auth.sessions count_all")?
        };
        Ok(row.0)
    }
}

fn map_user_row_sqlite(row: sqlx::sqlite::SqliteRow) -> User {
    let email_verified: i64 = row.get("email_verified");
    User {
        id: row.get("id"),
        email: row.get("email"),
        email_verified: email_verified != 0,
        display_name: row.get("display_name"),
        created_at: row.get("created_at"),
    }
}

fn map_session_row_sqlite(row: sqlx::sqlite::SqliteRow) -> Session {
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

fn map_passkey_row_sqlite(row: sqlx::sqlite::SqliteRow) -> PasskeyCred {
    let transports: String = row.get("transports");
    let sign_count: i64 = row.get("sign_count");
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
        passkey_json: row.get("passkey_json"),
    }
}
