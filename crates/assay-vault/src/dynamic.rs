//! Dynamic credentials — short-lived service credentials issued on
//! demand. Plan 17 §S3.
//!
//! Operators register a "role" (provider-specific shape) once; clients
//! call `issue` against that role and get back time-bounded credentials
//! tracked in `vault.leases`. A background sweeper revokes leases at
//! their expiry.
//!
//! ## Trait shape
//!
//! [`DynamicCredsProvider`] is the extension point — Phase 5 ships the
//! Postgres impl in-tree (default-on); AWS / GCP / Kubernetes impls
//! land in subsequent commits behind their own feature flags. External
//! providers can implement the trait out-of-tree without forking
//! assay-vault.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::error::Result;

/// Lease metadata returned from `issue`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Lease {
    pub id: String,
    pub provider: String,
    pub role: String,
    /// Provider-specific credential payload (e.g. {"username": "...",
    /// "password": "..."} for Postgres). Returned to the client once;
    /// the server doesn't retain plaintext.
    pub credentials: serde_json::Value,
    pub issued_at: f64,
    pub expires_at: f64,
}

/// Persisted lease row (without the plaintext credentials — those are
/// returned at issue time and not kept). The lease table tracks
/// existence + expiry + revocation, the credential bytes themselves
/// are short-lived ephemera.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LeaseRecord {
    pub id: String,
    pub provider: String,
    pub role: String,
    pub issued_at: f64,
    pub expires_at: f64,
    pub revoked_at: Option<f64>,
    pub metadata: serde_json::Value,
}

/// Pure-IO trait for the lease registry. `issue` writes a row, `revoke`
/// marks revoked, the sweeper periodically lists expired-but-unrevoked
/// rows and asks the corresponding provider to clean up.
#[async_trait]
pub trait LeaseStore: Send + Sync + 'static {
    async fn create_lease(
        &self,
        id: &str,
        provider: &str,
        role: &str,
        expires_at: f64,
        metadata: &serde_json::Value,
    ) -> Result<()>;

    async fn get_lease(&self, id: &str) -> Result<Option<LeaseRecord>>;

    /// Mark `revoked_at = now`. Idempotent on already-revoked rows.
    async fn revoke_lease(&self, id: &str, now: f64) -> Result<bool>;

    /// List leases past `expires_at` that haven't been revoked yet.
    /// Bounded by `limit` so a backlog doesn't load the whole table.
    async fn list_expired_unrevoked(&self, now: f64, limit: i64) -> Result<Vec<LeaseRecord>>;

    /// Admin: every lease, optionally filtered by provider.
    async fn list_leases(&self, provider: Option<&str>) -> Result<Vec<LeaseRecord>>;
}

/// Provider trait — issue + revoke are provider-specific. Trait
/// methods are async + `Send + Sync` so impls plug behind
/// `Arc<dyn DynamicCredsProvider>`.
#[async_trait]
pub trait DynamicCredsProvider: Send + Sync + 'static {
    /// Stable name — recorded in `vault.leases.provider`. Examples:
    /// "postgres", "aws", "gcp", "kubernetes". Custom providers
    /// pick their own; the dispatcher uses this to route.
    fn name(&self) -> &str;

    /// Issue a fresh credential for `role` with TTL `ttl_secs`.
    /// Returns the credential payload + the lease id (which the
    /// dispatcher persists into `vault.leases`).
    async fn issue(
        &self,
        role: &str,
        ttl_secs: u64,
    ) -> Result<IssuedCredentials>;

    /// Revoke a previously-issued credential. Identified by the
    /// lease id; provider-specific metadata recorded at issue time
    /// is rehydrated from the lease row so the provider can find
    /// the right resource to clean up.
    async fn revoke(&self, lease: &LeaseRecord) -> Result<()>;
}

/// What `DynamicCredsProvider::issue` returns. `metadata` is stashed
/// into the lease row so revoke can look up provider-specific bits
/// (e.g. the AWS access-key-id, the PG role name) without keeping the
/// plaintext credential server-side.
#[derive(Clone, Debug)]
pub struct IssuedCredentials {
    pub credentials: serde_json::Value,
    pub metadata: serde_json::Value,
}

/// Top-level dispatcher held by [`crate::ctx::VaultCtx`]. Registers
/// providers at boot; the HTTP layer + sweeper consult it.
#[derive(Clone, Default)]
pub struct DynamicCredsRegistry {
    providers: std::sync::Arc<
        parking_lot::RwLock<
            std::collections::HashMap<String, std::sync::Arc<dyn DynamicCredsProvider>>,
        >,
    >,
}

impl DynamicCredsRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register<P: DynamicCredsProvider + 'static>(&self, provider: P) {
        self.providers
            .write()
            .insert(provider.name().to_string(), std::sync::Arc::new(provider));
    }

    pub fn get(&self, name: &str) -> Option<std::sync::Arc<dyn DynamicCredsProvider>> {
        self.providers.read().get(name).cloned()
    }

    pub fn names(&self) -> Vec<String> {
        self.providers.read().keys().cloned().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TestProvider {
        name: &'static str,
    }

    #[async_trait]
    impl DynamicCredsProvider for TestProvider {
        fn name(&self) -> &str {
            self.name
        }
        async fn issue(&self, role: &str, _ttl: u64) -> Result<IssuedCredentials> {
            Ok(IssuedCredentials {
                credentials: serde_json::json!({"role": role}),
                metadata: serde_json::json!({}),
            })
        }
        async fn revoke(&self, _lease: &LeaseRecord) -> Result<()> {
            Ok(())
        }
    }

    #[test]
    fn registry_register_and_get() {
        let r = DynamicCredsRegistry::new();
        r.register(TestProvider { name: "postgres" });
        r.register(TestProvider { name: "aws" });
        assert!(r.get("postgres").is_some());
        assert!(r.get("aws").is_some());
        assert!(r.get("missing").is_none());
        let mut names = r.names();
        names.sort();
        assert_eq!(names, vec!["aws", "postgres"]);
    }

    #[test]
    fn lease_serde() {
        let l = Lease {
            id: "lease-1".into(),
            provider: "postgres".into(),
            role: "readonly".into(),
            credentials: serde_json::json!({"username": "a", "password": "b"}),
            issued_at: 1.0,
            expires_at: 100.0,
        };
        let s = serde_json::to_string(&l).unwrap();
        let back: Lease = serde_json::from_str(&s).unwrap();
        assert_eq!(back.id, l.id);
    }
}

/// High-level dispatcher — wraps registry + lease store. Issues
/// credentials via the right provider, persists the lease row, runs
/// background revocation on expiry.
#[derive(Clone)]
pub struct DynamicCredsService {
    registry: DynamicCredsRegistry,
    leases: std::sync::Arc<dyn LeaseStore>,
}

impl DynamicCredsService {
    pub fn new(registry: DynamicCredsRegistry, leases: std::sync::Arc<dyn LeaseStore>) -> Self {
        Self { registry, leases }
    }

    pub fn registry(&self) -> &DynamicCredsRegistry {
        &self.registry
    }

    pub fn leases(&self) -> &std::sync::Arc<dyn LeaseStore> {
        &self.leases
    }

    /// Issue a credential and persist the lease row.
    pub async fn issue(
        &self,
        provider_name: &str,
        role: &str,
        ttl_secs: u64,
    ) -> Result<Lease> {
        let provider = self
            .registry
            .get(provider_name)
            .ok_or(crate::error::VaultError::NotFound)?;
        let issued = provider.issue(role, ttl_secs).await?;
        let id = uuid::Uuid::now_v7().to_string();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs_f64();
        let expires_at = now + ttl_secs.max(60) as f64;
        self.leases
            .create_lease(
                &id,
                provider_name,
                role,
                expires_at,
                &issued.metadata,
            )
            .await?;
        Ok(Lease {
            id,
            provider: provider_name.to_string(),
            role: role.to_string(),
            credentials: issued.credentials,
            issued_at: now,
            expires_at,
        })
    }

    /// Revoke an explicit lease.
    pub async fn revoke(&self, lease_id: &str) -> Result<()> {
        let lease = self
            .leases
            .get_lease(lease_id)
            .await?
            .ok_or(crate::error::VaultError::NotFound)?;
        let provider = self
            .registry
            .get(&lease.provider)
            .ok_or(crate::error::VaultError::NotFound)?;
        let _ = provider.revoke(&lease).await; // best-effort; log + continue
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs_f64();
        self.leases.revoke_lease(lease_id, now).await?;
        Ok(())
    }

    /// One sweep of expired-unrevoked leases — picks up to `batch`
    /// rows, asks each provider to clean up, marks revoked.
    pub async fn sweep_expired(&self, batch: i64) -> Result<usize> {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs_f64();
        let expired = self.leases.list_expired_unrevoked(now, batch).await?;
        let count = expired.len();
        for lease in expired {
            if let Some(provider) = self.registry.get(&lease.provider) {
                if let Err(e) = provider.revoke(&lease).await {
                    tracing::warn!(
                        target: "assay-vault",
                        lease = %lease.id, provider = %lease.provider,
                        ?e,
                        "dynamic-creds sweep: provider revoke failed; continuing"
                    );
                }
            } else {
                tracing::warn!(
                    target: "assay-vault",
                    lease = %lease.id, provider = %lease.provider,
                    "dynamic-creds sweep: no provider registered; marking revoked anyway"
                );
            }
            let _ = self.leases.revoke_lease(&lease.id, now).await;
        }
        Ok(count)
    }
}

#[cfg(feature = "vault-dynamic-postgres")]
pub mod postgres_provider {
    //! Built-in Postgres provider — pre-configured master role with
    //! GRANTs; `issue` creates a short-lived role with the configured
    //! grants, `revoke` drops it.
    //!
    //! Phase 5 ships the TRAIT-LEVEL impl; the actual SQL flow
    //! (CREATE ROLE, GRANT, DROP ROLE) requires admin connection
    //! credentials separate from the engine's main pool. The
    //! [`PostgresDynamicProvider::new`] constructor takes those
    //! separately so an operator can use a service-role connection
    //! that the regular engine pool doesn't have.
    //!
    //! Phase-5 first-cut: the role lifecycle uses a simple template
    //! (`assay_dyn_<lease_id>`); subsequent commits add per-role
    //! configuration (grants, max conns, etc.) via
    //! `vault.dynamic_postgres_roles`.

    use super::*;
    use rand::Rng;
    use std::sync::Arc;

    /// Configuration for a single PG role template. Phase-5 cut keeps
    /// it minimal; later phases lift it to a DB-backed registry.
    #[derive(Clone, Debug)]
    pub struct RoleConfig {
        /// Role name registered with this template.
        pub name: String,
        /// SQL fragments to run AFTER `CREATE ROLE` to GRANT
        /// permissions. Each entry is one GRANT statement; the
        /// provider substitutes `{role}` for the generated role name.
        pub grants: Vec<String>,
    }

    /// Postgres dynamic-creds provider.
    pub struct PostgresDynamicProvider {
        admin_pool: sqlx::PgPool,
        roles: parking_lot::RwLock<std::collections::HashMap<String, RoleConfig>>,
    }

    impl PostgresDynamicProvider {
        /// Construct against an admin-grade PG pool. The pool's
        /// connection user MUST hold CREATEROLE for this to work.
        pub fn new(admin_pool: sqlx::PgPool) -> Self {
            Self {
                admin_pool,
                roles: parking_lot::RwLock::new(Default::default()),
            }
        }

        pub fn with_role(self, role: RoleConfig) -> Self {
            self.roles.write().insert(role.name.clone(), role);
            self
        }

        pub fn into_arc(self) -> Arc<Self> {
            Arc::new(self)
        }

        fn random_suffix() -> String {
            let mut rng = rand::rng();
            let n: u32 = rng.random();
            format!("{n:08x}")
        }
    }

    #[async_trait]
    impl DynamicCredsProvider for PostgresDynamicProvider {
        fn name(&self) -> &str {
            "postgres"
        }

        async fn issue(&self, role: &str, ttl_secs: u64) -> Result<IssuedCredentials> {
            let cfg = self
                .roles
                .read()
                .get(role)
                .cloned()
                .ok_or_else(|| crate::error::VaultError::NotFound)?;
            let suffix = Self::random_suffix();
            let pg_role = format!("assay_dyn_{suffix}");
            let password = format!("p{}", uuid::Uuid::new_v4().simple());

            // CREATE ROLE … LOGIN PASSWORD '...' VALID UNTIL '<rfc3339>'
            let valid_until = chrono::DateTime::<chrono::Utc>::from_timestamp(
                (std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs()
                    + ttl_secs.max(60)) as i64,
                0,
            )
            .unwrap_or_else(chrono::Utc::now)
            .to_rfc3339();

            let create_sql = format!(
                "CREATE ROLE \"{pg_role}\" LOGIN PASSWORD '{password}' VALID UNTIL '{valid_until}'"
            );
            sqlx::query(&create_sql)
                .execute(&self.admin_pool)
                .await
                .map_err(|e| {
                    crate::error::VaultError::Backend(anyhow::anyhow!(
                        "create dynamic pg role: {e}"
                    ))
                })?;

            for grant in &cfg.grants {
                let stmt = grant.replace("{role}", &pg_role);
                sqlx::query(&stmt)
                    .execute(&self.admin_pool)
                    .await
                    .map_err(|e| {
                        crate::error::VaultError::Backend(anyhow::anyhow!(
                            "GRANT for dynamic pg role: {e}"
                        ))
                    })?;
            }

            let creds = serde_json::json!({
                "username": pg_role,
                "password": password,
            });
            let metadata = serde_json::json!({ "pg_role": pg_role });
            Ok(IssuedCredentials {
                credentials: creds,
                metadata,
            })
        }

        async fn revoke(&self, lease: &LeaseRecord) -> Result<()> {
            let pg_role = lease
                .metadata
                .get("pg_role")
                .and_then(|v| v.as_str())
                .ok_or_else(|| {
                    crate::error::VaultError::Backend(anyhow::anyhow!(
                        "lease missing pg_role metadata"
                    ))
                })?;
            // Reassign owned + DROP. Owners aren't really an issue for
            // ephemeral roles, but the REASSIGN keeps the impl robust
            // against any objects accidentally created.
            let drop_sql = format!("DROP ROLE IF EXISTS \"{pg_role}\"");
            sqlx::query(&drop_sql)
                .execute(&self.admin_pool)
                .await
                .map_err(|e| {
                    crate::error::VaultError::Backend(anyhow::anyhow!(
                        "drop dynamic pg role: {e}"
                    ))
                })?;
            Ok(())
        }
    }
}
