use assay::discovery::{build_index, discover_modules, search_modules, ModuleSource};

use std::collections::HashSet;

#[test]
fn test_discover_modules_returns_builtins() {
    let modules = discover_modules();
    let names: Vec<&str> = modules.iter().map(|m| m.module_name.as_str()).collect();
    assert!(names.contains(&"http"), "should contain http builtin");
    assert!(names.contains(&"json"), "should contain json builtin");
    assert!(names.contains(&"log"), "should contain log builtin");
}

#[test]
fn test_discover_modules_returns_stdlib() {
    let modules = discover_modules();
    let names: Vec<&str> = modules.iter().map(|m| m.module_name.as_str()).collect();
    assert!(
        names.contains(&"assay.grafana"),
        "should contain assay.grafana stdlib, got: {names:?}"
    );
    assert!(
        names.contains(&"assay.vault"),
        "should contain assay.vault stdlib, got: {names:?}"
    );
}

#[test]
fn test_builtin_has_correct_source() {
    let modules = discover_modules();
    let http_mod = modules
        .iter()
        .find(|m| m.module_name == "http")
        .expect("http module should exist");
    assert_eq!(http_mod.source, ModuleSource::BuiltIn);
}

#[test]
fn test_stdlib_has_correct_source() {
    let modules = discover_modules();
    let grafana_mod = modules
        .iter()
        .find(|m| m.module_name == "assay.grafana")
        .expect("assay.grafana module should exist");
    assert_eq!(grafana_mod.source, ModuleSource::BuiltIn);
}

#[test]
fn test_build_index_returns_engine() {
    let modules = discover_modules();
    let index = build_index(&modules);
    // Verify it's a working SearchEngine by performing a search
    let results = index.search("grafana", 5);
    assert!(!results.is_empty(), "index search should return results");
}

#[test]
fn test_search_modules_finds_grafana() {
    let results = search_modules("grafana", 5);
    let ids: Vec<&str> = results.iter().map(|r| r.id.as_str()).collect();
    assert!(
        ids.contains(&"assay.grafana"),
        "search for 'grafana' should find assay.grafana, got: {ids:?}"
    );
}

#[test]
fn test_search_modules_finds_http() {
    let results = search_modules("http client", 5);
    let ids: Vec<&str> = results.iter().map(|r| r.id.as_str()).collect();
    assert!(
        ids.contains(&"http"),
        "search for 'http client' should find http, got: {ids:?}"
    );
}

#[test]
fn test_discover_modules_no_duplicates_by_name() {
    let modules = discover_modules();
    let mut seen = HashSet::new();
    for m in &modules {
        assert!(
            seen.insert(&m.module_name),
            "duplicate module name: {}",
            m.module_name
        );
    }
}

#[test]
fn test_search_modules_limit_respected() {
    let results = search_modules("a", 3);
    assert!(
        results.len() <= 3,
        "search with limit 3 should return at most 3 results, got {}",
        results.len()
    );
}
