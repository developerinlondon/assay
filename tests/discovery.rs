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

// ---------------------------------------------------------------------------
// TDD RED: keyword gap tests — each should FAIL until keyword enrichment is added
// ---------------------------------------------------------------------------

#[test]
fn test_search_keyword_webhook_finds_http() {
    let results = search_modules("webhook", 5);
    let ids: Vec<&str> = results.iter().map(|r| r.id.as_str()).collect();
    assert!(
        ids.contains(&"http"),
        "search for 'webhook' should find http builtin, got: {ids:?}"
    );
}

#[test]
fn test_search_keyword_request_finds_http() {
    let results = search_modules("request", 5);
    let ids: Vec<&str> = results.iter().map(|r| r.id.as_str()).collect();
    assert!(
        ids.contains(&"http"),
        "search for 'request' should find http builtin, got: {ids:?}"
    );
}

#[test]
fn test_search_keyword_endpoint_finds_http() {
    let results = search_modules("endpoint", 5);
    let ids: Vec<&str> = results.iter().map(|r| r.id.as_str()).collect();
    assert!(
        ids.contains(&"http"),
        "search for 'endpoint' should find http builtin, got: {ids:?}"
    );
}

#[test]
fn test_search_keyword_letsencrypt_finds_certmanager() {
    let results = search_modules("letsencrypt", 5);
    let ids: Vec<&str> = results.iter().map(|r| r.id.as_str()).collect();
    assert!(
        ids.contains(&"assay.certmanager"),
        "search for 'letsencrypt' should find assay.certmanager, got: {ids:?}"
    );
}

#[test]
fn test_search_keyword_password_finds_vault() {
    let results = search_modules("password", 5);
    let ids: Vec<&str> = results.iter().map(|r| r.id.as_str()).collect();
    assert!(
        ids.contains(&"assay.vault"),
        "search for 'password' should find assay.vault, got: {ids:?}"
    );
}

#[test]
fn test_search_keyword_cicd_finds_argocd() {
    let results = search_modules("cicd", 5);
    let ids: Vec<&str> = results.iter().map(|r| r.id.as_str()).collect();
    assert!(
        ids.contains(&"assay.argocd") || ids.contains(&"assay.flux"),
        "search for 'cicd' should find assay.argocd or assay.flux, got: {ids:?}"
    );
}

#[test]
fn test_search_keyword_metric_finds_prometheus() {
    let results = search_modules("metric", 5);
    let ids: Vec<&str> = results.iter().map(|r| r.id.as_str()).collect();
    assert!(
        ids.contains(&"assay.prometheus"),
        "search for 'metric' should find assay.prometheus, got: {ids:?}"
    );
}

#[test]
fn test_search_keyword_terraform_finds_crossplane() {
    let results = search_modules("terraform", 5);
    let ids: Vec<&str> = results.iter().map(|r| r.id.as_str()).collect();
    assert!(
        ids.contains(&"assay.crossplane"),
        "search for 'terraform' should find assay.crossplane, got: {ids:?}"
    );
}

#[test]
fn test_search_keyword_rotation_finds_vault() {
    let results = search_modules("rotation", 5);
    let ids: Vec<&str> = results.iter().map(|r| r.id.as_str()).collect();
    assert!(
        ids.contains(&"assay.vault") || ids.contains(&"assay.certmanager"),
        "search for 'rotation' should find assay.vault or assay.certmanager, got: {ids:?}"
    );
}

#[test]
fn test_search_keyword_encryption_finds_vault() {
    let results = search_modules("encryption", 5);
    let ids: Vec<&str> = results.iter().map(|r| r.id.as_str()).collect();
    assert!(
        ids.contains(&"assay.vault"),
        "search for 'encryption' should find assay.vault, got: {ids:?}"
    );
}

#[test]
fn test_search_keyword_observability_finds_monitoring() {
    let results = search_modules("observability", 5);
    let ids: Vec<&str> = results.iter().map(|r| r.id.as_str()).collect();
    assert!(
        ids.contains(&"assay.prometheus") || ids.contains(&"assay.grafana"),
        "search for 'observability' should find assay.prometheus or assay.grafana, got: {ids:?}"
    );
}

#[test]
fn test_search_keyword_ssl_finds_certmanager() {
    let results = search_modules("ssl", 5);
    let ids: Vec<&str> = results.iter().map(|r| r.id.as_str()).collect();
    assert!(
        ids.contains(&"assay.certmanager"),
        "search for 'ssl' should find assay.certmanager, got: {ids:?}"
    );
}

#[test]
fn test_search_keyword_deploy_finds_k8s() {
    let results = search_modules("deploy", 5);
    let ids: Vec<&str> = results.iter().map(|r| r.id.as_str()).collect();
    assert!(
        ids.contains(&"assay.k8s"),
        "search for 'deploy' should find assay.k8s, got: {ids:?}"
    );
}

#[test]
fn test_search_keyword_failover_finds_velero() {
    let results = search_modules("failover", 5);
    let ids: Vec<&str> = results.iter().map(|r| r.id.as_str()).collect();
    assert!(
        ids.contains(&"assay.velero"),
        "search for 'failover' should find assay.velero, got: {ids:?}"
    );
}

#[test]
fn test_search_keyword_docker_finds_harbor() {
    let results = search_modules("docker", 5);
    let ids: Vec<&str> = results.iter().map(|r| r.id.as_str()).collect();
    assert!(
        ids.contains(&"assay.harbor"),
        "search for 'docker' should find assay.harbor, got: {ids:?}"
    );
}

// ---------------------------------------------------------------------------
// Regression tests — these MUST PASS with current code
// ---------------------------------------------------------------------------

#[test]
fn test_search_regression_grafana_is_first() {
    let results = search_modules("grafana", 5);
    assert!(!results.is_empty(), "search for 'grafana' should return results");
    assert_eq!(
        results[0].id, "assay.grafana",
        "search for 'grafana' should return assay.grafana first, got: {:?}",
        results.iter().map(|r| r.id.as_str()).collect::<Vec<_>>()
    );
}

#[test]
fn test_search_regression_vault_in_results() {
    let results = search_modules("vault", 5);
    let ids: Vec<&str> = results.iter().map(|r| r.id.as_str()).collect();
    assert!(
        ids.contains(&"assay.vault"),
        "search for 'vault' should find assay.vault in results, got: {ids:?}"
    );
}

#[test]
fn test_search_regression_prometheus_is_first() {
    let results = search_modules("prometheus", 5);
    assert!(!results.is_empty(), "search for 'prometheus' should return results");
    assert_eq!(
        results[0].id, "assay.prometheus",
        "search for 'prometheus' should return assay.prometheus first, got: {:?}",
        results.iter().map(|r| r.id.as_str()).collect::<Vec<_>>()
    );
}

#[test]
fn test_search_regression_kubernetes_finds_k8s() {
    let results = search_modules("kubernetes", 5);
    let ids: Vec<&str> = results.iter().map(|r| r.id.as_str()).collect();
    assert!(
        ids.contains(&"assay.k8s"),
        "search for 'kubernetes' should find assay.k8s, got: {ids:?}"
    );
}