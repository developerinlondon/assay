//! Write-time validation: the same whitelist the evaluator enforces, but with
//! named errors a UI can surface verbatim. A condition the evaluator could
//! never evaluate is refused here rather than stored dead-on-arrival.

use serde_json::Value;

use crate::condition::{ConditionKeySpec, ConditionKeys, ConditionOperator, PolicyCondition};
use crate::model::{Effect, Statement, is_valid_action, is_valid_resource};
use crate::parse::{
    has_explicit_timezone, is_valid_like_pattern, parse_cidr, parse_instant, parse_number,
};

pub struct Vocabulary<'a> {
    pub is_known_action: &'a dyn Fn(&str) -> bool,
    /// Already merged with the built-in `request:*` keys.
    pub condition_keys: &'a ConditionKeys,
    /// A host resource whitelist beyond the shape rule. `None` keeps every
    /// shape-valid resource.
    pub is_known_resource: Option<&'a dyn Fn(&str) -> bool>,
}

/// A non-empty list of well-formed statements, or the first offending piece.
pub fn validate_statements(
    input: &Value,
    vocabulary: &Vocabulary<'_>,
) -> Result<Vec<Statement>, String> {
    let Some(list) = input.as_array().filter(|list| !list.is_empty()) else {
        return Err("statements must be a non-empty array".into());
    };
    list.iter()
        .enumerate()
        .map(|(index, raw)| {
            validate_statement(raw, vocabulary)
                .map_err(|error| format!("statement {index}: {error}"))
        })
        .collect()
}

fn validate_statement(raw: &Value, vocabulary: &Vocabulary<'_>) -> Result<Statement, String> {
    let object = raw.as_object().ok_or("is not an object")?;
    let effect = match object.get("effect").and_then(Value::as_str) {
        Some("allow") => Effect::Allow,
        Some("deny") => Effect::Deny,
        _ => return Err("effect must be \"allow\" or \"deny\"".into()),
    };
    let actions =
        non_empty_strings(object.get("actions")).ok_or("actions must be a non-empty array")?;
    let resources =
        non_empty_strings(object.get("resources")).ok_or("resources must be a non-empty array")?;
    for action in &actions {
        if !is_valid_action(action, vocabulary.is_known_action) {
            return Err(format!("unknown or wildcard action \"{action}\""));
        }
    }
    for resource in &resources {
        if !is_valid_resource(resource) {
            return Err(format!(
                "resource \"{resource}\" — only a single trailing * is allowed"
            ));
        }
        if vocabulary
            .is_known_resource
            .is_some_and(|known| !known(resource))
        {
            return Err(format!("unknown resource \"{resource}\""));
        }
    }
    let conditions = match object.get("conditions") {
        None | Some(Value::Null) => None,
        Some(raw) => Some(
            serde_json::to_value(validate_conditions(raw, vocabulary.condition_keys)?)
                .map_err(|error| error.to_string())?,
        ),
    };
    Ok(Statement {
        effect,
        actions,
        resources,
        conditions,
    })
}

fn non_empty_strings(raw: Option<&Value>) -> Option<Vec<String>> {
    let list = raw?.as_array()?;
    if list.is_empty() {
        return None;
    }
    list.iter()
        .map(|item| item.as_str().map(str::to_string))
        .collect()
}

pub fn validate_conditions(
    input: &Value,
    keys: &ConditionKeys,
) -> Result<Vec<PolicyCondition>, String> {
    let Some(list) = input.as_array() else {
        return Err("conditions must be an array".into());
    };
    list.iter()
        .enumerate()
        .map(|(index, raw)| {
            validate_condition(raw, keys).map_err(|error| format!("condition {index}: {error}"))
        })
        .collect()
}

fn validate_condition(raw: &Value, keys: &ConditionKeys) -> Result<PolicyCondition, String> {
    let object = raw.as_object().ok_or("is not an object")?;
    let operator_name = object
        .get("operator")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let operator = ConditionOperator::parse(operator_name)
        .ok_or_else(|| format!("unknown operator \"{operator_name}\""))?;
    let key = object
        .get("key")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let spec = keys
        .get(key)
        .ok_or_else(|| format!("unknown key \"{key}\""))?;
    if operator.key_type() != spec.key_type {
        return Err(format!(
            "{} cannot test {key} (a {} key)",
            operator.as_str(),
            serde_json::to_value(spec.key_type)
                .ok()
                .and_then(|v| v.as_str().map(str::to_string))
                .unwrap_or_default()
        ));
    }
    let authored = Authored {
        object,
        operator,
        key,
        spec,
    };
    if operator.is_set() {
        return validate_set_condition(&authored);
    }
    validate_scalar_condition(&authored)
}

struct Authored<'a> {
    object: &'a serde_json::Map<String, Value>,
    operator: ConditionOperator,
    key: &'a str,
    spec: &'a ConditionKeySpec,
}

impl Authored<'_> {
    fn stored(&self, value: Option<String>, values: Option<Vec<String>>) -> PolicyCondition {
        PolicyCondition {
            operator: self.operator.as_str().to_string(),
            key: self.key.to_string(),
            value,
            values,
        }
    }

    fn normalize(&self, text: &str) -> String {
        if self.spec.lowercase {
            text.to_lowercase()
        } else {
            text.to_string()
        }
    }
}

fn validate_set_condition(authored: &Authored<'_>) -> Result<PolicyCondition, String> {
    let name = authored.operator.as_str();
    if authored.object.contains_key("value") {
        return Err(format!("{name} takes values, not value"));
    }
    let items = authored
        .object
        .get("values")
        .and_then(Value::as_array)
        .filter(|items| !items.is_empty())
        .ok_or_else(|| format!("{name} needs a non-empty values array"))?;
    let mut values = Vec::with_capacity(items.len());
    for item in items {
        let text = item
            .as_str()
            .filter(|text| !text.is_empty())
            .ok_or("every entry in values must be a non-empty string")?;
        if authored.operator == ConditionOperator::StringLikeIn {
            like_pattern_mistake(text)?;
        }
        values.push(authored.normalize(text));
    }
    Ok(authored.stored(None, Some(values)))
}

fn validate_scalar_condition(authored: &Authored<'_>) -> Result<PolicyCondition, String> {
    let operator = authored.operator;
    let name = operator.as_str();
    if authored.object.contains_key("values") {
        return Err(format!("{name} takes value, not values"));
    }
    let raw = authored.object.get("value");
    // A numeric bound authored as a JSON number is unambiguous — normalize it
    // rather than bounce the author to quoting it.
    let value = match (raw.and_then(Value::as_f64), operator) {
        (
            Some(number),
            ConditionOperator::NumericLessThan | ConditionOperator::NumericGreaterThan,
        ) if number.is_finite() => crate::condition::format_number(number),
        _ => raw
            .and_then(Value::as_str)
            .filter(|text| !text.is_empty())
            .ok_or("value must be a non-empty string")?
            .to_string(),
    };
    check_scalar_value(operator, &value)?;
    Ok(authored.stored(Some(authored.normalize(&value)), None))
}

fn check_scalar_value(operator: ConditionOperator, value: &str) -> Result<(), String> {
    match operator {
        ConditionOperator::NumericLessThan | ConditionOperator::NumericGreaterThan => {
            if value.trim().is_empty() || parse_number(value).is_none() {
                return Err(format!("\"{value}\" is not a number"));
            }
        }
        ConditionOperator::DateLessThan | ConditionOperator::DateGreaterThan => {
            if parse_instant(value).is_none() {
                return Err(format!("\"{value}\" is not a parseable date"));
            }
            if !has_explicit_timezone(value) {
                return Err(format!(
                    "\"{value}\" has no timezone — use Z or an explicit ±HH:MM offset (a bare \
                     timestamp parses in the server's local time)"
                ));
            }
        }
        ConditionOperator::StringLike => like_pattern_mistake(value)?,
        ConditionOperator::IpAddress | ConditionOperator::NotIpAddress
            if parse_cidr(value).is_none() =>
        {
            return Err(format!(
                "\"{value}\" is not a valid IPv4 or IPv6 CIDR (e.g. 10.0.0.0/8, 2001:db8::/32)"
            ));
        }
        _ => {}
    }
    Ok(())
}

/// A pattern in a regex or SQL-LIKE dialect matches literally here, which
/// reads as a policy and does nothing — refuse it with the fix in the message.
fn like_pattern_mistake(pattern: &str) -> Result<(), String> {
    if pattern.contains('|') {
        return Err(format!(
            "like-pattern \"{pattern}\" — \"|\" is not alternation here; use StringIn/StringLikeIn \
             with a values list"
        ));
    }
    if pattern.contains('%') {
        return Err(format!(
            "like-pattern \"{pattern}\" — \"%\" is not a wildcard here; the only wildcard is a \
             single trailing *"
        ));
    }
    if !is_valid_like_pattern(pattern) {
        return Err(format!(
            "like-pattern \"{pattern}\" — only a single trailing * is allowed"
        ));
    }
    Ok(())
}
