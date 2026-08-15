//! The enforcement seam: every "may these subjects do this action on this
//! resource?" question goes through `Engine::check`. The engine is
//! in-process and storage-free — a host hands it the grants it resolved.

use chrono::{DateTime, Utc};
use serde_json::Value;

use crate::action::{ActionCatalogue, ActionDerivation};
use crate::condition::{
    ConditionContext, ConditionKeys, builtin_context_entries, resolve_condition_keys,
};
use crate::describe::{AuthzDescriptor, describe};
use crate::evaluate::{
    Decision, Outcome, Query, Reason, applicable_grants, decide, refuse_bad_chain,
};
use crate::model::{ResolvedGrant, ScopeEntry, Statement, SubjectEntry};
use crate::validate::{Vocabulary, validate_statements};

#[derive(Default)]
pub struct EngineConfig {
    pub grants: Vec<ResolvedGrant>,
    /// Grants an app-owned synthesizer contributes (membership roles, an
    /// open-mode baseline). Unioned with `grants` and confined by the same
    /// subject and scope matching.
    pub synthesized_grants: Vec<ResolvedGrant>,
    pub condition_keys: ConditionKeys,
    /// When declared, a chain entry of any other kind denies.
    pub scope_kinds: Option<Vec<String>>,
    /// The chain used when a check names none.
    pub default_scope_chain: Vec<ScopeEntry>,
    pub actions: Option<ActionCatalogue>,
    /// Derivation declared directly, for a host with no full catalogue.
    pub action_derivation: Option<ActionDerivation>,
}

pub struct Engine {
    config: EngineConfig,
    keys: ConditionKeys,
}

#[derive(Default)]
pub struct CheckOptions {
    pub scope_chain: Option<Vec<ScopeEntry>>,
    pub context: ConditionContext,
    /// Resolved by the host at its boundary. Absent leaves the built-in key
    /// unpopulated, so a condition on it fails closed.
    pub source_ip: Option<String>,
    pub now: Option<DateTime<Utc>>,
    /// The host says this caller bypasses policy entirely, so a careless
    /// broad deny cannot lock the operator out.
    pub bypass: bool,
}

/// The deny-wins verdict plus whether the STORED grants alone would allow it,
/// which tells an explicit grant apart from an ambient default.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CheckDetail {
    pub decision: Decision,
    pub reason: Reason,
    pub allowed: bool,
    pub allowed_by_stored_grants: bool,
}

impl Engine {
    pub fn new(config: EngineConfig) -> Self {
        let keys = resolve_condition_keys(&config.condition_keys);
        Self { config, keys }
    }

    pub fn check(
        &self,
        subjects: &[SubjectEntry],
        action: &str,
        resource: &str,
        options: &CheckOptions,
    ) -> CheckDetail {
        let chain = options
            .scope_chain
            .as_deref()
            .unwrap_or(&self.config.default_scope_chain);
        let context = self.build_context(options);
        let query = Query {
            action,
            resource,
            context: &context,
            condition_keys: &self.keys,
            action_derivation: self.derivation(),
        };

        if let Some(refusal) = refuse_bad_chain(chain, self.config.scope_kinds.as_deref()) {
            return self.detail(refusal, false, options.bypass);
        }
        let stored = applicable_grants(&self.config.grants, subjects, chain);
        let synthesized = applicable_grants(&self.config.synthesized_grants, subjects, chain);
        let stored_outcome = decide(&stored, &query);
        let combined: Vec<&ResolvedGrant> =
            stored.iter().chain(synthesized.iter()).copied().collect();
        let outcome = if synthesized.is_empty() {
            stored_outcome
        } else {
            decide(&combined, &query)
        };
        self.detail(
            outcome,
            stored_outcome.decision == Decision::Allow,
            options.bypass,
        )
    }

    /// The grants effective for these subjects over a chain, for a "why" view.
    pub fn grants_for(
        &self,
        subjects: &[SubjectEntry],
        scope_chain: Option<&[ScopeEntry]>,
    ) -> Vec<&ResolvedGrant> {
        let chain = scope_chain.unwrap_or(&self.config.default_scope_chain);
        if refuse_bad_chain(chain, self.config.scope_kinds.as_deref()).is_some() {
            return Vec::new();
        }
        let mut out = applicable_grants(&self.config.grants, subjects, chain);
        out.extend(applicable_grants(
            &self.config.synthesized_grants,
            subjects,
            chain,
        ));
        out
    }

    /// Validate authored statements against this engine's vocabulary. Without
    /// a declared catalogue every non-wildcard action is accepted.
    pub fn validate(&self, statements: &Value) -> Result<Vec<Statement>, String> {
        let accept_all = |_: &str| true;
        let known: Box<dyn Fn(&str) -> bool + '_> = match self.config.actions.as_ref() {
            Some(catalogue) => Box::new(move |action: &str| catalogue.is_known_action(action)),
            None => Box::new(accept_all),
        };
        validate_statements(
            statements,
            &Vocabulary {
                is_known_action: &known,
                condition_keys: &self.keys,
                is_known_resource: None,
            },
        )
    }

    pub fn describe(&self) -> AuthzDescriptor {
        describe(
            self.config.actions.as_ref(),
            &self.config.condition_keys,
            self.config.scope_kinds.as_deref().unwrap_or_default(),
        )
    }

    pub fn condition_keys(&self) -> &ConditionKeys {
        &self.keys
    }

    fn derivation(&self) -> Option<&ActionDerivation> {
        self.config
            .actions
            .as_ref()
            .map(ActionCatalogue::derivation)
            .or(self.config.action_derivation.as_ref())
            .filter(|derivation| !derivation.is_empty())
    }

    /// Built inside the seam from the real check inputs only, never from a
    /// caller-supplied attribute bag. The built-ins always win.
    fn build_context(&self, options: &CheckOptions) -> ConditionContext {
        let mut context = options.context.clone();
        let now = options.now.unwrap_or_else(Utc::now);
        context.extend(builtin_context_entries(now, options.source_ip.as_deref()));
        context
    }

    fn detail(&self, outcome: Outcome, stored_allows: bool, bypass: bool) -> CheckDetail {
        if bypass {
            return CheckDetail {
                decision: Decision::Allow,
                reason: Reason::AdminBypass,
                allowed: true,
                allowed_by_stored_grants: stored_allows,
            };
        }
        CheckDetail {
            decision: outcome.decision,
            reason: outcome.reason,
            allowed: outcome.allowed(),
            allowed_by_stored_grants: stored_allows,
        }
    }
}
