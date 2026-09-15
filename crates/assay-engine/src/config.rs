//! Engine configuration loaded from TOML.
//!
//! Phase 8 wires in `AuthConfig` so the engine binary can compose an
//! `assay_auth::AuthCtx` per-deployment (issuer, OIDC provider toggle,
//! session/cookie shape). When `auth` isn't compiled in (Cargo feature
//! off) the auth section is parsed but never read — keeping the TOML
//! shape stable across feature configurations.
//!
//! Env-var substitution: `${VAR}` and `${VAR:-default}` references in
//! the TOML are expanded against the process environment before parsing
//! (added in 0.3.1). This keeps secrets out of config files when the
//! engine runs under K8s/systemd/etc. — the typical pattern is
//! `url = "${DATABASE_URL}"` with `DATABASE_URL` injected from a
//! Secret/EnvironmentFile.

use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[non_exhaustive]
pub struct EngineConfig {
    pub server: ServerConfig,
    pub backend: BackendConfig,
    #[serde(default)]
    pub workflow: WorkflowConfig,
    #[serde(default)]
    pub auth: AuthConfig,
    #[serde(default)]
    pub vault: VaultConfig,
    #[serde(default)]
    pub dashboard: DashboardConfig,
    #[serde(default)]
    pub logging: LoggingConfig,
    /// TTL in seconds for the engine_events outbox. Rows older than this
    /// are pruned hourly by the cleanup loop. Default 3 days.
    #[serde(default = "default_engine_events_ttl_secs")]
    pub engine_events_ttl_secs: u64,
    /// Modules to flip from `enabled = FALSE` to `enabled = TRUE` on
    /// first boot when they're compiled in. Empty by default — operators
    /// of existing v0.1.2 deployments shouldn't get unexpected auth
    /// migrations on upgrade. Local-dev convenience: set to
    /// `["auth"]` in `engine.local.toml` to flip auth on without an
    /// extra step.
    #[serde(default)]
    pub auto_enable_modules: Vec<String>,
}

fn default_engine_events_ttl_secs() -> u64 {
    3 * 86_400
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[non_exhaustive]
pub struct ServerConfig {
    #[serde(default = "default_bind_addr")]
    pub bind_addr: String,
    /// Operator-supplied canonical URL for the engine API and dashboard.
    /// Auth defaults to this origin too, but can use a dedicated hostname
    /// through `auth.public_url`. Defaults to the bind address over plain
    /// HTTP for local development; production deployments MUST override it
    /// with the public HTTPS URL.
    #[serde(default = "default_public_url")]
    pub public_url: String,
    /// Host header values accepted by the public server. Empty keeps the
    /// embedded/local-development behavior and accepts every host. Health
    /// checks remain reachable when this allowlist is populated.
    #[serde(default)]
    pub allowed_hosts: Vec<String>,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            bind_addr: default_bind_addr(),
            public_url: default_public_url(),
            allowed_hosts: Vec::new(),
        }
    }
}

fn default_bind_addr() -> String {
    "0.0.0.0:3000".to_string()
}

fn default_public_url() -> String {
    "http://localhost:3000".to_string()
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "lowercase")]
#[non_exhaustive]
pub enum BackendConfig {
    Postgres {
        /// Postgres connection URL, e.g. `postgres://user:pass@host:5432/db`.
        /// PostgreSQL 18 is the minimum supported version.
        url: String,
    },
    Sqlite {
        /// Directory holding the per-module SQLite files
        /// (`<data_dir>/engine.db`, `<data_dir>/workflow.db`, …). Created
        /// on startup if missing. Defaults to `./data`. Use `:memory:`
        /// in `path` (legacy) or set `data_dir = ":memory:"` to keep the
        /// engine purely in-memory for tests.
        #[serde(default = "default_data_dir")]
        data_dir: String,
        /// Legacy single-file SQLite path. Deprecated in v0.1.2 — when
        /// set, the engine logs a deprecation notice and treats it as
        /// `data_dir = parent(path)` so existing configs keep working
        /// during the transition.
        #[serde(default)]
        path: Option<String>,
    },
}

fn default_data_dir() -> String {
    "./data".to_string()
}

impl BackendConfig {
    /// Resolve the effective data directory for SQLite. PG returns `None`.
    pub fn sqlite_data_dir(&self) -> Option<String> {
        match self {
            Self::Sqlite { data_dir, path } => {
                // Legacy `path` wins for backwards compat — treat the
                // parent dir as the new data_dir so existing v0.1.1
                // configs migrate without surprise.
                if let Some(p) = path {
                    let parent = std::path::Path::new(p)
                        .parent()
                        .map(|p| p.display().to_string())
                        .filter(|s| !s.is_empty());
                    Some(parent.unwrap_or_else(|| data_dir.clone()))
                } else {
                    Some(data_dir.clone())
                }
            }
            Self::Postgres { .. } => None,
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[non_exhaustive]
pub struct WorkflowConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
}

/// Auth-module deployment shape. Read by the engine binary when the
/// `auth` Cargo feature is compiled in AND `engine.modules.auth.enabled`
/// is TRUE; otherwise the defaults are harmless.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[non_exhaustive]
pub struct AuthConfig {
    /// Canonical browser-facing origin for the auth surface. Defaults to
    /// `server.public_url` when unset. This allows one engine deployment to
    /// expose auth on a dedicated hostname without changing its API origin.
    pub public_url: Option<String>,
    /// JWT issuer + OIDC `iss` claim. Defaults to
    /// `<auth.public_url>/auth` when unset, which matches the route
    /// mount point.
    pub issuer: Option<String>,
    /// JWT audience list — also used by the OIDC provider when minting
    /// access_tokens for resource servers. Defaults to `[issuer]`.
    #[serde(default)]
    pub audience: Vec<String>,
    #[serde(default)]
    pub session: AuthSessionConfig,
    #[serde(default)]
    pub passkey: AuthPasskeyConfig,
    #[serde(default)]
    pub recovery: AuthRecoveryConfig,
    #[serde(default)]
    pub oidc_provider: AuthOidcProviderConfig,
    /// Admin API keys — comma-separated bearer tokens that grant access
    /// to `/admin/*` routes. Operators rotate these via the engine
    /// config. Per-token, no expiry; for fancier admin auth (Zanzibar
    /// roles, session-based admin) see plan 12c § 6.7. Empty list locks
    /// admin routes entirely (404 → 401).
    #[serde(default)]
    pub admin_api_keys: Vec<String>,
    /// External OIDC issuers trusted to mint JWTs the engine accepts
    /// pass-through (v0.3.2). Each entry's JWKS is discovered via
    /// `<issuer_url>/.well-known/openid-configuration` at boot and
    /// refreshed periodically thereafter. Tokens whose `iss` claim
    /// matches a configured issuer are verified against that issuer's
    /// keys; everything else falls through to the engine's internal
    /// JWT path. When this list is non-empty, the engine boots without
    /// requiring operator users / `admin_api_keys` — the upstream IdP
    /// is the source of truth for identity.
    ///
    /// Mirrors the v0.12.1 `--auth-issuer` / `--auth-audience` CLI
    /// flags in the new TOML config shape. Multiple issuers are allowed
    /// for deployments that span more than one IdP.
    ///
    /// Field is private so future entries (per-issuer policy, claim
    /// mappers, etc.) can be added without breaking downstream
    /// construction. Read via [`AuthConfig::external_issuers`].
    #[serde(default)]
    external_issuers: Vec<ExternalIssuerConfig>,
}

impl AuthConfig {
    /// Read access to the parsed `[[auth.external_issuers]]` blocks.
    pub fn external_issuers(&self) -> &[ExternalIssuerConfig] {
        &self.external_issuers
    }
}

/// One trusted external OIDC issuer for pass-through JWT validation.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[non_exhaustive]
pub struct ExternalIssuerConfig {
    /// Issuer URL — the value the JWT's `iss` claim is matched against
    /// and the base for `<issuer_url>/.well-known/openid-configuration`
    /// discovery. Trailing slashes are normalized.
    pub issuer_url: String,
    /// Accepted `aud` claim values. A token whose `aud` isn't in this
    /// list is rejected. Empty list = audience check disabled (NOT
    /// recommended; set explicitly per deployment).
    #[serde(default)]
    pub audience: Vec<String>,
    /// JWKS refresh interval in seconds (background task). Default 3600
    /// (1 hour). Minimum effective value 60 seconds — anything smaller
    /// is clamped to avoid hammering the upstream's JWKS endpoint.
    #[serde(default = "default_jwks_refresh_secs")]
    pub jwks_refresh_secs: u64,
}

fn default_jwks_refresh_secs() -> u64 {
    3600
}

/// Session module knobs.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[non_exhaustive]
pub struct AuthSessionConfig {
    /// Default session lifetime in seconds. `None` ⇒ uses the
    /// `assay_auth::session::DEFAULT_SESSION_DURATION` (30 days).
    pub ttl_seconds: Option<u64>,
}

/// WebAuthn / passkey module knobs.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[non_exhaustive]
pub struct AuthPasskeyConfig {
    /// Relying-party id — the host (no scheme/port) the browser will
    /// scope passkeys to. Defaults to the host of `auth.public_url`, or
    /// `server.public_url` when no dedicated auth origin is configured.
    pub rp_id: Option<String>,
    /// Human-readable label browsers show. Defaults to `"Assay"`.
    pub rp_name: Option<String>,
}

/// Self-service password-recovery deployment knobs.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[non_exhaustive]
pub struct AuthRecoveryConfig {
    /// Mount the public recovery endpoints and send reset emails.
    #[serde(default)]
    pub enabled: bool,
    /// Lifetime of a single-use recovery token. Defaults to 15 minutes.
    #[serde(default = "default_recovery_token_ttl_seconds")]
    pub token_ttl_seconds: u64,
    /// Minimum time between recovery emails for one account.
    #[serde(default = "default_recovery_cooldown_seconds")]
    pub request_cooldown_seconds: u64,
    /// SMTP delivery settings. Required when recovery is enabled.
    pub smtp: Option<AuthSmtpConfig>,
}

impl Default for AuthRecoveryConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            token_ttl_seconds: default_recovery_token_ttl_seconds(),
            request_cooldown_seconds: default_recovery_cooldown_seconds(),
            smtp: None,
        }
    }
}

fn default_recovery_token_ttl_seconds() -> u64 {
    15 * 60
}

fn default_recovery_cooldown_seconds() -> u64 {
    60
}

/// SMTP settings used only by password recovery.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[non_exhaustive]
pub struct AuthSmtpConfig {
    pub host: String,
    #[serde(default = "default_smtp_port")]
    pub port: u16,
    pub username: String,
    pub password: String,
    pub from: String,
    #[serde(default = "default_true")]
    pub starttls: bool,
}

fn default_smtp_port() -> u16 {
    587
}

/// OIDC provider knobs.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[non_exhaustive]
pub struct AuthOidcProviderConfig {
    /// Whether the OIDC provider routes (/authorize /token /userinfo …)
    /// are mounted. Defaults to `true` when the Cargo feature is on.
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Override the issuer URL used by the OIDC provider. Defaults to
    /// the parent [`AuthConfig::issuer`] when unset.
    pub issuer_override: Option<String>,
    /// `true`  → federation callback creates an `auth.users` row on
    ///           first sign-in for a new upstream identity (open
    ///           signup; legacy library default — kept as the default
    ///           here so omitted config does not silently flip
    ///           existing deployments into invite-only).
    /// `false` → callback looks up by email (and requires the
    ///           upstream `email_verified` claim); missing rows
    ///           return 403. Operators pre-populate `auth.users` via
    ///           the admin API or the sysops `/auth/users` page.
    ///           Recommended for shared / multi-tenant deployments —
    ///           must be set explicitly.
    #[serde(default = "default_true")]
    pub auto_provision: bool,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[non_exhaustive]
pub struct VaultConfig {
    #[serde(default)]
    pub hashicorp_compat: HashicorpCompatConfig,
    #[serde(default)]
    pub sealing: VaultSealingConfig,
}

/// Where the vault's unseal material comes from, and what this
/// deployment has accepted in its absence.
///
/// Omitting the section entirely means "read `ASSAY_VAULT_SEAL_KEY`",
/// which is what the engine has always done. What changed is the other
/// half: a vault-enabled engine that finds no material now refuses to
/// start instead of writing its master key to the store in the clear.
///
/// Every field here is deliberately config-only. A secret in the process
/// environment is readable from `/proc/<pid>/environ`, is carried into
/// core dumps and shows up in `systemctl show -p Environment`, so this
/// section exists to give a deployment somewhere else to put one — and
/// the two `allow_*` escapes have no environment variable at all, so
/// that nobody turns sealing off by copying a line into a deployment
/// template. (String fields still take `${VAR}` like every other config
/// string: substitution is a textual pass over the whole file, so the
/// escapes are not literally env-proof — they just have no shortcut of
/// their own.)
///
/// `value` and `passphrase` hold the real secret once `${VAR}` has been
/// expanded, so neither `Debug` nor `Serialize` is derived over them —
/// both report only whether the field is set.
#[derive(Clone, Deserialize, Serialize)]
#[non_exhaustive]
pub struct VaultSealingConfig {
    /// `"env"` (default), `"file"`, `"value"` or `"passphrase"`.
    #[serde(default = "default_seal_source")]
    pub source: String,
    /// `source = "env"`: which variable to read. Named rather than fixed
    /// so a deployment whose secret injector has its own conventions
    /// does not have to rename the secret to suit us.
    #[serde(default = "default_seal_env_var")]
    pub var: String,
    /// `source = "file"`: the path to read. Refused unless only the
    /// engine's own account can read it.
    pub path: Option<String>,
    /// `source = "value"`: the key inline — in practice a `${VAR}`
    /// reference. Any string, base64 included; it is hashed exactly as
    /// the environment variable is, so the same value means the same
    /// key whichever way it arrives.
    ///
    /// Serializes as its presence, never its value: this struct is
    /// returned by the engine config endpoint.
    #[serde(serialize_with = "serialize_presence_only")]
    pub value: Option<String>,
    /// `source = "passphrase"`: something a human types, run through
    /// Argon2id rather than a plain hash. Needs the
    /// `vault-sealing-passphrase` build feature.
    ///
    /// Serializes as its presence, never its value: this struct is
    /// returned by the engine config endpoint.
    #[serde(serialize_with = "serialize_presence_only")]
    pub passphrase: Option<String>,
    /// `source = "passphrase"`: required, and public. The derived key
    /// has to survive a restart, so the salt cannot be random per-boot;
    /// its job is separation between deployments, not secrecy.
    pub salt: Option<String>,
    /// Permit booting with the master key stored in the clear.
    ///
    /// **This removes the protection the vault's sealing exists to give:
    /// a database dump becomes a copy of every secret in it.** It is for
    /// local development. Every boot that uses it logs at ERROR. It
    /// permits *minting* a key in the clear and never opens a store that
    /// is already sealed.
    #[serde(default)]
    pub allow_plaintext_kek: bool,
    /// Permit re-sealing a store that currently holds a plaintext key.
    ///
    /// **The rewrite is one-way.** Afterwards the master key exists only
    /// under your unseal material, and losing that material loses every
    /// secret the vault wraps — there is no plaintext copy left to fall
    /// back on. Back up `vault.kek_metadata` before setting this.
    #[serde(default)]
    pub allow_plaintext_migration: bool,
}

impl Default for VaultSealingConfig {
    fn default() -> Self {
        // Spelled out rather than derived: `String::default()` is empty,
        // and an omitted `[vault.sealing]` has to mean "read the
        // variable the engine has always read", not "read the variable
        // named nothing". Same reasoning as HashicorpCompatConfig above.
        Self {
            source: default_seal_source(),
            var: default_seal_env_var(),
            path: None,
            value: None,
            passphrase: None,
            salt: None,
            allow_plaintext_kek: false,
            allow_plaintext_migration: false,
        }
    }
}

fn default_seal_source() -> String {
    "env".to_string()
}

fn default_seal_env_var() -> String {
    "ASSAY_VAULT_SEAL_KEY".to_string()
}

/// What a redacted field reads as. Matches the engine API's own
/// placeholder so one response does not use two spellings.
const REDACTED: &str = "[REDACTED]";

/// Serialize an optional secret as its presence, never its value.
///
/// `GET /api/v1/engine/core/config` serializes [`EngineConfig`] whole,
/// and `${VAR}` references are expanded before the TOML is parsed — so
/// by the time anything serializes this struct it holds the real seal
/// key, not a reference to one. Redacting at the field rather than in
/// that one handler's key-name filter means a future caller that
/// serializes the config cannot reintroduce the leak, and it keeps the
/// useful half: an operator can still see *whether* a source is
/// configured.
fn serialize_presence_only<S>(value: &Option<String>, serializer: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    match value {
        Some(_) => serializer.serialize_some(REDACTED),
        None => serializer.serialize_none(),
    }
}

/// Hand-written for the same reason the vault crate hand-writes `Debug`
/// for `SealSource` and `SealKey`: a derive here would print the seal
/// key into any log line that renders the config.
impl std::fmt::Debug for VaultSealingConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let redact = |v: &Option<String>| v.as_ref().map(|_| REDACTED);
        f.debug_struct("VaultSealingConfig")
            .field("source", &self.source)
            .field("var", &self.var)
            .field("path", &self.path)
            .field("value", &redact(&self.value))
            .field("passphrase", &redact(&self.passphrase))
            // The salt is public by design — it separates deployments
            // rather than hiding anything, and an operator needs to be
            // able to read back the one their store was sealed with.
            .field("salt", &self.salt)
            .field("allow_plaintext_kek", &self.allow_plaintext_kek)
            .field("allow_plaintext_migration", &self.allow_plaintext_migration)
            .finish()
    }
}

/// Vault / OpenBao KV2 read facade at `/v1/*`. Off unless an operator asks
/// for it: serving a second dialect of the secret store at the engine root is
/// a deliberate act, not a default.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[non_exhaustive]
pub struct HashicorpCompatConfig {
    #[serde(default)]
    pub enabled: bool,
    /// Set this to the mount the estate's OpenBao used and consumers keep
    /// their existing paths.
    #[serde(default = "default_vault_compat_mount")]
    pub mount: String,
}

impl Default for HashicorpCompatConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            mount: default_vault_compat_mount(),
        }
    }
}

fn default_vault_compat_mount() -> String {
    "secrets".to_string()
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[non_exhaustive]
pub struct DashboardConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Operator consoles (`/workflow`, `/engine`, `/vault`, and
    /// `/auth/console`). Defaults to the legacy `enabled` value.
    pub operator_enabled: Option<bool>,
    /// Public browser authentication UI (`/auth/login`, recovery, and the
    /// auth landing page). Defaults to the legacy `enabled` value.
    pub auth_ui_enabled: Option<bool>,
}

impl Default for DashboardConfig {
    fn default() -> Self {
        // When the `[dashboard]` section is omitted entirely from
        // engine.toml, serde calls Default::default() — and bool's
        // derived default is `false`. We want `enabled: true` here so
        // a fresh engine.toml without a [dashboard] section still
        // mounts the SPAs out of the box.
        Self {
            enabled: true,
            operator_enabled: None,
            auth_ui_enabled: None,
        }
    }
}

impl DashboardConfig {
    pub fn operator_enabled(&self) -> bool {
        self.operator_enabled.unwrap_or(self.enabled)
    }

    pub fn auth_ui_enabled(&self) -> bool {
        self.auth_ui_enabled.unwrap_or(self.enabled)
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[non_exhaustive]
pub struct LoggingConfig {
    #[serde(default = "default_log_level")]
    pub level: String,
    #[serde(default = "default_log_format")]
    pub format: String,
}

impl Default for LoggingConfig {
    fn default() -> Self {
        Self {
            level: default_log_level(),
            format: default_log_format(),
        }
    }
}

fn default_true() -> bool {
    true
}

fn default_log_level() -> String {
    "info".to_string()
}

fn default_log_format() -> String {
    "pretty".to_string()
}

impl EngineConfig {
    /// Load `engine.toml`. String fields support `${VAR}` and
    /// `${VAR:-default}` env-var references; references with no default
    /// error out at load time when the variable is unset. Bracket-less
    /// `$VAR` is left untouched, and `${...}` whose contents aren't a
    /// valid identifier are passed through verbatim.
    pub fn from_file(path: &Path) -> anyhow::Result<Self> {
        let raw = std::fs::read_to_string(path)
            .map_err(|e| anyhow::anyhow!("read config {}: {e}", path.display()))?;
        let expanded = expand_env_vars(&raw, |name| std::env::var(name).ok())
            .map_err(|e| anyhow::anyhow!("expand env vars in {}: {e}", path.display()))?;
        let cfg: Self = toml::from_str(&expanded)
            .map_err(|e| anyhow::anyhow!("parse config {}: {e}", path.display()))?;
        Ok(cfg)
    }
}

/// Expand `${VAR}` and `${VAR:-default}` references in `raw` using
/// `lookup` to resolve names. The lookup-by-closure shape keeps this
/// pure for unit tests (the binary path uses `std::env::var`).
///
/// Behavior:
/// - `${VAR}` → value if set, error if unset.
/// - `${VAR:-default}` → value if set, else the default (which may be empty).
/// - Bracket-less `$VAR` is untouched.
/// - `${...}` whose contents aren't a valid identifier are passed
///   through verbatim — keeps non-substitution `${...}` literals usable
///   in odd field values without false positives.
fn expand_env_vars<F>(raw: &str, lookup: F) -> anyhow::Result<String>
where
    F: Fn(&str) -> Option<String>,
{
    let mut out = String::with_capacity(raw.len());
    let mut rest = raw;
    while let Some(idx) = rest.find("${") {
        out.push_str(&rest[..idx]);
        let after_open = &rest[idx + 2..];
        let close_idx = after_open
            .find('}')
            .ok_or_else(|| anyhow::anyhow!("unclosed `${{` in config"))?;
        let inner = &after_open[..close_idx];
        let (var_name, default) = match inner.split_once(":-") {
            Some((n, d)) => (n, Some(d)),
            None => (inner, None),
        };
        if !is_valid_var_name(var_name) {
            // Not a valid identifier — pass the whole `${...}` through.
            out.push_str("${");
            out.push_str(inner);
            out.push('}');
        } else {
            match lookup(var_name) {
                Some(val) => out.push_str(&val),
                None => match default {
                    Some(def) => out.push_str(def),
                    None => {
                        return Err(anyhow::anyhow!(
                            "env var `{}` is not set and has no default",
                            var_name
                        ));
                    }
                },
            }
        }
        rest = &after_open[close_idx + 1..];
    }
    out.push_str(rest);
    Ok(out)
}

fn is_valid_var_name(s: &str) -> bool {
    let mut chars = s.chars();
    match chars.next() {
        Some(c) if c == '_' || c.is_ascii_alphabetic() => {}
        _ => return false,
    }
    chars.all(|c| c == '_' || c.is_ascii_alphanumeric())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lookup_from<'a>(map: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<String> + 'a {
        move |name: &str| {
            map.iter()
                .find(|(k, _)| *k == name)
                .map(|(_, v)| (*v).to_string())
        }
    }

    #[test]
    fn no_substitution_passes_through() {
        let s = "plain string with $literal but no expansion markers";
        assert_eq!(expand_env_vars(s, lookup_from(&[])).unwrap(), s);
    }

    #[test]
    fn substitutes_set_var() {
        let out = expand_env_vars("value=${FOO}", lookup_from(&[("FOO", "hello")])).unwrap();
        assert_eq!(out, "value=hello");
    }

    #[test]
    fn errors_on_unset_var_with_no_default() {
        let err = expand_env_vars("${MISSING}", lookup_from(&[])).unwrap_err();
        assert!(err.to_string().contains("MISSING"));
    }

    #[test]
    fn falls_back_to_default_when_unset() {
        let out = expand_env_vars("${MISSING:-fallback}", lookup_from(&[])).unwrap();
        assert_eq!(out, "fallback");
    }

    #[test]
    fn ignores_default_when_var_set() {
        let out = expand_env_vars("${FOO:-fallback}", lookup_from(&[("FOO", "actual")])).unwrap();
        assert_eq!(out, "actual");
    }

    #[test]
    fn empty_default_yields_empty_string() {
        let out = expand_env_vars("[${MISSING:-}]", lookup_from(&[])).unwrap();
        assert_eq!(out, "[]");
    }

    #[test]
    fn substitutes_multiple_vars_in_one_string() {
        let out = expand_env_vars(
            "postgres://u:p@${HOST}:${PORT}/x",
            lookup_from(&[("HOST", "db.example.com"), ("PORT", "5432")]),
        )
        .unwrap();
        assert_eq!(out, "postgres://u:p@db.example.com:5432/x");
    }

    #[test]
    fn dollar_without_braces_passes_through() {
        // Bracket-less `$IDENT` is intentionally left alone — only the
        // `${...}` form is treated as an env reference.
        let s = "$HOME and $USER stay literal";
        let out = expand_env_vars(s, lookup_from(&[])).unwrap();
        assert_eq!(out, s);
    }

    #[test]
    fn invalid_identifier_passes_through_verbatim() {
        // Digit-leading is not a valid identifier; `${1NOT_VALID}` stays literal.
        let s = "${1NOT_VALID}";
        assert_eq!(expand_env_vars(s, lookup_from(&[])).unwrap(), s);
    }

    #[test]
    fn unclosed_brace_errors() {
        let err = expand_env_vars("${UNCLOSED", lookup_from(&[])).unwrap_err();
        assert!(err.to_string().contains("unclosed"));
    }

    #[test]
    fn substitutes_inside_toml_string_values() {
        let toml_input = r#"
[backend]
type = "postgres"
url = "${DB}"
"#;
        let expanded =
            expand_env_vars(toml_input, lookup_from(&[("DB", "postgres://u:p@h/d")])).unwrap();
        assert!(expanded.contains(r#"url = "postgres://u:p@h/d""#));
    }

    #[test]
    fn is_valid_var_name_accepts_typical_names() {
        assert!(is_valid_var_name("DATABASE_URL"));
        assert!(is_valid_var_name("_PRIVATE"));
        assert!(is_valid_var_name("X"));
        assert!(is_valid_var_name("X1"));
    }

    #[test]
    fn is_valid_var_name_rejects_bad_names() {
        assert!(!is_valid_var_name(""));
        assert!(!is_valid_var_name("1LEADING_DIGIT"));
        assert!(!is_valid_var_name("HAS SPACE"));
        assert!(!is_valid_var_name("HAS-DASH"));
        assert!(!is_valid_var_name("HAS.DOT"));
    }

    #[test]
    fn from_file_loads_static_toml() {
        // Integration sanity that the from_file path still works after the
        // expansion step is wired in. Uses a config with no env-var
        // references to keep the test hermetic.
        let path = std::env::temp_dir().join("assay-engine-config-from-file-static.toml");
        std::fs::write(
            &path,
            r#"
[server]
bind_addr = "127.0.0.1:3000"

[backend]
type = "sqlite"
data_dir = "/tmp/assay-engine-test-data-static"
"#,
        )
        .unwrap();
        let cfg = EngineConfig::from_file(&path).unwrap();
        let _ = std::fs::remove_file(&path);
        match cfg.backend {
            BackendConfig::Sqlite { ref data_dir, .. } => {
                assert_eq!(data_dir, "/tmp/assay-engine-test-data-static");
            }
            _ => panic!("expected sqlite backend"),
        }
    }

    fn minimal_config_with(sections: &str) -> EngineConfig {
        let base = r#"
[server]
bind_addr = "127.0.0.1:3000"

[backend]
type = "sqlite"
data_dir = ":memory:"
"#;
        toml::from_str(&format!("{base}{sections}")).unwrap()
    }

    #[test]
    fn the_vault_compat_facade_is_off_until_an_operator_asks_for_it() {
        let cfg = minimal_config_with("");

        assert!(!cfg.vault.hashicorp_compat.enabled);
        assert_eq!(cfg.vault.hashicorp_compat.mount, "secrets");
    }

    #[test]
    fn the_vault_compat_mount_is_operator_selectable() {
        let cfg = minimal_config_with(
            r#"
[vault.hashicorp_compat]
enabled = true
mount = "kv"
"#,
        );

        assert!(cfg.vault.hashicorp_compat.enabled);
        assert_eq!(cfg.vault.hashicorp_compat.mount, "kv");
    }

    /// An omitted section has to mean "read the variable the engine has
    /// always read", not "read the variable named nothing" — which is
    /// what a derived `Default` would give.
    #[test]
    fn omitting_the_sealing_section_still_reads_the_usual_variable() {
        let cfg = minimal_config_with("");

        assert_eq!(cfg.vault.sealing.source, "env");
        assert_eq!(cfg.vault.sealing.var, "ASSAY_VAULT_SEAL_KEY");
        assert!(cfg.vault.sealing.path.is_none());
    }

    /// Both escapes default off. A deployment gets the protection by
    /// saying nothing, and gives it up only by writing it down.
    #[test]
    fn a_plaintext_master_key_is_refused_until_an_operator_asks_for_it() {
        let cfg = minimal_config_with("");

        assert!(!cfg.vault.sealing.allow_plaintext_kek);
        assert!(!cfg.vault.sealing.allow_plaintext_migration);
    }

    #[test]
    fn a_file_backed_seal_key_deserializes() {
        let cfg = minimal_config_with(
            r#"
[vault.sealing]
source = "file"
path = "/run/secrets/vault-seal-key"
"#,
        );

        assert_eq!(cfg.vault.sealing.source, "file");
        assert_eq!(
            cfg.vault.sealing.path.as_deref(),
            Some("/run/secrets/vault-seal-key")
        );
        // The variable name keeps its default even when unused, so
        // switching `source` back needs no second edit.
        assert_eq!(cfg.vault.sealing.var, "ASSAY_VAULT_SEAL_KEY");
    }

    #[test]
    fn a_passphrase_and_its_salt_deserialize() {
        let cfg = minimal_config_with(
            r#"
[vault.sealing]
source = "passphrase"
passphrase = "correct horse battery staple correct horse"
salt = "an-example-public-salt"
"#,
        );

        assert_eq!(cfg.vault.sealing.source, "passphrase");
        assert_eq!(
            cfg.vault.sealing.passphrase.as_deref(),
            Some("correct horse battery staple correct horse")
        );
        assert_eq!(
            cfg.vault.sealing.salt.as_deref(),
            Some("an-example-public-salt")
        );
    }

    #[test]
    fn a_deployment_that_named_its_own_variable_keeps_it() {
        let cfg = minimal_config_with(
            r#"
[vault.sealing]
var = "MY_DEPLOYMENT_SEAL_KEY"
"#,
        );

        assert_eq!(cfg.vault.sealing.source, "env");
        assert_eq!(cfg.vault.sealing.var, "MY_DEPLOYMENT_SEAL_KEY");
    }

    /// A `path` with no `source = "file"` parses fine — the engine
    /// refuses it later rather than silently reading the environment
    /// instead. This just pins that the fields survive deserialization
    /// so that check has something to look at.
    #[test]
    fn a_field_belonging_to_another_source_still_deserializes() {
        let cfg = minimal_config_with(
            r#"
[vault.sealing]
path = "/run/secrets/vault-seal-key"
"#,
        );

        assert_eq!(cfg.vault.sealing.source, "env");
        assert!(cfg.vault.sealing.path.is_some());
    }

    /// `GET /api/v1/engine/core/config` serializes this struct whole to
    /// an operator, and `${VAR}` is expanded before parsing — so the
    /// real seal key is in the struct by then. Whoever holds that
    /// response and a database dump would have every vault secret, which
    /// is the exact property sealing exists to remove.
    #[test]
    fn an_inline_seal_key_never_survives_serialization() {
        const SEAL: &str = "a-very-secret-inline-seal-key-33c";
        let cfg = minimal_config_with(&format!(
            r#"
[vault.sealing]
source = "value"
value = "{SEAL}"
"#
        ));

        let json = serde_json::to_string(&cfg).expect("serialize config");
        assert!(!json.contains(SEAL), "the seal key leaked: {json}");
        assert!(
            json.contains("[REDACTED]"),
            "an operator still has to see that a value is configured: {json}"
        );
        // The struct itself keeps the real value — only the way out is
        // redacted, or boot could not seal anything.
        assert_eq!(cfg.vault.sealing.value.as_deref(), Some(SEAL));
    }

    #[test]
    fn a_passphrase_never_survives_serialization_but_its_salt_does() {
        const PHRASE: &str = "correct horse battery staple correct horse";
        let cfg = minimal_config_with(&format!(
            r#"
[vault.sealing]
source = "passphrase"
passphrase = "{PHRASE}"
salt = "an-example-public-salt"
"#
        ));

        let json = serde_json::to_string(&cfg).expect("serialize config");
        assert!(!json.contains(PHRASE), "the passphrase leaked: {json}");
        // The salt is public by design: it separates deployments rather
        // than hiding anything, and is useless without the passphrase.
        assert!(json.contains("an-example-public-salt"), "{json}");
    }

    /// A config that reaches a log line through `Debug` is the same leak
    /// by another route.
    #[test]
    fn debug_redacts_the_secret_bearing_sealing_fields() {
        const SEAL: &str = "a-very-secret-inline-seal-key-33c";
        let cfg = minimal_config_with(&format!(
            r#"
[vault.sealing]
source = "value"
value = "{SEAL}"
"#
        ));

        let rendered = format!("{:?}", cfg.vault.sealing);
        assert!(!rendered.contains(SEAL), "{rendered}");
        assert!(rendered.contains("[REDACTED]"), "{rendered}");
        assert!(rendered.contains("source"), "{rendered}");
    }

    /// An absent source must read as absent, not as a redacted one that
    /// is set — otherwise the endpoint lies about what is configured.
    #[test]
    fn an_unset_seal_source_serializes_as_absent() {
        let cfg = minimal_config_with("");
        let json = serde_json::to_string(&cfg).expect("serialize config");
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();

        assert!(v["vault"]["sealing"]["value"].is_null(), "{json}");
        assert!(v["vault"]["sealing"]["passphrase"].is_null(), "{json}");
    }

    #[test]
    fn the_sealing_escapes_are_settable_and_independent() {
        let cfg = minimal_config_with(
            r#"
[vault.sealing]
allow_plaintext_migration = true
"#,
        );

        assert!(cfg.vault.sealing.allow_plaintext_migration);
        assert!(
            !cfg.vault.sealing.allow_plaintext_kek,
            "consenting to a migration must not also permit a plaintext key"
        );
    }

    #[test]
    fn password_recovery_is_disabled_by_default() {
        let cfg: EngineConfig = toml::from_str(
            r#"
[server]
bind_addr = "127.0.0.1:3000"

[backend]
type = "sqlite"
data_dir = ":memory:"
"#,
        )
        .unwrap();

        assert!(!cfg.auth.recovery.enabled);
        assert_eq!(cfg.auth.recovery.token_ttl_seconds, 900);
        assert_eq!(cfg.auth.recovery.request_cooldown_seconds, 60);
        assert!(cfg.auth.recovery.smtp.is_none());
    }

    #[test]
    fn password_recovery_smtp_configuration_deserializes() {
        let cfg: EngineConfig = toml::from_str(
            r#"
[server]
bind_addr = "127.0.0.1:3000"

[backend]
type = "sqlite"
data_dir = ":memory:"

[auth.recovery]
enabled = true
token_ttl_seconds = 1200
request_cooldown_seconds = 90

[auth.recovery.smtp]
host = "smtp.example.com"
port = 587
username = "mailer"
password = "secret"
from = "Example Auth <noreply@example.com>"
starttls = true
"#,
        )
        .unwrap();

        assert!(cfg.auth.recovery.enabled);
        assert_eq!(cfg.auth.recovery.token_ttl_seconds, 1200);
        assert_eq!(cfg.auth.recovery.request_cooldown_seconds, 90);
        let smtp = cfg.auth.recovery.smtp.unwrap();
        assert_eq!(smtp.host, "smtp.example.com");
        assert_eq!(smtp.port, 587);
        assert_eq!(smtp.username, "mailer");
        assert_eq!(smtp.password, "secret");
        assert_eq!(smtp.from, "Example Auth <noreply@example.com>");
        assert!(smtp.starttls);
    }

    #[test]
    fn flagship_host_and_dashboard_boundaries_deserialize() {
        let cfg: EngineConfig = toml::from_str(
            r#"
[server]
bind_addr = "127.0.0.1:3000"
allowed_hosts = ["auth.assay.rs", "engine.assay.rs"]

[backend]
type = "sqlite"
data_dir = ":memory:"

[dashboard]
enabled = true
operator_enabled = false
auth_ui_enabled = true
"#,
        )
        .unwrap();

        assert_eq!(
            cfg.server.allowed_hosts,
            ["auth.assay.rs", "engine.assay.rs"]
        );
        assert!(!cfg.dashboard.operator_enabled());
        assert!(cfg.dashboard.auth_ui_enabled());
    }

    #[test]
    fn dashboard_surface_flags_preserve_the_legacy_enabled_default() {
        let dashboard = DashboardConfig::default();
        assert!(dashboard.operator_enabled());
        assert!(dashboard.auth_ui_enabled());
        assert!(ServerConfig::default().allowed_hosts.is_empty());
    }
}
