use serde_json::Value;

pub fn redact_json_text(text: &str, keys: &[String]) -> Option<String> {
    if keys.is_empty() {
        return None;
    }
    let mut value: Value = serde_json::from_str(text).ok()?;
    let lowered: Vec<String> = keys.iter().map(|k| k.to_ascii_lowercase()).collect();
    strip(&mut value, &lowered);
    serde_json::to_string(&value).ok()
}

pub fn is_redacted_header(name: &str, keys: &[String]) -> bool {
    let name = name.to_ascii_lowercase();
    keys.iter().any(|k| k.to_ascii_lowercase() == name)
}

fn strip(value: &mut Value, keys: &[String]) {
    match value {
        Value::Object(map) => {
            for (_, v) in map.iter_mut() {
                strip(v, keys);
            }
            for key in map.keys().cloned().collect::<Vec<_>>() {
                if keys.contains(&key.to_ascii_lowercase()) {
                    map.insert(key, Value::String("[redacted]".to_string()));
                }
            }
        }
        Value::Array(items) => {
            for item in items.iter_mut() {
                strip(item, keys);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keys() -> Vec<String> {
        vec!["password".into(), "token".into()]
    }

    #[test]
    fn nested_and_arrayed_secrets_are_replaced_not_removed() {
        let out = redact_json_text(
            r#"{"user":{"name":"a","password":"hunter2"},"items":[{"token":"t"}]}"#,
            &keys(),
        )
        .expect("redacted");
        let parsed: Value = serde_json::from_str(&out).expect("valid json");
        assert_eq!(
            parsed["user"]["password"],
            Value::String("[redacted]".into())
        );
        assert_eq!(
            parsed["items"][0]["token"],
            Value::String("[redacted]".into())
        );
        assert_eq!(parsed["user"]["name"], Value::String("a".into()));
    }

    #[test]
    fn matching_is_case_insensitive() {
        let out = redact_json_text(r#"{"Password":"hunter2"}"#, &keys()).expect("redacted");
        assert!(!out.contains("hunter2"));
    }

    #[test]
    fn non_json_and_empty_keys_pass_through_untouched() {
        assert!(redact_json_text("not json at all", &keys()).is_none());
        assert!(redact_json_text(r#"{"password":"x"}"#, &[]).is_none());
    }

    #[test]
    fn header_names_match_case_insensitively() {
        assert!(is_redacted_header("X-Auth-Token", &["x-auth-token".into()]));
        assert!(!is_redacted_header("content-type", &keys()));
    }
}
