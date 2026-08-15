//! Subjects, scopes, statements and grants.
//!
//! Subject and scope entries are deliberately loose: anything that is not a
//! `{kind, id}` pair of non-empty strings is kept as `Malformed` rather than
//! rejected at parse time, so the evaluator applies the fail-closed rule that
//! belongs to it instead of failing to load the input at all.

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Subject {
    pub kind: String,
    pub id: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Scope {
    pub kind: String,
    pub id: String,
}

macro_rules! loose_ref {
    ($entry:ident, $inner:ident, $doc:literal) => {
        #[doc = $doc]
        #[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
        #[serde(from = "Value", into = "Value")]
        pub enum $entry {
            Valid($inner),
            Malformed,
        }

        impl $entry {
            pub fn valid(&self) -> Option<&$inner> {
                match self {
                    Self::Valid(v) => Some(v),
                    Self::Malformed => None,
                }
            }

            pub fn is_valid(&self) -> bool {
                matches!(self, Self::Valid(_))
            }
        }

        impl From<Value> for $entry {
            fn from(value: Value) -> Self {
                match parse_kind_id(&value) {
                    Some((kind, id)) => Self::Valid($inner { kind, id }),
                    None => Self::Malformed,
                }
            }
        }

        impl From<$entry> for Value {
            fn from(entry: $entry) -> Self {
                match entry {
                    $entry::Valid(v) => serde_json::json!({ "kind": v.kind, "id": v.id }),
                    $entry::Malformed => Value::Null,
                }
            }
        }

        impl From<$inner> for $entry {
            fn from(inner: $inner) -> Self {
                Self::Valid(inner)
            }
        }

        impl Default for $entry {
            fn default() -> Self {
                Self::Malformed
            }
        }
    };
}

loose_ref!(
    SubjectEntry,
    Subject,
    "A subject as the caller supplied it. A `Malformed` entry never matches a grant."
);
loose_ref!(
    ScopeEntry,
    Scope,
    "A scope-chain entry as the caller supplied it. A `Malformed` entry denies the check."
);

fn parse_kind_id(value: &Value) -> Option<(String, String)> {
    let object = value.as_object()?;
    let kind = object.get("kind")?.as_str()?;
    let id = object.get("id")?.as_str()?;
    if kind.is_empty() || id.is_empty() {
        return None;
    }
    Some((kind.to_string(), id.to_string()))
}

/// A statement whose effect is absent or unreadable is `Malformed`: it matches
/// neither an allow pass nor a deny pass, so it is skipped rather than taking
/// the whole engine down at construction.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(from = "Value", into = "Value")]
pub enum Effect {
    Allow,
    Deny,
    #[default]
    Malformed,
}

impl From<Value> for Effect {
    fn from(value: Value) -> Self {
        match value.as_str() {
            Some("allow") => Self::Allow,
            Some("deny") => Self::Deny,
            _ => Self::Malformed,
        }
    }
}

impl From<Effect> for Value {
    fn from(effect: Effect) -> Self {
        match effect {
            Effect::Allow => Value::String("allow".into()),
            Effect::Deny => Value::String("deny".into()),
            Effect::Malformed => Value::Null,
        }
    }
}

/// A single policy statement. `conditions` is held as raw JSON because the
/// evaluator is the last line of defense: a row written by an older code
/// path, a migration, or a future bug must still fail closed here rather than
/// fail to load.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Statement {
    #[serde(default)]
    pub effect: Effect,
    #[serde(default)]
    pub actions: Vec<String>,
    #[serde(default)]
    pub resources: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conditions: Option<Value>,
}

/// Conditions carried by the GRANT, so one curated policy is attachable with
/// different limits per subject. They narrow ALLOW statements only.
pub type GrantBounds = Vec<Value>;

/// A grant folded into evaluation: the policy's statements plus who it was
/// granted to and where in the hierarchy it applies.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResolvedGrant {
    #[serde(default, alias = "policy_id")]
    pub policy_id: String,
    #[serde(default, alias = "policy_name")]
    pub policy_name: String,
    /// Absent means `Malformed`, which drops the grant rather than the engine.
    #[serde(default)]
    pub subject: SubjectEntry,
    #[serde(default)]
    pub scope: ScopeEntry,
    #[serde(default)]
    pub statements: Vec<Statement>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bounds: Option<GrantBounds>,
}

/// A resource pattern is valid iff it is non-empty with at most one `*`, and
/// only as the final character.
pub fn is_valid_resource(resource: &str) -> bool {
    match resource.find('*') {
        None => !resource.is_empty(),
        Some(star) => star == resource.len() - 1,
    }
}

/// Requested resource against a statement resource: exact, or trailing-`*`
/// prefix match.
pub fn resource_matches(pattern: &str, requested: &str) -> bool {
    match pattern.strip_suffix('*') {
        Some(prefix) => requested.starts_with(prefix),
        None => pattern == requested,
    }
}

/// A statement action is valid iff the host-declared registry knows it. Any
/// `*` disqualifies it — actions never carry wildcards.
pub fn is_valid_action(action: &str, is_known_action: &dyn Fn(&str) -> bool) -> bool {
    !action.contains('*') && is_known_action(action)
}
