//! Action derivation: a host declares that one action derives from another,
//! so a statement naming a coarse action covers a declared family of finer
//! ones. Not a wildcard — the expansion is closed and enumerable. Derivation
//! is registry data, never policy data, and single-parent by design: deny
//! expands exactly as allow does, so multiple parents would let a deny on any
//! ancestor silently kill a leaf.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

pub const MAX_DERIVATION_DEPTH: usize = 64;

/// Mirrors condition evaluation. An unresolvable walk is NOT a no-match:
/// collapsing it would let a deny naming a base stop covering its leaves.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ActionMatch {
    Match,
    NoMatch,
    Unresolvable,
}

impl ActionMatch {
    /// Across a statement's actions, keep the strongest signal: a definitive
    /// match beats an unresolvable walk, which beats a plain no-match.
    pub fn or_stronger(self, other: Self) -> Self {
        match (self, other) {
            (Self::Match, _) | (_, Self::Match) => Self::Match,
            (Self::Unresolvable, _) | (_, Self::Unresolvable) => Self::Unresolvable,
            _ => Self::NoMatch,
        }
    }
}

/// Child action to the action it derives from.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ActionDerivation(pub BTreeMap<String, String>);

impl ActionDerivation {
    pub fn parent_of(&self, action: &str) -> Option<&str> {
        self.0.get(action).map(String::as_str)
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl FromIterator<(String, String)> for ActionDerivation {
    fn from_iter<T: IntoIterator<Item = (String, String)>>(iter: T) -> Self {
        Self(iter.into_iter().collect())
    }
}

/// Walks upward from the REQUESTED action: statements name the base, checks
/// name the leaf. Without a derivation this is exact equality.
pub fn match_action<'a>(
    pattern: &str,
    requested: &'a str,
    parents: Option<&'a ActionDerivation>,
) -> ActionMatch {
    if pattern == requested {
        return ActionMatch::Match;
    }
    let Some(parents) = parents else {
        return ActionMatch::NoMatch;
    };
    let mut seen: BTreeSet<&str> = BTreeSet::from([requested]);
    let mut current = parents.parent_of(requested);
    let mut depth = 0usize;
    while let Some(action) = current {
        if action.is_empty() {
            return ActionMatch::NoMatch;
        }
        if action == pattern {
            return ActionMatch::Match;
        }
        if seen.contains(action) || depth >= MAX_DERIVATION_DEPTH {
            return ActionMatch::Unresolvable;
        }
        seen.insert(action);
        current = parents.parent_of(action);
        depth += 1;
    }
    ActionMatch::NoMatch
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActionCatalogueEntry {
    pub action: String,
    #[serde(
        default,
        alias = "derives_from",
        skip_serializing_if = "Option::is_none"
    )]
    pub derives_from: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// A malformed vocabulary is a host bug at boot: the engine must never hold a
/// derivation graph it cannot enumerate, so indexing rejects rather than
/// degrades.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ActionCatalogueError(pub String);

impl std::fmt::Display for ActionCatalogueError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for ActionCatalogueError {}

#[derive(Clone, Debug, Default)]
pub struct ActionCatalogue {
    entries: Vec<ActionCatalogueEntry>,
    derivation: ActionDerivation,
    children: BTreeMap<String, Vec<String>>,
    actions: BTreeSet<String>,
}

impl ActionCatalogue {
    pub fn index(entries: Vec<ActionCatalogueEntry>) -> Result<Self, ActionCatalogueError> {
        let mut actions: BTreeSet<String> = BTreeSet::new();
        for entry in &entries {
            if entry.action.is_empty() {
                return Err(ActionCatalogueError("catalogue entry has no action".into()));
            }
            if entry.action.contains('*') {
                return Err(ActionCatalogueError(format!(
                    "action \"{}\" contains a wildcard",
                    entry.action
                )));
            }
            if !actions.insert(entry.action.clone()) {
                return Err(ActionCatalogueError(format!(
                    "action \"{}\" declared twice",
                    entry.action
                )));
            }
        }

        let mut derivation = BTreeMap::new();
        let mut children: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for entry in &entries {
            let Some(parent) = entry.derives_from.as_ref() else {
                continue;
            };
            if !actions.contains(parent) {
                return Err(ActionCatalogueError(format!(
                    "action \"{}\" derives from unknown action \"{parent}\"",
                    entry.action
                )));
            }
            if *parent == entry.action {
                return Err(ActionCatalogueError(format!(
                    "action \"{}\" derives from itself",
                    entry.action
                )));
            }
            derivation.insert(entry.action.clone(), parent.clone());
            children
                .entry(parent.clone())
                .or_default()
                .push(entry.action.clone());
        }

        let catalogue = Self {
            entries,
            derivation: ActionDerivation(derivation),
            children,
            actions,
        };
        catalogue.reject_cycles()?;
        Ok(catalogue)
    }

    fn reject_cycles(&self) -> Result<(), ActionCatalogueError> {
        for start in self.derivation.0.keys() {
            let mut seen: BTreeSet<&str> = BTreeSet::from([start.as_str()]);
            let mut current = self.derivation.parent_of(start);
            while let Some(action) = current {
                if !seen.insert(action) {
                    return Err(ActionCatalogueError(format!(
                        "derivation cycle through action \"{action}\""
                    )));
                }
                current = self.derivation.parent_of(action);
            }
        }
        Ok(())
    }

    pub fn derivation(&self) -> &ActionDerivation {
        &self.derivation
    }

    pub fn entries(&self) -> &[ActionCatalogueEntry] {
        &self.entries
    }

    pub fn is_known_action(&self, action: &str) -> bool {
        self.actions.contains(action)
    }

    /// The enumerability that separates a derivation from a wildcard.
    pub fn descendants_of(&self, base: &str) -> Vec<String> {
        let mut out: BTreeSet<String> = BTreeSet::new();
        let mut queue: Vec<String> = self.children.get(base).cloned().unwrap_or_default();
        while let Some(next) = queue.pop() {
            if !out.insert(next.clone()) {
                continue;
            }
            queue.extend(self.children.get(&next).cloned().unwrap_or_default());
        }
        out.into_iter().collect()
    }
}
