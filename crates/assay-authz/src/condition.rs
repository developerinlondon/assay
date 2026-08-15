//! The closed condition vocabulary: operators, key types, and the attribute
//! bag conditions test. A condition is never free expression code.

use std::collections::BTreeMap;

use chrono::{DateTime, SecondsFormat, Timelike, Utc};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ConditionOperator {
    StringEquals,
    StringNotEquals,
    StringLike,
    StringIn,
    StringNotIn,
    StringLikeIn,
    NumericLessThan,
    NumericGreaterThan,
    DateLessThan,
    DateGreaterThan,
    IpAddress,
    NotIpAddress,
}

pub const CONDITION_OPERATORS: [ConditionOperator; 12] = [
    ConditionOperator::StringEquals,
    ConditionOperator::StringNotEquals,
    ConditionOperator::StringLike,
    ConditionOperator::StringIn,
    ConditionOperator::StringNotIn,
    ConditionOperator::StringLikeIn,
    ConditionOperator::NumericLessThan,
    ConditionOperator::NumericGreaterThan,
    ConditionOperator::DateLessThan,
    ConditionOperator::DateGreaterThan,
    ConditionOperator::IpAddress,
    ConditionOperator::NotIpAddress,
];

/// Operators whose bound is a list, so an allowlist is one condition rather
/// than one statement per permitted value.
pub const SET_OPERATORS: [ConditionOperator; 3] = [
    ConditionOperator::StringIn,
    ConditionOperator::StringNotIn,
    ConditionOperator::StringLikeIn,
];

impl ConditionOperator {
    pub fn parse(name: &str) -> Option<Self> {
        CONDITION_OPERATORS
            .into_iter()
            .find(|op| op.as_str() == name)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::StringEquals => "StringEquals",
            Self::StringNotEquals => "StringNotEquals",
            Self::StringLike => "StringLike",
            Self::StringIn => "StringIn",
            Self::StringNotIn => "StringNotIn",
            Self::StringLikeIn => "StringLikeIn",
            Self::NumericLessThan => "NumericLessThan",
            Self::NumericGreaterThan => "NumericGreaterThan",
            Self::DateLessThan => "DateLessThan",
            Self::DateGreaterThan => "DateGreaterThan",
            Self::IpAddress => "IpAddress",
            Self::NotIpAddress => "NotIpAddress",
        }
    }

    pub fn key_type(self) -> ConditionKeyType {
        match self {
            Self::StringEquals
            | Self::StringNotEquals
            | Self::StringLike
            | Self::StringIn
            | Self::StringNotIn
            | Self::StringLikeIn => ConditionKeyType::String,
            Self::NumericLessThan | Self::NumericGreaterThan => ConditionKeyType::Number,
            Self::DateLessThan | Self::DateGreaterThan => ConditionKeyType::Date,
            Self::IpAddress | Self::NotIpAddress => ConditionKeyType::Ip,
        }
    }

    pub fn is_set(self) -> bool {
        SET_OPERATORS.contains(&self)
    }
}

/// A validated condition as it is stored: exactly one bound is ever set.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PolicyCondition {
    pub operator: String,
    pub key: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub values: Option<Vec<String>>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ConditionKeyType {
    #[default]
    String,
    Number,
    Date,
    Ip,
}

/// A host-declared condition key. `lowercase` marks a key whose values are
/// compared case-insensitively on BOTH sides, so a mixed-case authored value
/// can never fail OPEN.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ConditionKeySpec {
    #[serde(rename = "type")]
    pub key_type: ConditionKeyType,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub lowercase: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// key to spec. A closed map: adding a key REQUIRES stating its type.
pub type ConditionKeys = BTreeMap<String, ConditionKeySpec>;

/// A multi-valued key (a principal holding several roles) carries a list;
/// every other key is a scalar.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ContextValue {
    Number(f64),
    List(Vec<String>),
    Text(String),
}

impl ContextValue {
    pub fn as_strings(&self) -> Vec<String> {
        match self {
            Self::List(items) => items.clone(),
            Self::Text(s) => vec![s.clone()],
            Self::Number(n) => vec![format_number(*n)],
        }
    }

    pub fn lowercased(&self) -> Self {
        match self {
            Self::Text(s) => Self::Text(s.to_lowercase()),
            Self::List(items) => Self::List(items.iter().map(|v| v.to_lowercase()).collect()),
            Self::Number(n) => Self::Number(*n),
        }
    }
}

/// The attribute bag conditions test — built from real check inputs only,
/// never from a caller-supplied bag.
pub type ConditionContext = BTreeMap<String, ContextValue>;

pub const BUILTIN_TIME_KEY: &str = "request:Time";
pub const BUILTIN_HOUR_KEY: &str = "request:HourUTC";
pub const BUILTIN_SOURCE_IP_KEY: &str = "request:SourceIp";

/// The keys the engine populates itself on every check. A key with no honest
/// value is left UNPOPULATED rather than faked, so a condition on it is
/// unmatchable: an allow never matches and a deny fires.
pub fn builtin_condition_keys() -> ConditionKeys {
    ConditionKeys::from([
        (
            BUILTIN_TIME_KEY.to_string(),
            ConditionKeySpec {
                key_type: ConditionKeyType::Date,
                title: Some("Before a date".into()),
                description: Some("The instant the check is evaluated, RFC 3339.".into()),
                ..Default::default()
            },
        ),
        (
            BUILTIN_HOUR_KEY.to_string(),
            ConditionKeySpec {
                key_type: ConditionKeyType::Number,
                title: Some("Hour of day (UTC)".into()),
                description: Some("0-23 on the evaluation clock.".into()),
                ..Default::default()
            },
        ),
        (
            BUILTIN_SOURCE_IP_KEY.to_string(),
            ConditionKeySpec {
                key_type: ConditionKeyType::Ip,
                title: Some("Source IP".into()),
                description: Some("The caller's network address, matched by CIDR.".into()),
                ..Default::default()
            },
        ),
    ])
}

/// Host keys merged with the built-ins; the built-ins always win so a host
/// declaration can never redefine what `request:*` means.
pub fn resolve_condition_keys(host_keys: &ConditionKeys) -> ConditionKeys {
    let mut merged = host_keys.clone();
    merged.extend(builtin_condition_keys());
    merged
}

/// The built-in entries for one evaluation instant. `source_ip` populates its
/// key only when non-empty.
pub fn builtin_context_entries(
    now: DateTime<Utc>,
    source_ip: Option<&str>,
) -> Vec<(String, ContextValue)> {
    let mut entries = vec![
        (
            BUILTIN_TIME_KEY.to_string(),
            ContextValue::Text(now.to_rfc3339_opts(SecondsFormat::Millis, true)),
        ),
        (
            BUILTIN_HOUR_KEY.to_string(),
            ContextValue::Number(f64::from(now.hour())),
        ),
    ];
    if let Some(ip) = source_ip.filter(|ip| !ip.is_empty()) {
        entries.push((
            BUILTIN_SOURCE_IP_KEY.to_string(),
            ContextValue::Text(ip.to_string()),
        ));
    }
    entries
}

/// Numbers reach string comparisons through the same rendering the reference
/// engine uses, so an integral value never grows a `.0` tail.
pub fn format_number(value: f64) -> String {
    if value.is_finite() && value.fract() == 0.0 && value.abs() < 1e21 {
        format!("{}", value as i128)
    } else {
        format!("{value}")
    }
}
