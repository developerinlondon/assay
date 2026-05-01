//! JSON-input argument resolution — literal / `@file` / `-` stdin.
//!
//! Saves shell-quoting pain on `--input`, `--search-attrs`, and signal
//! payloads. Shape matches `curl`'s `@file` convention so users who
//! already script against REST APIs don't have to learn a new idiom.

use std::io::Read;

use serde_json::Value;

/// Resolve a user-provided JSON argument to a `serde_json::Value`.
///
/// - `-`       read the whole of stdin and parse
/// - `@PATH`   read the file and parse
/// - anything else: parse directly
///
/// Returns an error message suitable for printing to stderr on failure.
pub fn resolve_json(raw: &str, what: &str) -> Result<Value, String> {
    if raw == "-" {
        let mut s = String::new();
        std::io::stdin()
            .read_to_string(&mut s)
            .map_err(|e| format!("{what}: reading stdin: {e}"))?;
        serde_json::from_str(&s).map_err(|e| format!("{what}: invalid JSON on stdin: {e}"))
    } else if let Some(path) = raw.strip_prefix('@') {
        let content = std::fs::read_to_string(path)
            .map_err(|e| format!("{what}: reading {path}: {e}"))?;
        serde_json::from_str(&content)
            .map_err(|e| format!("{what}: invalid JSON in {path}: {e}"))
    } else {
        serde_json::from_str(raw).map_err(|e| format!("{what}: invalid JSON: {e}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_literal_json() {
        let v = resolve_json(r#"{"a":1}"#, "x").unwrap();
        assert_eq!(v["a"], 1);
    }

    #[test]
    fn parses_file_json() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("input.json");
        std::fs::write(&path, r#"{"b":"y"}"#).unwrap();
        let arg = format!("@{}", path.display());
        let v = resolve_json(&arg, "x").unwrap();
        assert_eq!(v["b"], "y");
    }

    #[test]
    fn invalid_literal_returns_helpful_error() {
        let err = resolve_json("not json", "payload").unwrap_err();
        assert!(err.contains("payload: invalid JSON"));
    }

    #[test]
    fn missing_file_returns_helpful_error() {
        let err = resolve_json("@/nonexistent/file", "payload").unwrap_err();
        assert!(err.contains("reading"));
    }
}
