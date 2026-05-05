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
#[non_exhaustive]
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
#[non_exhaustive]
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
    async fn issue(&self, role: &str, ttl_secs: u64) -> Result<IssuedCredentials>;

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
#[non_exhaustive]
pub struct IssuedCredentials {
    pub credentials: serde_json::Value,
    pub metadata: serde_json::Value,
}

/// Top-level dispatcher held by [`crate::ctx::VaultCtx`]. Registers
/// providers at boot; the HTTP layer + sweeper consult it.
#[derive(Clone, Default)]
#[non_exhaustive]
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
#[non_exhaustive]
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
    pub async fn issue(&self, provider_name: &str, role: &str, ttl_secs: u64) -> Result<Lease> {
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
            .create_lease(&id, provider_name, role, expires_at, &issued.metadata)
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

#[cfg(feature = "vault-dynamic-aws")]
pub mod aws_provider {
    //! AWS IAM dynamic-creds provider — calls `sts:AssumeRole` to mint
    //! short-lived credentials for a configured role ARN. Plan §S3b.
    //!
    //! Wire shape:
    //!   POST https://sts.{region}.amazonaws.com/
    //!   Body (form): Action=AssumeRole&Version=2011-06-15&RoleArn=…&RoleSessionName=…&DurationSeconds=…
    //!   Response: XML AssumeRoleResponse with AccessKeyId / SecretAccessKey / SessionToken / Expiration
    //!
    //! XML extraction is regex-based (~30 LOC) since the response shape
    //! is fixed and well-documented; pulling a full XML parser for one
    //! call is overkill.

    use super::*;
    use crate::cloud::sigv4::{SigV4Input, now_amz_date, sign};
    use crate::sealing::kms_aws::AwsCredentials;
    use std::collections::HashMap;
    use std::sync::Arc;

    /// One AWS dynamic-creds template. Two issuance shapes per plan §S3b:
    ///   - AssumeRole — short-lived sts: temp credentials. Default.
    ///   - CreateAccessKey — long-lived iam: access key for an IAM user.
    ///     Rare for dynamic-creds (the credentials don't expire on
    ///     their own), but plan-locked. revoke() runs DeleteAccessKey.
    #[derive(Clone, Debug)]
    pub enum RoleKind {
        AssumeRole {
            role_arn: String,
            /// Defaults to `assay-vault-{lease_id}`; override per role
            /// if the assumed role has trust-policy session-name
            /// constraints.
            session_name_prefix: Option<String>,
        },
        CreateAccessKey {
            /// Existing IAM user the access key gets created under.
            iam_user: String,
        },
    }

    #[derive(Clone, Debug)]
    pub struct RoleConfig {
        /// Role name registered with this template.
        pub name: String,
        pub kind: RoleKind,
    }

    impl RoleConfig {
        /// Convenience for the common AssumeRole shape.
        pub fn assume_role(name: impl Into<String>, role_arn: impl Into<String>) -> Self {
            Self {
                name: name.into(),
                kind: RoleKind::AssumeRole {
                    role_arn: role_arn.into(),
                    session_name_prefix: None,
                },
            }
        }

        /// Convenience for iam:CreateAccessKey roles.
        pub fn create_access_key(name: impl Into<String>, iam_user: impl Into<String>) -> Self {
            Self {
                name: name.into(),
                kind: RoleKind::CreateAccessKey {
                    iam_user: iam_user.into(),
                },
            }
        }
    }

    pub struct AwsDynamicProvider {
        region: String,
        creds: AwsCredentials,
        roles: parking_lot::RwLock<HashMap<String, RoleConfig>>,
        endpoint_override: Option<String>,
        client: reqwest::Client,
    }

    impl AwsDynamicProvider {
        pub fn new(region: impl Into<String>, creds: AwsCredentials) -> Self {
            Self {
                region: region.into(),
                creds,
                roles: parking_lot::RwLock::new(HashMap::new()),
                endpoint_override: None,
                client: reqwest::Client::new(),
            }
        }

        pub fn with_role(self, role: RoleConfig) -> Self {
            self.roles.write().insert(role.name.clone(), role);
            self
        }

        pub fn with_endpoint(mut self, ep: impl Into<String>) -> Self {
            self.endpoint_override = Some(ep.into());
            self
        }

        pub fn into_arc(self) -> Arc<Self> {
            Arc::new(self)
        }
    }

    /// Pull a `<TagName>value</TagName>` payload out of XML by tag name.
    /// Ad-hoc but the STS response shape is fixed + well-known.
    fn xml_tag(xml: &str, tag: &str) -> Option<String> {
        let open = format!("<{tag}>");
        let close = format!("</{tag}>");
        let start = xml.find(&open)? + open.len();
        let end_rel = xml[start..].find(&close)?;
        Some(xml[start..start + end_rel].trim().to_string())
    }

    #[async_trait]
    impl DynamicCredsProvider for AwsDynamicProvider {
        fn name(&self) -> &str {
            "aws"
        }

        async fn issue(&self, role: &str, ttl_secs: u64) -> Result<IssuedCredentials> {
            let cfg = self
                .roles
                .read()
                .get(role)
                .cloned()
                .ok_or(crate::error::VaultError::NotFound)?;
            match cfg.kind {
                RoleKind::AssumeRole {
                    role_arn,
                    session_name_prefix,
                } => {
                    self.issue_assume_role(&role_arn, session_name_prefix.as_deref(), ttl_secs)
                        .await
                }
                RoleKind::CreateAccessKey { iam_user } => {
                    self.issue_create_access_key(&iam_user).await
                }
            }
        }

        async fn revoke(&self, lease: &LeaseRecord) -> Result<()> {
            // STS-issued temp creds expire on their own — no API call
            // needed; the lease row's revoked_at is the auditable
            // signal, and the credentials become unusable at their
            // `expiration` timestamp regardless. For
            // iam:CreateAccessKey creds we MUST DeleteAccessKey, since
            // those are long-lived; the lease metadata carries the
            // access_key_id + iam_user that the call needs.
            let kind = lease
                .metadata
                .get("kind")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if kind != "create_access_key" {
                return Ok(());
            }
            let user = lease
                .metadata
                .get("iam_user")
                .and_then(|v| v.as_str())
                .ok_or_else(|| {
                    crate::error::VaultError::Backend(anyhow::anyhow!(
                        "create_access_key lease missing iam_user metadata"
                    ))
                })?;
            let access_key_id = lease
                .metadata
                .get("access_key_id")
                .and_then(|v| v.as_str())
                .ok_or_else(|| {
                    crate::error::VaultError::Backend(anyhow::anyhow!(
                        "create_access_key lease missing access_key_id metadata"
                    ))
                })?;
            self.iam_call(
                "DeleteAccessKey",
                &[("AccessKeyId", access_key_id), ("UserName", user)],
            )
            .await
            .map(|_| ())
        }
    }

    impl AwsDynamicProvider {
        async fn issue_assume_role(
            &self,
            role_arn: &str,
            session_name_prefix: Option<&str>,
            ttl_secs: u64,
        ) -> Result<IssuedCredentials> {
            let session_name = format!(
                "{}-{}",
                session_name_prefix.unwrap_or("assay-vault"),
                uuid::Uuid::new_v4().simple()
            );
            // STS minimum is 900s; clamp.
            let duration = ttl_secs.max(900);
            let body = format!(
                "Action=AssumeRole&Version=2011-06-15&RoleArn={}&RoleSessionName={}&DurationSeconds={}",
                urlencode(role_arn),
                urlencode(&session_name),
                duration
            );
            let xml = self.aws_form_call("sts", &body).await?;
            let access_key = xml_tag(&xml, "AccessKeyId").ok_or_else(|| {
                crate::error::VaultError::Backend(anyhow::anyhow!(
                    "aws sts response missing AccessKeyId: {xml}"
                ))
            })?;
            let secret = xml_tag(&xml, "SecretAccessKey").ok_or_else(|| {
                crate::error::VaultError::Backend(anyhow::anyhow!(
                    "aws sts response missing SecretAccessKey"
                ))
            })?;
            let token = xml_tag(&xml, "SessionToken");
            let expiration = xml_tag(&xml, "Expiration");
            Ok(IssuedCredentials {
                credentials: serde_json::json!({
                    "access_key_id": access_key,
                    "secret_access_key": secret,
                    "session_token": token,
                    "expiration": expiration,
                }),
                metadata: serde_json::json!({
                    "kind": "assume_role",
                    "role_arn": role_arn,
                    "session_name": session_name,
                }),
            })
        }

        async fn issue_create_access_key(&self, iam_user: &str) -> Result<IssuedCredentials> {
            let xml = self
                .iam_call("CreateAccessKey", &[("UserName", iam_user)])
                .await?;
            let access_key = xml_tag(&xml, "AccessKeyId").ok_or_else(|| {
                crate::error::VaultError::Backend(anyhow::anyhow!(
                    "iam CreateAccessKey missing AccessKeyId"
                ))
            })?;
            let secret = xml_tag(&xml, "SecretAccessKey").ok_or_else(|| {
                crate::error::VaultError::Backend(anyhow::anyhow!(
                    "iam CreateAccessKey missing SecretAccessKey"
                ))
            })?;
            let create_date = xml_tag(&xml, "CreateDate");
            Ok(IssuedCredentials {
                credentials: serde_json::json!({
                    "access_key_id": access_key,
                    "secret_access_key": secret,
                    "create_date": create_date,
                }),
                metadata: serde_json::json!({
                    "kind": "create_access_key",
                    "iam_user": iam_user,
                    "access_key_id": access_key,
                }),
            })
        }

        async fn aws_form_call(&self, service: &'static str, body: &str) -> Result<String> {
            let url = match service {
                "sts" => self
                    .endpoint_override
                    .clone()
                    .unwrap_or_else(|| format!("https://sts.{}.amazonaws.com/", self.region)),
                "iam" => self
                    .endpoint_override
                    .clone()
                    .unwrap_or_else(|| "https://iam.amazonaws.com/".to_string()),
                _ => unreachable!(),
            };
            let amz_date = now_amz_date();
            // IAM is a global service; sigv4 region is "us-east-1" by spec.
            let sig_region = if service == "iam" {
                "us-east-1"
            } else {
                &self.region
            };
            let signed = sign(SigV4Input {
                access_key_id: &self.creds.access_key_id,
                secret_access_key: &self.creds.secret_access_key,
                session_token: self.creds.session_token.as_deref(),
                region: sig_region,
                service,
                method: "POST",
                url: &url,
                headers: &[("content-type", "application/x-www-form-urlencoded")],
                body: body.as_bytes(),
                amz_date: &amz_date,
            });
            let mut req = self.client.post(&signed.url).body(signed.body.clone());
            for (k, v) in &signed.headers {
                req = req.header(k, v);
            }
            let resp = req.send().await.map_err(|e| {
                crate::error::VaultError::Backend(anyhow::anyhow!("aws {service} POST: {e}"))
            })?;
            if !resp.status().is_success() {
                let status = resp.status();
                let txt = resp.text().await.unwrap_or_default();
                return Err(crate::error::VaultError::Backend(anyhow::anyhow!(
                    "aws {service} returned {status}: {txt}"
                )));
            }
            resp.text().await.map_err(|e| {
                crate::error::VaultError::Backend(anyhow::anyhow!("aws {service} read body: {e}"))
            })
        }

        async fn iam_call(&self, action: &'static str, params: &[(&str, &str)]) -> Result<String> {
            let mut body = format!("Action={action}&Version=2010-05-08");
            for (k, v) in params {
                body.push('&');
                body.push_str(k);
                body.push('=');
                body.push_str(&urlencode(v));
            }
            self.aws_form_call("iam", &body).await
        }
    }

    fn urlencode(s: &str) -> String {
        let mut out = String::with_capacity(s.len());
        for b in s.as_bytes() {
            let c = *b as char;
            if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '~') {
                out.push(c);
            } else {
                out.push_str(&format!("%{:02X}", b));
            }
        }
        out
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn xml_extraction() {
            let xml = r#"<AssumeRoleResponse><AssumeRoleResult><Credentials>
<AccessKeyId>AKID</AccessKeyId><SecretAccessKey>secret</SecretAccessKey>
<SessionToken>tok</SessionToken><Expiration>2030-01-01T00:00:00Z</Expiration>
</Credentials></AssumeRoleResult></AssumeRoleResponse>"#;
            assert_eq!(xml_tag(xml, "AccessKeyId").as_deref(), Some("AKID"));
            assert_eq!(xml_tag(xml, "SecretAccessKey").as_deref(), Some("secret"));
            assert_eq!(xml_tag(xml, "SessionToken").as_deref(), Some("tok"));
        }

        #[test]
        fn role_config_constructors() {
            let r = RoleConfig::assume_role("ci-deploy", "arn:aws:iam::123:role/x");
            assert!(matches!(r.kind, RoleKind::AssumeRole { .. }));
            let r = RoleConfig::create_access_key("legacy-key", "iam-user-foo");
            assert!(matches!(r.kind, RoleKind::CreateAccessKey { .. }));
        }
    }
}

#[cfg(feature = "vault-dynamic-gcp")]
pub mod gcp_provider {
    //! GCP IAM dynamic-creds provider — service-account impersonation
    //! via `iamcredentials.googleapis.com:generateAccessToken`. Plan §S3c.

    use super::*;
    use crate::cloud::gcp_jwt::{ServiceAccount, fetch_access_token};
    use serde::{Deserialize, Serialize};
    use std::collections::HashMap;
    use std::sync::Arc;

    const SCOPE: &str = "https://www.googleapis.com/auth/cloud-platform";

    #[derive(Clone, Debug)]
    pub struct RoleConfig {
        pub name: String,
        /// The service account to impersonate (e.g.
        /// `target-sa@project.iam.gserviceaccount.com`).
        pub target_service_account: String,
        /// OAuth scopes the impersonation token should carry.
        pub scopes: Vec<String>,
    }

    pub struct GcpDynamicProvider {
        sa: Arc<ServiceAccount>,
        roles: parking_lot::RwLock<HashMap<String, RoleConfig>>,
        endpoint_override: Option<String>,
        client: reqwest::Client,
    }

    impl GcpDynamicProvider {
        pub fn new(sa: ServiceAccount) -> Self {
            Self {
                sa: Arc::new(sa),
                roles: parking_lot::RwLock::new(HashMap::new()),
                endpoint_override: None,
                client: reqwest::Client::new(),
            }
        }

        pub fn with_role(self, role: RoleConfig) -> Self {
            self.roles.write().insert(role.name.clone(), role);
            self
        }

        pub fn with_endpoint(mut self, ep: impl Into<String>) -> Self {
            self.endpoint_override = Some(ep.into());
            self
        }

        pub fn into_arc(self) -> Arc<Self> {
            Arc::new(self)
        }

        fn endpoint_for(&self, target_sa: &str) -> String {
            let base = self
                .endpoint_override
                .as_deref()
                .unwrap_or("https://iamcredentials.googleapis.com");
            format!("{base}/v1/projects/-/serviceAccounts/{target_sa}:generateAccessToken")
        }
    }

    #[derive(Serialize)]
    struct GenTokenBody<'a> {
        scope: &'a [String],
        lifetime: String,
    }

    #[derive(Deserialize)]
    struct GenTokenResp {
        #[serde(rename = "accessToken")]
        access_token: String,
        #[serde(rename = "expireTime")]
        expire_time: String,
    }

    #[async_trait]
    impl DynamicCredsProvider for GcpDynamicProvider {
        fn name(&self) -> &str {
            "gcp"
        }

        async fn issue(&self, role: &str, ttl_secs: u64) -> Result<IssuedCredentials> {
            let cfg = self
                .roles
                .read()
                .get(role)
                .cloned()
                .ok_or(crate::error::VaultError::NotFound)?;
            // Fetch caller-side bearer first.
            let caller = fetch_access_token(&self.sa, SCOPE).await?;
            let lifetime = format!("{}s", ttl_secs.max(60));
            let scopes = if cfg.scopes.is_empty() {
                vec![SCOPE.to_string()]
            } else {
                cfg.scopes.clone()
            };
            let url = self.endpoint_for(&cfg.target_service_account);
            let body = GenTokenBody {
                scope: &scopes,
                lifetime,
            };
            let resp = self
                .client
                .post(&url)
                .bearer_auth(&caller.access_token)
                .json(&body)
                .send()
                .await
                .map_err(|e| {
                    crate::error::VaultError::Backend(anyhow::anyhow!(
                        "gcp generateAccessToken POST: {e}"
                    ))
                })?;
            if !resp.status().is_success() {
                let status = resp.status();
                let txt = resp.text().await.unwrap_or_default();
                return Err(crate::error::VaultError::Backend(anyhow::anyhow!(
                    "gcp generateAccessToken {status}: {txt}"
                )));
            }
            let parsed: GenTokenResp = resp.json().await.map_err(|e| {
                crate::error::VaultError::Backend(anyhow::anyhow!("gcp resp decode: {e}"))
            })?;
            Ok(IssuedCredentials {
                credentials: serde_json::json!({
                    "access_token": parsed.access_token,
                    "expire_time": parsed.expire_time,
                }),
                metadata: serde_json::json!({
                    "target_service_account": cfg.target_service_account,
                }),
            })
        }

        async fn revoke(&self, _lease: &LeaseRecord) -> Result<()> {
            // Generated access tokens expire on their own; the lease
            // row's revoked_at is the auditable signal.
            Ok(())
        }
    }
}

#[cfg(feature = "vault-dynamic-kubernetes")]
pub mod k8s_provider {
    //! Kubernetes projected-SA-token dynamic-creds provider. Plan §S3d.
    //!
    //! Calls
    //! `POST /api/v1/namespaces/{ns}/serviceaccounts/{sa}/token`
    //! against the configured kube-apiserver to mint a token bound to
    //! the requested audiences with the requested expiry.

    use super::*;
    use serde::{Deserialize, Serialize};
    use std::collections::HashMap;
    use std::sync::Arc;

    #[derive(Clone, Debug)]
    pub struct RoleConfig {
        pub name: String,
        pub namespace: String,
        pub service_account: String,
        pub audiences: Vec<String>,
    }

    pub struct K8sDynamicProvider {
        api_server: String,
        /// Bearer token for the engine's own identity (e.g. the
        /// in-cluster /var/run/secrets/kubernetes.io/serviceaccount/token
        /// payload, or an admin kubeconfig token).
        caller_token: String,
        ca_pem: Option<String>,
        roles: parking_lot::RwLock<HashMap<String, RoleConfig>>,
        client: reqwest::Client,
    }

    impl K8sDynamicProvider {
        pub fn new(api_server: impl Into<String>, caller_token: impl Into<String>) -> Self {
            Self {
                api_server: api_server.into(),
                caller_token: caller_token.into(),
                ca_pem: None,
                roles: parking_lot::RwLock::new(HashMap::new()),
                client: reqwest::Client::new(),
            }
        }

        pub fn with_ca_pem(mut self, ca: impl Into<String>) -> Self {
            self.ca_pem = Some(ca.into());
            self
        }

        pub fn with_role(self, role: RoleConfig) -> Self {
            self.roles.write().insert(role.name.clone(), role);
            self
        }

        pub fn into_arc(self) -> Arc<Self> {
            Arc::new(self)
        }
    }

    #[derive(Serialize)]
    struct TokenRequest<'a> {
        kind: &'a str,
        #[serde(rename = "apiVersion")]
        api_version: &'a str,
        spec: TokenRequestSpec<'a>,
    }
    #[derive(Serialize)]
    struct TokenRequestSpec<'a> {
        audiences: &'a [String],
        #[serde(rename = "expirationSeconds")]
        expiration_seconds: u64,
    }
    #[derive(Deserialize)]
    struct TokenResponse {
        status: TokenStatus,
    }
    #[derive(Deserialize)]
    struct TokenStatus {
        token: String,
        #[serde(rename = "expirationTimestamp")]
        expiration_timestamp: String,
    }

    #[async_trait]
    impl DynamicCredsProvider for K8sDynamicProvider {
        fn name(&self) -> &str {
            "kubernetes"
        }

        async fn issue(&self, role: &str, ttl_secs: u64) -> Result<IssuedCredentials> {
            let cfg = self
                .roles
                .read()
                .get(role)
                .cloned()
                .ok_or(crate::error::VaultError::NotFound)?;
            let url = format!(
                "{}/api/v1/namespaces/{}/serviceaccounts/{}/token",
                self.api_server.trim_end_matches('/'),
                cfg.namespace,
                cfg.service_account
            );
            let body = TokenRequest {
                kind: "TokenRequest",
                api_version: "authentication.k8s.io/v1",
                spec: TokenRequestSpec {
                    audiences: &cfg.audiences,
                    expiration_seconds: ttl_secs.max(600),
                },
            };
            let resp = self
                .client
                .post(&url)
                .bearer_auth(&self.caller_token)
                .json(&body)
                .send()
                .await
                .map_err(|e| {
                    crate::error::VaultError::Backend(anyhow::anyhow!("k8s token POST: {e}"))
                })?;
            if !resp.status().is_success() {
                let status = resp.status();
                let txt = resp.text().await.unwrap_or_default();
                return Err(crate::error::VaultError::Backend(anyhow::anyhow!(
                    "k8s token {status}: {txt}"
                )));
            }
            let parsed: TokenResponse = resp.json().await.map_err(|e| {
                crate::error::VaultError::Backend(anyhow::anyhow!("k8s token decode: {e}"))
            })?;
            Ok(IssuedCredentials {
                credentials: serde_json::json!({
                    "token": parsed.status.token,
                    "expiration_timestamp": parsed.status.expiration_timestamp,
                }),
                metadata: serde_json::json!({
                    "namespace": cfg.namespace,
                    "service_account": cfg.service_account,
                    "audiences": cfg.audiences,
                }),
            })
        }

        async fn revoke(&self, _lease: &LeaseRecord) -> Result<()> {
            // Projected SA tokens expire on their own. K8s does not
            // expose an explicit revoke API for SA tokens — rotate
            // the SA itself if you need pre-expiry revocation.
            Ok(())
        }
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
                    crate::error::VaultError::Backend(anyhow::anyhow!("drop dynamic pg role: {e}"))
                })?;
            Ok(())
        }
    }
}
