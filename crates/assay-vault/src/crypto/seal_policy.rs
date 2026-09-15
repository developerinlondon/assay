//! What boot does when there is no unseal material, and when re-sealing
//! a store would be irreversible.
//!
//! Sealing that is opt-in by environment variable is sealing most
//! deployments will not have: a vault-enabled engine that never set
//! `ASSAY_VAULT_SEAL_KEY` used to mint a fresh KEK, write it to
//! [`crate::crypto::kek::KEK_TABLE`] in the clear, log one warning, and
//! serve — which is exactly the property sealing exists to remove. So
//! boot now fails closed, and the way past it is a config field an
//! operator has to type, never an environment variable somebody could
//! set by accident in a deployment template.
//!
//! The second decision here is the one-way one. Re-sealing a store that
//! holds a plaintext KEK rewrites that row in place; afterwards the key
//! exists only under the operator's material, and losing it loses every
//! secret the vault wraps, with no plaintext copy left to fall back on.
//! That used to happen unasked on the first boot that had a key. It now
//! needs its own flag, and the refusal names the backup to take first.

use crate::crypto::env_seal::{ENV_VAR, MIN_CHARS, SealKey};
use crate::crypto::kek::KEK_TABLE;
use crate::crypto::seal_source::SealSource;
use crate::error::{Result, VaultError};

/// The config field that permits a plaintext KEK at rest.
pub const CFG_ALLOW_PLAINTEXT_KEK: &str = "[vault.sealing] allow_plaintext_kek";

/// The config field that permits the one-way re-seal of a plaintext KEK.
pub const CFG_ALLOW_PLAINTEXT_MIGRATION: &str = "[vault.sealing] allow_plaintext_migration";

/// How this deployment gets its unseal material, and what it has
/// explicitly accepted in the absence of any.
#[derive(Clone, Debug, Default)]
pub struct SealPolicy {
    /// Where to look. `None` means the deployment configured no source
    /// at all, which is distinct from a source that resolved to nothing.
    pub source: Option<SealSource>,
    /// Permit minting and storing the KEK in the clear. Local
    /// development only; logged at ERROR on every boot that uses it.
    pub allow_plaintext_kek: bool,
    /// Permit rewriting an existing plaintext KEK row as sealed. One-way
    /// — see the module comment. [`Self::resolve`] carries this onto
    /// [`Unseal::Sealed`], which is the only thing the loaders read, so
    /// this field is the single source of truth for the decision.
    pub allow_plaintext_migration: bool,
}

/// The resolved decision boot proceeds under.
///
/// This carries the migration consent rather than leaving the loaders to
/// take it as a second argument. A one-way, secret-destroying operation
/// should not have two places to say yes: a caller that set
/// `allow_plaintext_migration: false` on the policy and passed `true`
/// alongside it would re-seal a store the policy just refused to.
#[non_exhaustive]
#[derive(Debug)]
pub enum Unseal {
    /// Material was found. The KEK is sealed under it at rest.
    Sealed {
        /// The material the KEK is sealed under.
        key: SealKey,
        /// Whether a store still holding a plaintext KEK may be rewritten
        /// as sealed on this boot.
        migrate_plaintext: bool,
    },
    /// No material, and the operator explicitly accepted a plaintext KEK
    /// at rest. This permits *minting* a key in the clear; it never
    /// opens a store that is already sealed.
    PlaintextPermitted,
}

impl Unseal {
    /// The seal key, if there is one. `None` is the permitted-plaintext
    /// case, which every caller has to handle as "there is no key",
    /// never as "sealing is optional here".
    pub fn seal_key(&self) -> Option<&SealKey> {
        match self {
            Self::Sealed { key, .. } => Some(key),
            Self::PlaintextPermitted => None,
        }
    }

    /// Whether this boot may perform the one-way re-seal of a store that
    /// currently holds its KEK in the clear. False without material,
    /// because there would be nothing to re-seal it under.
    pub fn may_migrate_plaintext(&self) -> bool {
        match self {
            Self::Sealed {
                migrate_plaintext, ..
            } => *migrate_plaintext,
            Self::PlaintextPermitted => false,
        }
    }

    pub fn is_sealed(&self) -> bool {
        matches!(self, Self::Sealed { .. })
    }
}

impl SealPolicy {
    /// A policy reading the default environment variable and permitting
    /// nothing — the shape a deployment gets when it omits
    /// `[vault.sealing]` entirely.
    pub fn from_default_env() -> Self {
        Self {
            source: Some(SealSource::default_env()),
            allow_plaintext_kek: false,
            allow_plaintext_migration: false,
        }
    }

    /// Resolve the configured source, or fail closed naming what to set.
    ///
    /// The error is the whole point of this function, so it spells out
    /// every way forward rather than telling the operator a flag exists.
    pub fn resolve(&self) -> Result<Unseal> {
        if let Some(source) = &self.source
            && let Some(key) = source.resolve()?
        {
            return Ok(Unseal::Sealed {
                key,
                migrate_plaintext: self.allow_plaintext_migration,
            });
        }

        if self.allow_plaintext_kek {
            tracing::error!(
                target: "assay-vault",
                table = KEK_TABLE,
                "the vault master key is held in {KEK_TABLE} in the clear, because \
                 `{CFG_ALLOW_PLAINTEXT_KEK}` is set. Anyone who can read a database dump, a \
                 backup or a replica can read every secret in the vault. This is a local \
                 development setting."
            );
            return Ok(Unseal::PlaintextPermitted);
        }

        // Only an environment source can resolve to nothing — every other
        // one errors on its own terms — so this names the variable that
        // was actually looked up, which a deployment may have renamed.
        let variable = self
            .source
            .as_ref()
            .and_then(SealSource::env_var)
            .unwrap_or(ENV_VAR);

        Err(VaultError::Invalid(format!(
            "the vault module is enabled but no unseal material is configured, so its master key \
             would be written to {KEK_TABLE} in the clear and a database dump would be a copy of \
             every secret. Set {variable} to a string of at least {MIN_CHARS} characters, or point \
             `[vault.sealing]` at a file, an inline value or a passphrase. For local development \
             only, set `{CFG_ALLOW_PLAINTEXT_KEK} = true` in engine.toml — there is deliberately \
             no environment variable for that."
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use zeroize::Zeroizing;

    const RAW: &str = "YzBmZTFhMmIzYzRkNWU2ZjcwODE5MmEzYjRjNWQ2ZTc=";

    fn value_source() -> SealSource {
        SealSource::Value {
            value: Zeroizing::new(RAW.to_string()),
        }
    }

    fn unset_env_source() -> SealSource {
        SealSource::Env {
            var: "ASSAY_VAULT_SEAL_KEY_UNSET_FOR_POLICY_TESTS".to_string(),
        }
    }

    #[test]
    fn configured_material_seals() {
        let policy = SealPolicy {
            source: Some(value_source()),
            ..SealPolicy::default()
        };
        assert!(policy.resolve().unwrap().is_sealed());
    }

    /// The consent the loaders act on has to be the one the operator
    /// wrote down. If the policy field and the loaders could disagree,
    /// a caller could re-seal a store the policy had just refused to.
    #[test]
    fn migration_consent_travels_with_the_resolved_decision() {
        let refused = SealPolicy {
            source: Some(value_source()),
            ..SealPolicy::default()
        };
        assert!(!refused.resolve().unwrap().may_migrate_plaintext());

        let granted = SealPolicy {
            source: Some(value_source()),
            allow_plaintext_kek: false,
            allow_plaintext_migration: true,
        };
        assert!(granted.resolve().unwrap().may_migrate_plaintext());
    }

    /// Permitted plaintext means there is no material, so there is
    /// nothing to re-seal under — consent there would be meaningless.
    #[test]
    fn permitted_plaintext_never_carries_migration_consent() {
        let policy = SealPolicy {
            source: Some(unset_env_source()),
            allow_plaintext_kek: true,
            allow_plaintext_migration: true,
        };
        let unseal = policy.resolve().unwrap();
        assert!(!unseal.is_sealed());
        assert!(!unseal.may_migrate_plaintext());
    }

    /// The gap the whole change exists to close.
    #[test]
    fn no_material_and_no_escape_hatch_refuses_to_boot() {
        let policy = SealPolicy {
            source: Some(unset_env_source()),
            ..SealPolicy::default()
        };
        let err = policy.resolve().unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains(ENV_VAR),
            "the message must name what to set: {msg}"
        );
        assert!(msg.contains(CFG_ALLOW_PLAINTEXT_KEK), "{msg}");
        assert!(msg.contains(KEK_TABLE), "{msg}");
    }

    /// A deployment that configured nothing at all fails the same way as
    /// one whose variable is unset.
    #[test]
    fn no_source_at_all_refuses_to_boot() {
        let err = SealPolicy::default().resolve().unwrap_err();
        assert!(err.to_string().contains(ENV_VAR), "{err}");
    }

    /// A deployment whose secret injector has its own conventions gets
    /// told the name it actually configured, not the default.
    #[test]
    fn the_refusal_names_the_variable_the_deployment_renamed() {
        let policy = SealPolicy {
            source: Some(SealSource::Env {
                var: "MY_DEPLOYMENT_SEAL_KEY".to_string(),
            }),
            ..SealPolicy::default()
        };
        let err = policy.resolve().unwrap_err();
        assert!(err.to_string().contains("MY_DEPLOYMENT_SEAL_KEY"), "{err}");
    }

    #[test]
    fn the_escape_hatch_permits_plaintext_but_yields_no_key() {
        let policy = SealPolicy {
            source: Some(unset_env_source()),
            allow_plaintext_kek: true,
            allow_plaintext_migration: false,
        };
        let unseal = policy.resolve().unwrap();
        assert!(!unseal.is_sealed());
        assert!(
            unseal.seal_key().is_none(),
            "permitted plaintext must never look like a key"
        );
    }

    /// The hatch is about the absence of material, not about tolerating
    /// broken material — a configured source that fails still fails.
    #[test]
    fn the_escape_hatch_does_not_excuse_a_broken_source() {
        let policy = SealPolicy {
            source: Some(SealSource::Value {
                value: Zeroizing::new("too-short".to_string()),
            }),
            allow_plaintext_kek: true,
            allow_plaintext_migration: false,
        };
        assert!(
            policy.resolve().is_err(),
            "a value that cannot derive a key is a misconfiguration, not an absent key"
        );
    }

    /// `Debug` on the policy is one derive away from a config dump, and
    /// the source it holds can carry a passphrase.
    #[test]
    fn debug_does_not_leak_the_configured_value() {
        let rendered = format!(
            "{:?}",
            SealPolicy {
                source: Some(value_source()),
                ..SealPolicy::default()
            }
        );
        assert!(!rendered.contains(RAW), "{rendered}");
    }
}
