use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::anyhow;
use assay_auth::password::PasswordHasher;
use assay_auth::recovery::{
    PasswordRecovery, RecoveryMailer, RecoveryRequestStatus, RecoveryStore,
};
use sha2::{Digest, Sha256};
use url::Url;

#[derive(Default)]
struct StoreState {
    email: Option<String>,
    email_verified: bool,
    token_hash: Option<String>,
    created_at: f64,
    expires_at: f64,
    password_hash: Option<String>,
    sessions_revoked: bool,
}

#[derive(Clone, Default)]
struct MemoryRecoveryStore(Arc<Mutex<StoreState>>);

#[async_trait::async_trait]
impl RecoveryStore for MemoryRecoveryStore {
    async fn issue(
        &self,
        email: &str,
        token_hash: &str,
        created_at: f64,
        expires_at: f64,
        cooldown_before: f64,
    ) -> anyhow::Result<Option<String>> {
        let mut state = self.0.lock().unwrap();
        let Some(recipient) = state.email.clone() else {
            return Ok(None);
        };
        if !state.email_verified || !recipient.eq_ignore_ascii_case(email) {
            return Ok(None);
        }
        if state.token_hash.is_some() && state.created_at > cooldown_before {
            return Ok(None);
        }
        state.token_hash = Some(token_hash.to_string());
        state.created_at = created_at;
        state.expires_at = expires_at;
        Ok(Some(recipient))
    }

    async fn delete(&self, token_hash: &str) -> anyhow::Result<()> {
        let mut state = self.0.lock().unwrap();
        if state.token_hash.as_deref() == Some(token_hash) {
            state.token_hash = None;
        }
        Ok(())
    }

    async fn complete(
        &self,
        token_hash: &str,
        now: f64,
        password_hash: &str,
    ) -> anyhow::Result<bool> {
        let mut state = self.0.lock().unwrap();
        if state.token_hash.as_deref() != Some(token_hash) || state.expires_at <= now {
            return Ok(false);
        }
        state.token_hash = None;
        state.password_hash = Some(password_hash.to_string());
        state.sessions_revoked = true;
        Ok(true)
    }
}

#[derive(Clone, Debug, PartialEq)]
struct Delivery {
    recipient: String,
    reset_url: String,
}

#[derive(Clone, Default)]
struct MemoryMailer {
    deliveries: Arc<Mutex<Vec<Delivery>>>,
    fail: bool,
}

#[async_trait::async_trait]
impl RecoveryMailer for MemoryMailer {
    async fn send(&self, recipient: &str, reset_url: &str) -> anyhow::Result<()> {
        if self.fail {
            return Err(anyhow!("delivery unavailable"));
        }
        self.deliveries.lock().unwrap().push(Delivery {
            recipient: recipient.to_string(),
            reset_url: reset_url.to_string(),
        });
        Ok(())
    }
}

fn manager(store: MemoryRecoveryStore, mailer: MemoryMailer) -> PasswordRecovery {
    PasswordRecovery::new(
        Arc::new(store),
        Arc::new(mailer),
        Url::parse("https://auth.example.com/auth/recovery").unwrap(),
        Duration::from_secs(900),
        Duration::from_secs(60),
    )
}

fn verified_store() -> MemoryRecoveryStore {
    let store = MemoryRecoveryStore::default();
    {
        let mut state = store.0.lock().unwrap();
        state.email = Some("user@example.com".to_string());
        state.email_verified = true;
    }
    store
}

fn raw_token(delivery: &Delivery) -> &str {
    delivery.reset_url.split("#token=").nth(1).unwrap()
}

#[tokio::test]
async fn a_verified_account_receives_a_fragment_url_while_only_the_hash_is_stored() {
    let store = verified_store();
    let mailer = MemoryMailer::default();
    let recovery = manager(store.clone(), mailer.clone());

    let status = recovery.request("USER@example.com").await.unwrap();

    assert_eq!(status, RecoveryRequestStatus::Delivered);
    let deliveries = mailer.deliveries.lock().unwrap();
    assert_eq!(deliveries.len(), 1);
    assert_eq!(deliveries[0].recipient, "user@example.com");
    assert!(
        deliveries[0]
            .reset_url
            .starts_with("https://auth.example.com/auth/recovery#token=")
    );
    let token = raw_token(&deliveries[0]);
    let expected_hash = data_encoding::HEXLOWER.encode(&Sha256::digest(token.as_bytes()));
    assert_eq!(
        store.0.lock().unwrap().token_hash.as_deref(),
        Some(expected_hash.as_str())
    );
    assert_ne!(store.0.lock().unwrap().token_hash.as_deref(), Some(token));
}

#[tokio::test]
async fn unknown_and_unverified_addresses_are_indistinguishable_and_send_nothing() {
    let unknown_store = MemoryRecoveryStore::default();
    let unknown_mailer = MemoryMailer::default();
    let unknown = manager(unknown_store, unknown_mailer.clone());

    let unverified_store = MemoryRecoveryStore::default();
    {
        let mut state = unverified_store.0.lock().unwrap();
        state.email = Some("user@example.com".to_string());
    }
    let unverified_mailer = MemoryMailer::default();
    let unverified = manager(unverified_store, unverified_mailer.clone());

    assert_eq!(
        unknown.request("user@example.com").await.unwrap(),
        RecoveryRequestStatus::Suppressed
    );
    assert_eq!(
        unverified.request("user@example.com").await.unwrap(),
        RecoveryRequestStatus::Suppressed
    );
    assert!(unknown_mailer.deliveries.lock().unwrap().is_empty());
    assert!(unverified_mailer.deliveries.lock().unwrap().is_empty());
}

#[tokio::test]
async fn a_delivery_failure_removes_the_undeliverable_token() {
    let store = verified_store();
    let mailer = MemoryMailer {
        fail: true,
        ..MemoryMailer::default()
    };
    let recovery = manager(store.clone(), mailer);

    let error = recovery.request("user@example.com").await.unwrap_err();

    assert!(error.to_string().contains("delivery unavailable"));
    assert!(store.0.lock().unwrap().token_hash.is_none());
}

#[tokio::test]
async fn completion_changes_the_password_revokes_sessions_and_consumes_the_token() {
    let store = verified_store();
    let mailer = MemoryMailer::default();
    let recovery = manager(store.clone(), mailer.clone());
    recovery.request("user@example.com").await.unwrap();
    let delivery = mailer.deliveries.lock().unwrap()[0].clone();
    let token = raw_token(&delivery);

    assert!(
        recovery
            .complete(token, "replacement password")
            .await
            .unwrap()
    );

    {
        let state = store.0.lock().unwrap();
        let password_hash = state.password_hash.as_deref().unwrap();
        assert!(
            PasswordHasher::default()
                .verify("replacement password", password_hash)
                .unwrap()
        );
        assert!(state.sessions_revoked);
        assert!(state.token_hash.is_none());
    }
    assert!(!recovery.complete(token, "another password").await.unwrap());
}

#[tokio::test]
async fn expired_tokens_cannot_change_the_password() {
    let store = verified_store();
    let recovery = manager(store.clone(), MemoryMailer::default());
    let token = "expired-token";
    {
        let mut state = store.0.lock().unwrap();
        state.token_hash = Some(data_encoding::HEXLOWER.encode(&Sha256::digest(token.as_bytes())));
        state.expires_at = 0.0;
    }

    assert!(
        !recovery
            .complete(token, "replacement password")
            .await
            .unwrap()
    );
    let state = store.0.lock().unwrap();
    assert!(state.password_hash.is_none());
    assert!(!state.sessions_revoked);
}

#[tokio::test]
async fn requests_inside_the_cooldown_do_not_send_another_message() {
    let store = verified_store();
    let mailer = MemoryMailer::default();
    let recovery = manager(store, mailer.clone());

    assert_eq!(
        recovery.request("user@example.com").await.unwrap(),
        RecoveryRequestStatus::Delivered
    );
    assert_eq!(
        recovery.request("user@example.com").await.unwrap(),
        RecoveryRequestStatus::Suppressed
    );
    assert_eq!(mailer.deliveries.lock().unwrap().len(), 1);
}

#[cfg(feature = "backend-sqlite")]
mod sqlite_store {
    use super::*;
    use assay_auth::AuthCtx;
    use assay_auth::recovery::SqliteRecoveryStore;
    use assay_auth::store::{SqliteSessionStore, SqliteUserStore};
    use axum::body::{Body, to_bytes};
    use axum::extract::FromRef;
    use axum::http::{Request, StatusCode};
    use sqlx::SqlitePool;
    use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
    use std::str::FromStr;
    use tower::ServiceExt;

    #[derive(Clone)]
    struct TestState {
        auth: AuthCtx,
    }

    impl FromRef<TestState> for AuthCtx {
        fn from_ref(state: &TestState) -> Self {
            state.auth.clone()
        }
    }

    #[derive(Clone)]
    struct DelayedMailer;

    #[async_trait::async_trait]
    impl RecoveryMailer for DelayedMailer {
        async fn send(&self, _recipient: &str, _reset_url: &str) -> anyhow::Result<()> {
            tokio::time::sleep(Duration::from_secs(1)).await;
            Ok(())
        }
    }

    async fn setup() -> SqlitePool {
        let suffix = format!("{}_{}_recovery", std::process::id(), uuid::Uuid::new_v4());
        let engine_uri = format!("file:assay_eng_{suffix}?mode=memory&cache=shared");
        let auth_uri = format!("file:assay_auth_{suffix}?mode=memory&cache=shared");
        let options = SqliteConnectOptions::from_str("sqlite::memory:")
            .unwrap()
            .create_if_missing(true);
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .after_connect(move |connection, metadata| {
                let engine_uri = engine_uri.clone();
                let auth_uri = auth_uri.clone();
                Box::pin(async move {
                    use sqlx::Executor;
                    let _ = metadata;
                    connection
                        .execute(format!("ATTACH DATABASE '{engine_uri}' AS engine").as_str())
                        .await?;
                    connection
                        .execute(format!("ATTACH DATABASE '{auth_uri}' AS auth").as_str())
                        .await?;
                    Ok(())
                })
            })
            .connect_with(options)
            .await
            .unwrap();
        sqlx::query(
            "CREATE TABLE engine.migrations (
                module TEXT NOT NULL,
                version INTEGER NOT NULL,
                PRIMARY KEY (module, version)
            )",
        )
        .execute(&pool)
        .await
        .unwrap();
        assay_auth::schema::migrate_sqlite(&pool).await.unwrap();
        pool
    }

    #[tokio::test]
    async fn completion_is_single_use_and_atomically_revokes_sessions() {
        let pool = setup().await;
        let version: i64 =
            sqlx::query_scalar("SELECT MAX(version) FROM engine.migrations WHERE module = 'auth'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(version, 6);
        sqlx::query(
            "INSERT INTO auth.users
             (id, email, email_verified, display_name, password_hash, created_at)
             VALUES ('user-1', 'user@example.com', 1, NULL, 'old-hash', 1)",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO auth.sessions
             (id, user_id, csrf_token, created_at, expires_at)
             VALUES ('session-1', 'user-1', 'csrf', 1, 9999)",
        )
        .execute(&pool)
        .await
        .unwrap();
        let store = SqliteRecoveryStore::new(pool.clone());

        assert_eq!(
            store
                .issue("USER@example.com", "hash-1", 100.0, 1000.0, 40.0)
                .await
                .unwrap()
                .as_deref(),
            Some("user@example.com")
        );
        assert!(
            store
                .issue("user@example.com", "hash-2", 110.0, 1010.0, 50.0)
                .await
                .unwrap()
                .is_none()
        );
        assert!(store.complete("hash-1", 200.0, "new-hash").await.unwrap());
        assert!(!store.complete("hash-1", 201.0, "other-hash").await.unwrap());

        let password: String =
            sqlx::query_scalar("SELECT password_hash FROM auth.users WHERE id = 'user-1'")
                .fetch_one(&pool)
                .await
                .unwrap();
        let sessions: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM auth.sessions WHERE user_id = 'user-1'")
                .fetch_one(&pool)
                .await
                .unwrap();
        let tokens: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM auth.password_recovery_tokens")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(password, "new-hash");
        assert_eq!(sessions, 0);
        assert_eq!(tokens, 0);
    }

    #[tokio::test]
    async fn public_endpoints_do_not_enumerate_accounts_and_complete_a_valid_reset() {
        let pool = setup().await;
        sqlx::query(
            "INSERT INTO auth.users
             (id, email, email_verified, display_name, password_hash, created_at)
             VALUES ('user-http', 'user@example.com', 1, NULL, 'old-hash', 1)",
        )
        .execute(&pool)
        .await
        .unwrap();
        let mailer = MemoryMailer::default();
        let recovery = PasswordRecovery::new(
            Arc::new(SqliteRecoveryStore::new(pool.clone())),
            Arc::new(mailer.clone()),
            Url::parse("https://auth.example.com/auth/recovery").unwrap(),
            Duration::from_secs(900),
            Duration::from_secs(60),
        );
        let auth = AuthCtx::new(
            Arc::new(SqliteUserStore::new(pool.clone())),
            Arc::new(SqliteSessionStore::new(pool.clone())),
        )
        .with_recovery(recovery);
        let app = assay_auth::recovery::router::<TestState>().with_state(TestState { auth });

        let known = app
            .clone()
            .oneshot(
                Request::post("/password/recovery/request")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"email":"user@example.com"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        let unknown = app
            .clone()
            .oneshot(
                Request::post("/password/recovery/request")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"email":"missing@example.com"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(known.status(), StatusCode::ACCEPTED);
        assert_eq!(unknown.status(), StatusCode::ACCEPTED);
        assert_eq!(
            to_bytes(known.into_body(), 1024).await.unwrap(),
            to_bytes(unknown.into_body(), 1024).await.unwrap()
        );

        let delivery = tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                if let Some(delivery) = mailer.deliveries.lock().unwrap().first().cloned() {
                    break delivery;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("recovery email was not delivered");
        let complete = app
            .oneshot(
                Request::post("/password/recovery/complete")
                    .header("content-type", "application/json")
                    .body(Body::from(format!(
                        r#"{{"token":"{}","password":"replacement password"}}"#,
                        raw_token(&delivery)
                    )))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(complete.status(), StatusCode::NO_CONTENT);
    }

    #[tokio::test]
    async fn recovery_request_response_does_not_wait_for_mail_delivery() {
        let pool = setup().await;
        sqlx::query(
            "INSERT INTO auth.users
             (id, email, email_verified, display_name, password_hash, created_at)
             VALUES ('user-latency', 'latency@example.com', 1, NULL, 'old-hash', 1)",
        )
        .execute(&pool)
        .await
        .unwrap();
        let recovery = PasswordRecovery::new(
            Arc::new(SqliteRecoveryStore::new(pool.clone())),
            Arc::new(DelayedMailer),
            Url::parse("https://auth.example.com/auth/recovery").unwrap(),
            Duration::from_secs(900),
            Duration::from_secs(60),
        );
        let auth = AuthCtx::new(
            Arc::new(SqliteUserStore::new(pool.clone())),
            Arc::new(SqliteSessionStore::new(pool)),
        )
        .with_recovery(recovery);
        let app = assay_auth::recovery::router::<TestState>().with_state(TestState { auth });
        let response = tokio::time::timeout(
            Duration::from_millis(250),
            app.oneshot(
                Request::post("/password/recovery/request")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"email":"latency@example.com"}"#))
                    .unwrap(),
            ),
        )
        .await;

        assert!(response.is_ok(), "request waited for SMTP delivery");
        assert_eq!(response.unwrap().unwrap().status(), StatusCode::ACCEPTED);
    }
}

#[cfg(feature = "backend-postgres")]
mod postgres_store {
    use super::*;
    use assay_auth::recovery::PostgresRecoveryStore;

    async fn setup() -> Option<sqlx::PgPool> {
        let url = std::env::var("ASSAY_TEST_DATABASE_URL").ok()?;
        if url.trim().is_empty() {
            return None;
        }
        let pool = sqlx::PgPool::connect(&url).await.ok()?;
        sqlx::query("CREATE SCHEMA IF NOT EXISTS engine")
            .execute(&pool)
            .await
            .ok()?;
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS engine.migrations (
                module TEXT NOT NULL,
                version INTEGER NOT NULL,
                PRIMARY KEY (module, version)
            )",
        )
        .execute(&pool)
        .await
        .ok()?;
        sqlx::query("DROP SCHEMA IF EXISTS auth CASCADE")
            .execute(&pool)
            .await
            .ok()?;
        sqlx::query("DELETE FROM engine.migrations WHERE module = 'auth'")
            .execute(&pool)
            .await
            .ok()?;
        assay_auth::schema::migrate_postgres(&pool).await.ok()?;
        Some(pool)
    }

    #[tokio::test]
    async fn completion_is_single_use_and_atomically_revokes_sessions() {
        let Some(pool) = setup().await else {
            eprintln!("skipping (ASSAY_TEST_DATABASE_URL not set)");
            return;
        };
        sqlx::query(
            "INSERT INTO auth.users
             (id, email, email_verified, display_name, password_hash, created_at)
             VALUES ('user-recovery', 'user@example.com', TRUE, NULL, 'old-hash', 1)",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO auth.sessions
             (id, user_id, csrf_token, created_at, expires_at)
             VALUES ('session-recovery', 'user-recovery', 'csrf', 1, 9999)",
        )
        .execute(&pool)
        .await
        .unwrap();
        let store = PostgresRecoveryStore::new(pool.clone());

        assert_eq!(
            store
                .issue("user@example.com", "hash-pg", 100.0, 1000.0, 40.0)
                .await
                .unwrap()
                .as_deref(),
            Some("user@example.com")
        );
        assert!(store.complete("hash-pg", 200.0, "new-hash").await.unwrap());
        assert!(
            !store
                .complete("hash-pg", 201.0, "other-hash")
                .await
                .unwrap()
        );

        let password: String =
            sqlx::query_scalar("SELECT password_hash FROM auth.users WHERE id = 'user-recovery'")
                .fetch_one(&pool)
                .await
                .unwrap();
        let sessions: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM auth.sessions WHERE user_id = 'user-recovery'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(password, "new-hash");
        assert_eq!(sessions, 0);
    }
}
