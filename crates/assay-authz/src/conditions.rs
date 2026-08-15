//! The pure ABAC condition engine: conditions narrow a statement that has
//! already matched by action and resource, and all of them must pass.
//!
//! Fail-closed is asymmetric by effect — an ALLOW contributes only on
//! `Match`, a DENY fires on `Match` and on `Unmatchable`. Both directions
//! therefore fail toward LESS access.

use serde_json::Value;

use crate::condition::{ConditionContext, ConditionKeys, ConditionOperator, ContextValue};
use crate::parse::{ip_in_cidr, is_valid_like_pattern, like_matches, parse_instant, parse_number};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConditionsVerdict {
    Match,
    NoMatch,
    Unmatchable,
}

struct CheckedCondition {
    operator: ConditionOperator,
    key: String,
    value: String,
    values: Option<Vec<String>>,
}

/// The pure tri-state evaluator. `conditions` is taken as raw JSON on
/// purpose: this is the LAST line of defense, so it re-validates shape even
/// though the write path already gated it. Every condition is shape-checked
/// BEFORE any is evaluated, so a malformed condition can never be masked by
/// an earlier false one.
pub fn eval_conditions(
    conditions: Option<&Value>,
    ctx: &ConditionContext,
    keys: &ConditionKeys,
) -> ConditionsVerdict {
    let Some(conditions) = conditions else {
        return ConditionsVerdict::Match;
    };
    if conditions.is_null() {
        return ConditionsVerdict::Match;
    }
    let Some(list) = conditions.as_array() else {
        return ConditionsVerdict::Unmatchable;
    };
    let mut checked = Vec::with_capacity(list.len());
    for condition in list {
        match check_shape(condition, keys) {
            Some(ok) => checked.push(ok),
            None => return ConditionsVerdict::Unmatchable,
        }
    }

    let mut all = true;
    for condition in &checked {
        let Some(raw) = ctx.get(&condition.key) else {
            return ConditionsVerdict::Unmatchable;
        };
        let lowercase = keys.get(&condition.key).is_some_and(|spec| spec.lowercase);
        let ctx_value = if lowercase {
            raw.lowercased()
        } else {
            raw.clone()
        };
        let outcome = match condition.values.as_ref() {
            Some(values) => {
                let bounds: Vec<String> = if lowercase {
                    values.iter().map(|v| v.to_lowercase()).collect()
                } else {
                    values.clone()
                };
                eval_one_set(condition.operator, &bounds, &ctx_value)
            }
            None => {
                let bound = if lowercase {
                    condition.value.to_lowercase()
                } else {
                    condition.value.clone()
                };
                eval_one(condition.operator, &bound, &ctx_value)
            }
        };
        match outcome {
            None => return ConditionsVerdict::Unmatchable,
            Some(false) => all = false,
            Some(true) => {}
        }
    }
    if all {
        ConditionsVerdict::Match
    } else {
        ConditionsVerdict::NoMatch
    }
}

/// Exactly one bound. A set operator carrying `value`, a scalar operator
/// carrying `values`, or a condition carrying both, is a document whose
/// intent cannot be read — refuse it rather than pick a side.
fn check_shape(condition: &Value, keys: &ConditionKeys) -> Option<CheckedCondition> {
    let object = condition.as_object()?;
    let operator = ConditionOperator::parse(object.get("operator")?.as_str()?)?;
    let key = object.get("key")?.as_str()?;
    let value = object.get("value");
    let values = object.get("values");

    let checked_values = if operator.is_set() {
        if value.is_some() {
            return None;
        }
        let items = values?.as_array()?;
        if items.is_empty() {
            return None;
        }
        let mut out = Vec::with_capacity(items.len());
        for item in items {
            let text = item.as_str()?;
            if text.is_empty() {
                return None;
            }
            out.push(text.to_string());
        }
        Some(out)
    } else {
        if values.is_some() {
            return None;
        }
        value?.as_str()?;
        None
    };

    let spec = keys.get(key)?;
    if operator.key_type() != spec.key_type {
        return None;
    }
    Some(CheckedCondition {
        operator,
        key: key.to_string(),
        value: value
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        values: checked_values,
    })
}

/// With a list on BOTH sides the semantic is a non-empty intersection, and
/// `StringNotIn` is its exact negation.
fn eval_one_set(
    operator: ConditionOperator,
    values: &[String],
    ctx_value: &ContextValue,
) -> Option<bool> {
    if values.is_empty() {
        return None;
    }
    let held = ctx_value.as_strings();
    match operator {
        ConditionOperator::StringIn | ConditionOperator::StringNotIn => {
            let hit = held.iter().any(|h| values.iter().any(|v| v == h));
            Some(if operator == ConditionOperator::StringIn {
                hit
            } else {
                !hit
            })
        }
        ConditionOperator::StringLikeIn => {
            if values.iter().any(|p| !is_valid_like_pattern(p)) {
                return None;
            }
            Some(
                held.iter()
                    .any(|h| values.iter().any(|pattern| like_matches(pattern, h))),
            )
        }
        _ => None,
    }
}

/// `None` when the condition cannot be evaluated at all — an unparseable
/// bound, an invalid like-pattern, a malformed CIDR, or a context value of
/// the wrong shape for the operator.
fn eval_one(operator: ConditionOperator, value: &str, ctx_value: &ContextValue) -> Option<bool> {
    match operator {
        ConditionOperator::StringEquals | ConditionOperator::StringNotEquals => {
            let equal = match ctx_value {
                ContextValue::List(items) => items.iter().any(|item| item == value),
                ContextValue::Text(text) => text == value,
                ContextValue::Number(_) => false,
            };
            Some(if operator == ConditionOperator::StringEquals {
                equal
            } else {
                !equal
            })
        }
        ConditionOperator::StringLike => {
            if !is_valid_like_pattern(value) {
                return None;
            }
            Some(match ctx_value {
                ContextValue::List(items) => items.iter().any(|item| like_matches(value, item)),
                other => other
                    .as_strings()
                    .iter()
                    .any(|item| like_matches(value, item)),
            })
        }
        ConditionOperator::NumericLessThan | ConditionOperator::NumericGreaterThan => {
            if value.trim().is_empty() {
                return None;
            }
            let bound = parse_number(value)?;
            let ContextValue::Number(at) = ctx_value else {
                return None;
            };
            Some(if operator == ConditionOperator::NumericLessThan {
                *at < bound
            } else {
                *at > bound
            })
        }
        ConditionOperator::DateLessThan | ConditionOperator::DateGreaterThan => {
            let ContextValue::Text(text) = ctx_value else {
                return None;
            };
            let bound = parse_instant(value)?;
            let at = parse_instant(text)?;
            Some(if operator == ConditionOperator::DateLessThan {
                at < bound
            } else {
                at > bound
            })
        }
        ConditionOperator::IpAddress | ConditionOperator::NotIpAddress => {
            let ContextValue::Text(text) = ctx_value else {
                return None;
            };
            let inside = ip_in_cidr(text, value)?;
            Some(if operator == ConditionOperator::IpAddress {
                inside
            } else {
                !inside
            })
        }
        _ => None,
    }
}
