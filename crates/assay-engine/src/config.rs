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
    /// Operator-supplied canonical URL the engine is reached at — used
    /// as the OIDC `iss` claim, biscuit token issuer, passkey origin,
    /// and the base for federation callbacks. Defaults to the bind addr
    /// over plain HTTP for local dev convenience; production deployments
    /// MUST override this with the public HTTPS URL.
    #[serde(default = "default_public_url")]
    pub public_url: String,
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
    /// JWT issuer + OIDC `iss` claim. Defaults to
    /// `<server.public_url>/auth` when unset, which matches the route
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

/// Vault-module deployment shape. Read by the engine binary when the
/// `vault` Cargo feature is compiled in AND `engine.modules.vault` is
/// enabled. The master KEK is sealed at rest under operator-supplied
/// unseal material (#113) — boot fails closed if neither
/// `unseal_key_source` nor `dev_plaintext_kek` is set.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[non_exhaustive]
pub struct VaultConfig {
    /// Where the engine reads the KEK unseal material. One of:
    /// - `env:NAME`        — base64 32-byte key (or `passphrase:<text>`)
    ///   from environment variable `NAME`. Preferred for K8s/systemd.
    /// - `file:/path`      — same, read from a `0600` file.
    /// - `base64:BBBB`     — inline base64 raw key (dev/test).
    /// - `passphrase:TEXT` — inline passphrase, Argon2id-stretched.
    ///
    /// Empty (the default) ⇒ no source. Combined with
    /// `dev_plaintext_kek = false` this makes the vault fail closed at
    /// boot rather than persist the KEK in plaintext. Supports the
    /// engine's `${VAR}` expansion, so `unseal_key_source = "env:..."`
    /// is the recommended indirection (don't inline secrets in TOML).
    #[serde(default)]
    pub unseal_key_source: String,
    /// Dev-only escape hatch. When `true`, the KEK is persisted in
    /// PLAINTEXT at rest and boot logs a CRITICAL warning. NEVER enable
    /// for real secrets — a DB read decrypts everything. Default false.
    #[serde(default)]
    pub dev_plaintext_kek: bool,
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
    /// scope passkeys to. Defaults to the host of `server.public_url`.
    pub rp_id: Option<String>,
    /// Human-readable label browsers show. Defaults to `"Assay"`.
    pub rp_name: Option<String>,
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

#[derive(Clone, Debug, Deserialize, Serialize)]
#[non_exhaustive]
pub struct DashboardConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
}

impl Default for DashboardConfig {
    fn default() -> Self {
        // When the `[dashboard]` section is omitted entirely from
        // engine.toml, serde calls Default::default() — and bool's
        // derived default is `false`. We want `enabled: true` here so
        // a fresh engine.toml without a [dashboard] section still
        // mounts the SPAs out of the box.
        Self { enabled: true }
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
}
