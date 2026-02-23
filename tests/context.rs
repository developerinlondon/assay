use assay::context::{format_context, ModuleContextEntry, QuickRefEntry};

#[test]
fn test_format_single_module() {
    let entries = vec![ModuleContextEntry {
        module_name: "assay.grafana".to_string(),
        description: "Grafana monitoring and dashboards.".to_string(),
        env_vars: vec!["GRAFANA_URL".to_string(), "GRAFANA_API_KEY".to_string()],
        quickrefs: vec![
            QuickRefEntry {
                signature: "c:health()".to_string(),
                return_hint: "{database, version, commit}".to_string(),
                description: "Check Grafana health".to_string(),
            },
            QuickRefEntry {
                signature: "c:datasources()".to_string(),
                return_hint: "[{id, name, type, url}]".to_string(),
                description: "List all datasources".to_string(),
            },
        ],
    }];

    let output = format_context(&entries);

    assert!(output.contains("# Assay Module Context"));
    assert!(output.contains("### assay.grafana"));
    assert!(output.contains("Grafana monitoring and dashboards."));
    assert!(output.contains("Env: GRAFANA_URL, GRAFANA_API_KEY"));
    assert!(output.contains("c:health()"));
    assert!(output.contains("Check Grafana health"));
    assert!(output.contains("c:datasources()"));
    assert!(output.contains("List all datasources"));
}

#[test]
fn test_format_env_vars() {
    let entries = vec![ModuleContextEntry {
        module_name: "assay.vault".to_string(),
        description: "Vault secret management.".to_string(),
        env_vars: vec!["VAULT_ADDR".to_string(), "VAULT_TOKEN".to_string()],
        quickrefs: vec![],
    }];

    let output = format_context(&entries);

    assert!(output.contains("Env: VAULT_ADDR, VAULT_TOKEN"));
}

#[test]
fn test_format_no_env_vars() {
    let entries = vec![ModuleContextEntry {
        module_name: "assay.k8s".to_string(),
        description: "Kubernetes resources.".to_string(),
        env_vars: vec![],
        quickrefs: vec![],
    }];

    let output = format_context(&entries);

    assert!(output.contains("### assay.k8s"));
    assert!(!output.contains("Env:"));
}

#[test]
fn test_format_empty_results() {
    let entries: Vec<ModuleContextEntry> = vec![];

    let output = format_context(&entries);

    assert!(output.contains("# Assay Module Context"));
    assert!(output.contains("No matching modules found."));
}

#[test]
fn test_format_builtins_always_present() {
    let entries: Vec<ModuleContextEntry> = vec![];

    let output = format_context(&entries);

    assert!(output.contains("## Built-in Functions"));
    assert!(output.contains("http.get(url, opts?)"));
    assert!(output.contains("json.parse(str)"));
    assert!(output.contains("env.get(key)"));
}

#[test]
fn test_format_multiple_modules() {
    let entries = vec![
        ModuleContextEntry {
            module_name: "assay.grafana".to_string(),
            description: "Grafana monitoring.".to_string(),
            env_vars: vec!["GRAFANA_URL".to_string()],
            quickrefs: vec![],
        },
        ModuleContextEntry {
            module_name: "assay.prometheus".to_string(),
            description: "Prometheus metrics.".to_string(),
            env_vars: vec!["PROMETHEUS_URL".to_string()],
            quickrefs: vec![],
        },
    ];

    let output = format_context(&entries);

    assert!(output.contains("### assay.grafana"));
    assert!(output.contains("### assay.prometheus"));
    assert!(output.contains("Grafana monitoring."));
    assert!(output.contains("Prometheus metrics."));
}

#[test]
fn test_format_line_length() {
    let entries = vec![ModuleContextEntry {
        module_name: "assay.grafana".to_string(),
        description: "Grafana monitoring and dashboards.".to_string(),
        env_vars: vec!["GRAFANA_URL".to_string()],
        quickrefs: vec![QuickRefEntry {
            signature: "c:health()".to_string(),
            return_hint: "{database, version, commit}".to_string(),
            description: "Check Grafana health".to_string(),
        }],
    }];

    let output = format_context(&entries);

    // Check that most lines are under 120 chars (allow some flexibility for edge cases)
    for line in output.lines() {
        assert!(
            line.len() <= 130,
            "Line exceeds 130 chars: {} (len={})",
            line,
            line.len()
        );
    }
}
