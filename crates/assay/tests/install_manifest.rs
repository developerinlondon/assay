//! `assay install` Manifest.lua parser tests.
//!
//! Phase 2 of plan 21. Covers happy-path parsing, sandbox enforcement,
//! and decode-time validation (missing fields, unknown fields, wrong types).

use assay::install::manifest::{Manifest, ManifestError, parse};

const MIN_OK: &str = r#"
return {
  assay = "0.15.6",
  extensions = {
    { name = "assay-engine",
      version = "0.4.1",
      sha256 = { x86_64 = "aaaa", aarch64 = "bbbb" } },
  },
  libs = {
    { name = "hostops", version = "0.1.0", sha256 = "cccc" },
  },
}
"#;

#[test]
fn parses_full_happy_path_manifest() {
    let m = parse(MIN_OK, "Manifest.lua").expect("parse");
    assert_eq!(m.assay.as_deref(), Some("0.15.6"));

    assert_eq!(m.extensions.len(), 1);
    let e = &m.extensions[0];
    assert_eq!(e.name, "assay-engine");
    assert_eq!(e.version, "0.4.1");
    assert_eq!(e.sha256.get("x86_64").map(String::as_str), Some("aaaa"));
    assert_eq!(e.sha256.get("aarch64").map(String::as_str), Some("bbbb"));
    assert!(e.source.is_none());

    assert_eq!(m.libs.len(), 1);
    let l = &m.libs[0];
    assert_eq!(l.name, "hostops");
    assert_eq!(l.version, "0.1.0");
    assert_eq!(l.sha256, "cccc");
    assert!(l.source.is_none());
}

#[test]
fn parses_minimal_empty_manifest() {
    let m: Manifest = parse("return {}", "Manifest.lua").expect("parse");
    assert_eq!(m, Manifest::default());
}

#[test]
fn parses_only_libs_no_extensions() {
    let m = parse(
        r#"return { libs = { { name="x", version="1", sha256="abc" } } }"#,
        "Manifest.lua",
    )
    .unwrap();
    assert!(m.extensions.is_empty());
    assert_eq!(m.libs.len(), 1);
}

#[test]
fn captures_source_override_on_extension_and_lib() {
    let m = parse(
        r#"return {
            extensions = {{
                name = "x", version = "1",
                sha256 = { x86_64 = "aa" },
                source = "https://mirror.example/x-1-x86_64.tar.gz",
            }},
            libs = {{
                name = "y", version = "1", sha256 = "bb",
                source = "https://mirror.example/y-1.tar.gz",
            }},
        }"#,
        "Manifest.lua",
    )
    .unwrap();
    assert_eq!(
        m.extensions[0].source.as_deref(),
        Some("https://mirror.example/x-1-x86_64.tar.gz")
    );
    assert_eq!(
        m.libs[0].source.as_deref(),
        Some("https://mirror.example/y-1.tar.gz")
    );
}

// --- sandbox tests ----------------------------------------------------

#[test]
fn rejects_os_execute_call() {
    let err = parse(
        r#"os.execute("rm -rf /")
           return {}"#,
        "Manifest.lua",
    )
    .unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("nil value") && msg.contains("os"),
        "expected nil-os error, got: {msg}"
    );
}

#[test]
fn rejects_io_open_call() {
    let err = parse(
        r#"io.open("/etc/passwd", "r")
           return {}"#,
        "Manifest.lua",
    )
    .unwrap_err();
    assert!(
        err.to_string().contains("nil value") && err.to_string().contains("io"),
        "expected nil-io error, got: {err}"
    );
}

#[test]
fn rejects_require_call() {
    let err = parse(
        r#"require("evil")
           return {}"#,
        "Manifest.lua",
    )
    .unwrap_err();
    assert!(
        err.to_string().contains("nil value") || err.to_string().contains("require"),
        "expected require to be nilled, got: {err}"
    );
}

#[test]
fn rejects_load_call() {
    let err = parse(
        r#"load("return os.execute('whoami')")()
           return {}"#,
        "Manifest.lua",
    )
    .unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("nil value") || msg.contains("load"),
        "expected load to be nilled, got: {msg}"
    );
}

// --- decode-time validation -------------------------------------------

#[test]
fn errors_when_top_level_is_not_a_table() {
    let err = parse(r#"return 42"#, "Manifest.lua").unwrap_err();
    matches!(err, ManifestError::NotATable { .. });
}

#[test]
fn errors_when_extension_missing_required_name() {
    let err = parse(
        r#"return { extensions = { { version="1", sha256={x86_64="a"} } } }"#,
        "Manifest.lua",
    )
    .unwrap_err();
    let msg = err.to_string();
    assert!(
        matches!(err, ManifestError::Decode { .. }),
        "expected Decode error, got: {msg}"
    );
    assert!(
        msg.contains("name"),
        "expected error to mention `name`, got: {msg}"
    );
}

#[test]
fn errors_when_extension_missing_sha256() {
    let err = parse(
        r#"return { extensions = { { name="x", version="1" } } }"#,
        "Manifest.lua",
    )
    .unwrap_err();
    let msg = err.to_string();
    assert!(matches!(err, ManifestError::Decode { .. }));
    assert!(
        msg.contains("sha256"),
        "expected error to mention `sha256`, got: {msg}"
    );
}

#[test]
fn errors_when_lib_missing_sha256() {
    let err = parse(
        r#"return { libs = { { name="x", version="1" } } }"#,
        "Manifest.lua",
    )
    .unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("sha256"));
}

#[test]
fn errors_on_unknown_field_in_extension() {
    let err = parse(
        r#"return { extensions = { {
            name="x", version="1",
            sha256={x86_64="a"},
            invalid_field = "boom",
        } } }"#,
        "Manifest.lua",
    )
    .unwrap_err();
    let msg = err.to_string();
    assert!(
        matches!(err, ManifestError::Decode { .. }),
        "expected Decode error, got: {msg}"
    );
    assert!(
        msg.contains("invalid_field") || msg.contains("unknown"),
        "expected unknown-field error, got: {msg}"
    );
}

#[test]
fn errors_on_wrong_type_for_field() {
    let err = parse(
        r#"return { extensions = { { name=42, version="1", sha256={x86_64="a"} } } }"#,
        "Manifest.lua",
    )
    .unwrap_err();
    assert!(matches!(err, ManifestError::Decode { .. }));
}

#[test]
fn allows_empty_sha256_table_decodes_to_empty_map() {
    // Note: structural validation (i.e. "must have at least one arch")
    // happens at install time, not parse time, so an empty sha256 table
    // decodes successfully here.
    let m = parse(
        r#"return { extensions = { { name="x", version="1", sha256={} } } }"#,
        "Manifest.lua",
    )
    .unwrap();
    assert!(m.extensions[0].sha256.is_empty());
}
