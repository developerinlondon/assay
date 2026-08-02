use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use rand::RngCore;
use sha2::{Digest, Sha256};
use url::Url;

use crate::AuthCtx;
use crate::password::PasswordHasher;

#[async_trait::async_trait]
pub trait RecoveryStore: Send + Sync + 'static {
    async fn issue(
        &self,
        email: &str,
        token_hash: &str,
        created_at: f64,
        expires_at: f64,
        cooldown_before: f64,
    ) -> anyhow::Result<Option<String>>;

    async fn delete(&self, token_hash: &str) -> anyhow::Result<()>;

    async fn valid(&self, token_hash: &str, now: f64) -> anyhow::Result<bool>;

    async fn complete(
        &self,
        token_hash: &str,
        now: f64,
        password_hash: &str,
    ) -> anyhow::Result<bool>;
}

#[async_trait::async_trait]
pub trait RecoveryMailer: Send + Sync + 'static {
    async fn send(&self, recipient: &str, reset_url: &str) -> anyhow::Result<()>;
}

#[derive(Clone)]
pub struct SmtpRecoverySettings {
    pub host: String,
    pub port: u16,
    pub username: String,
    pub password: String,
    pub from: String,
    pub starttls: bool,
}

#[derive(Clone)]
pub struct SmtpRecoveryMailer {
    transport: lettre::AsyncSmtpTransport<lettre::Tokio1Executor>,
    from: lettre::message::Mailbox,
}

impl SmtpRecoveryMailer {
    pub fn new(settings: SmtpRecoverySettings) -> anyhow::Result<Self> {
        use lettre::transport::smtp::authentication::Credentials;
        if settings.host.trim().is_empty() {
            return Err(anyhow::anyhow!("SMTP host is empty"));
        }
        let from = settings
            .from
            .parse()
            .map_err(|error| anyhow::anyhow!("invalid SMTP sender: {error}"))?;
        let credentials = Credentials::new(settings.username, settings.password);
        let builder = if settings.starttls {
            lettre::AsyncSmtpTransport::<lettre::Tokio1Executor>::starttls_relay(&settings.host)
                .map_err(|error| anyhow::anyhow!("invalid SMTP relay: {error}"))?
        } else {
            lettre::AsyncSmtpTransport::<lettre::Tokio1Executor>::builder_dangerous(settings.host)
        };
        let transport = builder
            .port(settings.port)
            .credentials(credentials)
            .timeout(Some(Duration::from_secs(10)))
            .build();
        Ok(Self { transport, from })
    }

    fn message(&self, recipient: &str, reset_url: &str) -> anyhow::Result<lettre::Message> {
        use lettre::message::header::ContentType;
        let recipient = recipient
            .parse()
            .map_err(|error| anyhow::anyhow!("invalid recovery recipient: {error}"))?;
        lettre::Message::builder()
            .from(self.from.clone())
            .to(recipient)
            .subject("Reset your password")
            .header(ContentType::TEXT_PLAIN)
            .body(format!(
                "A password reset was requested for your account.\n\n\
                 Open this link to choose a new password:\n{reset_url}\n\n\
                 This link expires in 15 minutes and can be used only once.\n\n\
                 If you did not request this, you can ignore this message."
            ))
            .map_err(|error| anyhow::anyhow!("build recovery message: {error}"))
    }
}

#[async_trait::async_trait]
impl RecoveryMailer for SmtpRecoveryMailer {
    async fn send(&self, recipient: &str, reset_url: &str) -> anyhow::Result<()> {
        use lettre::AsyncTransport;
        let message = self.message(recipient, reset_url)?;
        self.transport
            .send(message)
            .await
            .map_err(|error| anyhow::anyhow!("send recovery message: {error}"))?;
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RecoveryRequestStatus {
    Delivered,
    Suppressed,
}

#[derive(Clone)]
pub struct PasswordRecovery {
    store: Arc<dyn RecoveryStore>,
    mailer: Arc<dyn RecoveryMailer>,
    recovery_url: Url,
    token_ttl: Duration,
    request_cooldown: Duration,
}

impl PasswordRecovery {
    pub fn new(
        store: Arc<dyn RecoveryStore>,
        mailer: Arc<dyn RecoveryMailer>,
        recovery_url: Url,
        token_ttl: Duration,
        request_cooldown: Duration,
    ) -> Self {
        Self {
            store,
            mailer,
            recovery_url,
            token_ttl,
            request_cooldown,
        }
    }

    pub async fn request(&self, email: &str) -> anyhow::Result<RecoveryRequestStatus> {
        let raw_token = random_token();
        let token_hash = token_hash(&raw_token);
        let created_at = now_secs();
        let expires_at = created_at + self.token_ttl.as_secs_f64();
        let cooldown_before = created_at - self.request_cooldown.as_secs_f64();
        let Some(recipient) = self
            .store
            .issue(email, &token_hash, created_at, expires_at, cooldown_before)
            .await?
        else {
            return Ok(RecoveryRequestStatus::Suppressed);
        };

        let mut reset_url = self.recovery_url.clone();
        reset_url.set_fragment(Some(&format!("token={raw_token}")));
        if let Err(delivery_error) = self.mailer.send(&recipient, reset_url.as_str()).await {
            if let Err(cleanup_error) = self.store.delete(&token_hash).await {
                return Err(anyhow::anyhow!(
                    "{delivery_error}; recovery token cleanup failed: {cleanup_error}"
                ));
            }
            return Err(delivery_error);
        }
        Ok(RecoveryRequestStatus::Delivered)
    }

    pub async fn complete(&self, token: &str, password: &str) -> anyhow::Result<bool> {
        let token_hash = token_hash(token);
        let now = now_secs();
        if !self.store.valid(&token_hash, now).await? {
            return Ok(false);
        }
        let password_hash = PasswordHasher::default().hash(password)?;
        self.store.complete(&token_hash, now, &password_hash).await
    }
}

use axum::Router;
use axum::extract::{FromRef, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Json, Response};
use axum::routing::post;
use serde::Deserialize;
use serde_json::json;

pub fn router<S>() -> Router<S>
where
    S: Clone + Send + Sync + 'static,
    AuthCtx: FromRef<S>,
{
    Router::new()
        .route("/password/recovery/request", post(request_recovery))
        .route("/password/recovery/complete", post(complete_recovery))
}

#[derive(Deserialize)]
struct RequestRecoveryBody {
    email: String,
}

async fn request_recovery(
    State(ctx): State<AuthCtx>,
    Json(body): Json<RequestRecoveryBody>,
) -> Response {
    let Some(recovery) = ctx.recovery else {
        return unavailable();
    };
    tokio::spawn(async move {
        if let Err(error) = recovery.request(&body.email).await {
            tracing::error!(%error, "password recovery request failed");
        }
    });
    (StatusCode::ACCEPTED, Json(json!({"status": "accepted"}))).into_response()
}

#[derive(Deserialize)]
struct CompleteRecoveryBody {
    token: String,
    password: String,
}

async fn complete_recovery(
    State(ctx): State<AuthCtx>,
    Json(body): Json<CompleteRecoveryBody>,
) -> Response {
    let Some(recovery) = ctx.recovery else {
        return unavailable();
    };
    if body.token.is_empty() || body.password.is_empty() || body.password.len() > 1024 {
        return invalid_token();
    }
    match recovery.complete(&body.token, &body.password).await {
        Ok(true) => StatusCode::NO_CONTENT.into_response(),
        Ok(false) => invalid_token(),
        Err(error) => {
            tracing::error!(%error, "password recovery completion failed");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

fn invalid_token() -> Response {
    (
        StatusCode::BAD_REQUEST,
        Json(json!({"error": "invalid_or_expired_token"})),
    )
        .into_response()
}

fn unavailable() -> Response {
    (
        StatusCode::SERVICE_UNAVAILABLE,
        Json(json!({"error": "password_recovery_unavailable"})),
    )
        .into_response()
}

fn random_token() -> String {
    let mut bytes = [0u8; 32];
    rand::rng().fill_bytes(&mut bytes);
    data_encoding::BASE64URL_NOPAD.encode(&bytes)
}

fn token_hash(token: &str) -> String {
    data_encoding::HEXLOWER.encode(&Sha256::digest(token.as_bytes()))
}

fn now_secs() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64()
}

#[cfg(feature = "backend-postgres")]
#[derive(Clone)]
pub struct PostgresRecoveryStore {
    pool: sqlx::PgPool,
}

#[cfg(feature = "backend-postgres")]
impl PostgresRecoveryStore {
    pub fn new(pool: sqlx::PgPool) -> Self {
        Self { pool }
    }
}

#[cfg(feature = "backend-postgres")]
#[async_trait::async_trait]
impl RecoveryStore for PostgresRecoveryStore {
    async fn issue(
        &self,
        email: &str,
        token_hash: &str,
        created_at: f64,
        expires_at: f64,
        cooldown_before: f64,
    ) -> anyhow::Result<Option<String>> {
        use anyhow::Context;
        let mut transaction = self.pool.begin().await.context("begin recovery issue")?;
        sqlx::query("DELETE FROM auth.password_recovery_tokens WHERE expires_at <= $1")
            .bind(created_at)
            .execute(&mut *transaction)
            .await
            .context("purge expired recovery tokens")?;
        let user: Option<(String, String)> = sqlx::query_as(
            "SELECT id, email FROM auth.users
             WHERE LOWER(email) = LOWER($1) AND email_verified = TRUE AND email IS NOT NULL",
        )
        .bind(email)
        .fetch_optional(&mut *transaction)
        .await
        .context("find verified recovery user")?;
        let Some((user_id, recipient)) = user else {
            transaction
                .commit()
                .await
                .context("commit suppressed recovery")?;
            return Ok(None);
        };
        let result = sqlx::query(
            "INSERT INTO auth.password_recovery_tokens
                 (token_hash, user_id, created_at, expires_at)
             VALUES ($1, $2, $3, $4)
             ON CONFLICT (user_id) DO UPDATE SET
                 token_hash = EXCLUDED.token_hash,
                 created_at = EXCLUDED.created_at,
                 expires_at = EXCLUDED.expires_at
             WHERE password_recovery_tokens.created_at <= $5",
        )
        .bind(token_hash)
        .bind(user_id)
        .bind(created_at)
        .bind(expires_at)
        .bind(cooldown_before)
        .execute(&mut *transaction)
        .await
        .context("issue recovery token")?;
        transaction
            .commit()
            .await
            .context("commit recovery issue")?;
        Ok((result.rows_affected() == 1).then_some(recipient))
    }

    async fn delete(&self, token_hash: &str) -> anyhow::Result<()> {
        use anyhow::Context;
        sqlx::query("DELETE FROM auth.password_recovery_tokens WHERE token_hash = $1")
            .bind(token_hash)
            .execute(&self.pool)
            .await
            .context("delete recovery token")?;
        Ok(())
    }

    async fn valid(&self, token_hash: &str, now: f64) -> anyhow::Result<bool> {
        use anyhow::Context;
        sqlx::query_scalar(
            "SELECT EXISTS(
                 SELECT 1 FROM auth.password_recovery_tokens
                 WHERE token_hash = $1 AND expires_at > $2
             )",
        )
        .bind(token_hash)
        .bind(now)
        .fetch_one(&self.pool)
        .await
        .context("validate recovery token")
    }

    async fn complete(
        &self,
        token_hash: &str,
        now: f64,
        password_hash: &str,
    ) -> anyhow::Result<bool> {
        use anyhow::Context;
        let mut transaction = self
            .pool
            .begin()
            .await
            .context("begin recovery completion")?;
        let user_id: Option<String> = sqlx::query_scalar(
            "DELETE FROM auth.password_recovery_tokens
             WHERE token_hash = $1 AND expires_at > $2
             RETURNING user_id",
        )
        .bind(token_hash)
        .bind(now)
        .fetch_optional(&mut *transaction)
        .await
        .context("consume recovery token")?;
        let Some(user_id) = user_id else {
            transaction
                .commit()
                .await
                .context("commit rejected recovery")?;
            return Ok(false);
        };
        sqlx::query("UPDATE auth.users SET password_hash = $2 WHERE id = $1")
            .bind(&user_id)
            .bind(password_hash)
            .execute(&mut *transaction)
            .await
            .context("replace recovered password")?;
        sqlx::query("DELETE FROM auth.sessions WHERE user_id = $1")
            .bind(user_id)
            .execute(&mut *transaction)
            .await
            .context("revoke recovered user sessions")?;
        transaction
            .commit()
            .await
            .context("commit recovery completion")?;
        Ok(true)
    }
}

#[cfg(feature = "backend-sqlite")]
#[derive(Clone)]
pub struct SqliteRecoveryStore {
    pool: sqlx::SqlitePool,
}

#[cfg(feature = "backend-sqlite")]
impl SqliteRecoveryStore {
    pub fn new(pool: sqlx::SqlitePool) -> Self {
        Self { pool }
    }
}

#[cfg(feature = "backend-sqlite")]
#[async_trait::async_trait]
impl RecoveryStore for SqliteRecoveryStore {
    async fn issue(
        &self,
        email: &str,
        token_hash: &str,
        created_at: f64,
        expires_at: f64,
        cooldown_before: f64,
    ) -> anyhow::Result<Option<String>> {
        use anyhow::Context;
        let mut transaction = self.pool.begin().await.context("begin recovery issue")?;
        sqlx::query("DELETE FROM auth.password_recovery_tokens WHERE expires_at <= ?")
            .bind(created_at)
            .execute(&mut *transaction)
            .await
            .context("purge expired recovery tokens")?;
        let user: Option<(String, String)> = sqlx::query_as(
            "SELECT id, email FROM auth.users
             WHERE LOWER(email) = LOWER(?) AND email_verified = 1 AND email IS NOT NULL",
        )
        .bind(email)
        .fetch_optional(&mut *transaction)
        .await
        .context("find verified recovery user")?;
        let Some((user_id, recipient)) = user else {
            transaction
                .commit()
                .await
                .context("commit suppressed recovery")?;
            return Ok(None);
        };
        let result = sqlx::query(
            "INSERT INTO auth.password_recovery_tokens
                 (token_hash, user_id, created_at, expires_at)
             VALUES (?, ?, ?, ?)
             ON CONFLICT (user_id) DO UPDATE SET
                 token_hash = excluded.token_hash,
                 created_at = excluded.created_at,
                 expires_at = excluded.expires_at
             WHERE password_recovery_tokens.created_at <= ?",
        )
        .bind(token_hash)
        .bind(user_id)
        .bind(created_at)
        .bind(expires_at)
        .bind(cooldown_before)
        .execute(&mut *transaction)
        .await
        .context("issue recovery token")?;
        transaction
            .commit()
            .await
            .context("commit recovery issue")?;
        Ok((result.rows_affected() == 1).then_some(recipient))
    }

    async fn delete(&self, token_hash: &str) -> anyhow::Result<()> {
        use anyhow::Context;
        sqlx::query("DELETE FROM auth.password_recovery_tokens WHERE token_hash = ?")
            .bind(token_hash)
            .execute(&self.pool)
            .await
            .context("delete recovery token")?;
        Ok(())
    }

    async fn valid(&self, token_hash: &str, now: f64) -> anyhow::Result<bool> {
        use anyhow::Context;
        sqlx::query_scalar(
            "SELECT EXISTS(
                 SELECT 1 FROM auth.password_recovery_tokens
                 WHERE token_hash = ? AND expires_at > ?
             )",
        )
        .bind(token_hash)
        .bind(now)
        .fetch_one(&self.pool)
        .await
        .context("validate recovery token")
    }

    async fn complete(
        &self,
        token_hash: &str,
        now: f64,
        password_hash: &str,
    ) -> anyhow::Result<bool> {
        use anyhow::Context;
        let mut transaction = self
            .pool
            .begin()
            .await
            .context("begin recovery completion")?;
        let user_id: Option<String> = sqlx::query_scalar(
            "DELETE FROM auth.password_recovery_tokens
             WHERE token_hash = ? AND expires_at > ?
             RETURNING user_id",
        )
        .bind(token_hash)
        .bind(now)
        .fetch_optional(&mut *transaction)
        .await
        .context("consume recovery token")?;
        let Some(user_id) = user_id else {
            transaction
                .commit()
                .await
                .context("commit rejected recovery")?;
            return Ok(false);
        };
        sqlx::query("UPDATE auth.users SET password_hash = ? WHERE id = ?")
            .bind(password_hash)
            .bind(&user_id)
            .execute(&mut *transaction)
            .await
            .context("replace recovered password")?;
        sqlx::query("DELETE FROM auth.sessions WHERE user_id = ?")
            .bind(user_id)
            .execute(&mut *transaction)
            .await
            .context("revoke recovered user sessions")?;
        transaction
            .commit()
            .await
            .context("commit recovery completion")?;
        Ok(true)
    }
}

#[cfg(test)]
mod smtp_tests {
    use super::*;

    fn settings() -> SmtpRecoverySettings {
        SmtpRecoverySettings {
            host: "smtp.example.com".to_string(),
            port: 587,
            username: "smtp-user".to_string(),
            password: "smtp-password".to_string(),
            from: "Example Auth <noreply@example.com>".to_string(),
            starttls: true,
        }
    }

    #[test]
    fn smtp_message_names_the_sender_recipient_expiry_and_fragment_url() {
        let mailer = SmtpRecoveryMailer::new(settings()).unwrap();
        let message = mailer
            .message(
                "user@example.com",
                "https://auth.example.com/auth/recovery#token=secret",
            )
            .unwrap();
        let formatted = String::from_utf8(message.formatted()).unwrap();
        assert!(formatted.contains("From: \"Example Auth\" <noreply@example.com>"));
        assert!(formatted.contains("To: user@example.com"));
        assert!(formatted.contains("Subject: Reset your password"));
        assert!(formatted.contains("https://auth.example.com/auth/recovery#token=secret"));
        assert!(formatted.contains("15 minutes"));
    }

    #[test]
    fn smtp_configuration_rejects_an_invalid_sender_before_boot() {
        let mut invalid = settings();
        invalid.from = "not an address".to_string();
        assert!(SmtpRecoveryMailer::new(invalid).is_err());
    }
}
