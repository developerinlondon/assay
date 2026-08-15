//! The vocabulary as DATA — the contract an administration surface consumes,
//! so one UI can administer any host that serves this shape.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::action::ActionCatalogue;
use crate::condition::{
    CONDITION_OPERATORS, ConditionKeyType, ConditionKeys, SET_OPERATORS, builtin_condition_keys,
    resolve_condition_keys,
};

/// Bumped only when an old consumer would render a WRONG form from a new
/// document — never for a purely additive field.
pub const DESCRIPTOR_VERSION: u32 = 1;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DescribedAction {
    pub action: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub derives_from: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DescribedConditionKey {
    #[serde(rename = "type")]
    pub key_type: ConditionKeyType,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub lowercase: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub built_in: bool,
    /// Derived from `type`, never authored: a UI that duplicated that table
    /// would drift from it.
    pub operators: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthzDescriptor {
    pub version: u32,
    pub actions: Vec<DescribedAction>,
    pub action_closures: BTreeMap<String, Vec<String>>,
    pub condition_keys: BTreeMap<String, DescribedConditionKey>,
    pub scope_kinds: Vec<String>,
    pub set_operators: Vec<String>,
}

/// A projection of the declared vocabulary, holding no state of its own.
pub fn describe(
    catalogue: Option<&ActionCatalogue>,
    condition_keys: &ConditionKeys,
    scope_kinds: &[String],
) -> AuthzDescriptor {
    let builtins = builtin_condition_keys();
    let mut actions = Vec::new();
    let mut action_closures = BTreeMap::new();
    if let Some(catalogue) = catalogue {
        for entry in catalogue.entries() {
            actions.push(DescribedAction {
                action: entry.action.clone(),
                derives_from: entry.derives_from.clone(),
                title: entry.title.clone(),
                description: entry.description.clone(),
            });
            action_closures.insert(
                entry.action.clone(),
                catalogue.descendants_of(&entry.action),
            );
        }
    }

    let described_keys = resolve_condition_keys(condition_keys)
        .into_iter()
        .map(|(key, spec)| {
            let described = DescribedConditionKey {
                key_type: spec.key_type,
                lowercase: spec.lowercase,
                built_in: builtins.contains_key(&key),
                operators: operators_for(spec.key_type),
                title: spec.title,
                description: spec.description,
            };
            (key, described)
        })
        .collect();

    AuthzDescriptor {
        version: DESCRIPTOR_VERSION,
        actions,
        action_closures,
        condition_keys: described_keys,
        scope_kinds: scope_kinds.to_vec(),
        set_operators: SET_OPERATORS.iter().map(|op| op.as_str().into()).collect(),
    }
}

fn operators_for(key_type: ConditionKeyType) -> Vec<String> {
    CONDITION_OPERATORS
        .iter()
        .filter(|op| op.key_type() == key_type)
        .map(|op| op.as_str().to_string())
        .collect()
}
