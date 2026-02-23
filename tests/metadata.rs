use assay::metadata::parse_metadata;

#[test]
fn test_parse_full_metadata() {
    let source = r#"--- @module assay.grafana
--- @description Grafana monitoring and dashboards. Health, datasources, annotations.
--- @keywords grafana, monitoring, dashboards, datasources, annotations
--- @env GRAFANA_URL, GRAFANA_API_KEY
--- @quickref c:health() -> {database, version, commit} | Check Grafana health
--- @quickref c:datasources() -> [{id, name, type, url}] | List all datasources

local M = {}
function M.client(url, opts)
  local c = {}
  function c:health() end
  function c:datasources() end
  return c
end
return M
"#;
    let meta = parse_metadata(source);
    assert_eq!(meta.module_name, "assay.grafana");
    assert_eq!(
        meta.description,
        "Grafana monitoring and dashboards. Health, datasources, annotations."
    );
    assert_eq!(
        meta.keywords,
        vec![
            "grafana",
            "monitoring",
            "dashboards",
            "datasources",
            "annotations"
        ]
    );
    assert_eq!(meta.env_vars, vec!["GRAFANA_URL", "GRAFANA_API_KEY"]);
    assert_eq!(meta.quickrefs.len(), 2);
    assert_eq!(meta.quickrefs[0].signature, "c:health()");
    assert_eq!(meta.quickrefs[0].return_hint, "{database, version, commit}");
    assert_eq!(meta.quickrefs[0].description, "Check Grafana health");
    assert_eq!(meta.quickrefs[1].signature, "c:datasources()");
    assert_eq!(meta.quickrefs[1].return_hint, "[{id, name, type, url}]");
    assert_eq!(meta.quickrefs[1].description, "List all datasources");
}

#[test]
fn test_parse_quickref_format() {
    let source = "--- @quickref c:health() -> {database, version} | Check health\n\nlocal M = {}\n";
    let meta = parse_metadata(source);
    assert_eq!(meta.quickrefs.len(), 1);
    let qr = &meta.quickrefs[0];
    assert_eq!(qr.signature, "c:health()");
    assert_eq!(qr.return_hint, "{database, version}");
    assert_eq!(qr.description, "Check health");
}

#[test]
fn test_auto_extract_client_functions() {
    let source = r#"local M = {}
function M.client(url, opts)
  local c = {}
  function c:health()
    return "ok"
  end
  function c:datasources()
    return {}
  end
  return c
end
return M
"#;
    let meta = parse_metadata(source);
    assert!(
        meta.auto_functions.contains(&"health".to_string()),
        "auto_functions should contain 'health', got: {:?}",
        meta.auto_functions
    );
    assert!(
        meta.auto_functions.contains(&"datasources".to_string()),
        "auto_functions should contain 'datasources', got: {:?}",
        meta.auto_functions
    );
}

#[test]
fn test_auto_extract_module_functions() {
    let source = r#"local M = {}
function M.client(url, opts)
  return {}
end
function M.helper()
  return true
end
return M
"#;
    let meta = parse_metadata(source);
    assert!(
        meta.auto_functions.contains(&"client".to_string()),
        "auto_functions should contain 'client', got: {:?}",
        meta.auto_functions
    );
    assert!(
        meta.auto_functions.contains(&"helper".to_string()),
        "auto_functions should contain 'helper', got: {:?}",
        meta.auto_functions
    );
}

#[test]
fn test_graceful_empty_input() {
    let meta = parse_metadata("");
    assert_eq!(meta.module_name, "");
    assert_eq!(meta.description, "");
    assert!(meta.keywords.is_empty());
    assert!(meta.env_vars.is_empty());
    assert!(meta.quickrefs.is_empty());
    assert!(meta.auto_functions.is_empty());
}

#[test]
fn test_graceful_partial_headers() {
    let source =
        "--- @module assay.partial\n--- @description Only two tags\n\nlocal M = {}\nreturn M\n";
    let meta = parse_metadata(source);
    assert_eq!(meta.module_name, "assay.partial");
    assert_eq!(meta.description, "Only two tags");
    assert!(meta.keywords.is_empty());
    assert!(meta.env_vars.is_empty());
    assert!(meta.quickrefs.is_empty());
    assert!(meta.auto_functions.is_empty());
}

#[test]
fn test_keywords_split() {
    let source = "--- @keywords grafana, monitoring, dashboards\n\nlocal M = {}\n";
    let meta = parse_metadata(source);
    assert_eq!(meta.keywords, vec!["grafana", "monitoring", "dashboards"]);
}

#[test]
fn test_env_vars_split() {
    let source = "--- @env GRAFANA_URL, GRAFANA_API_KEY\n\nlocal M = {}\n";
    let meta = parse_metadata(source);
    assert_eq!(meta.env_vars, vec!["GRAFANA_URL", "GRAFANA_API_KEY"]);
}

#[test]
fn test_no_env_tag() {
    let source = "--- @module assay.noenv\n--- @description No env vars here\n\nlocal M = {}\n";
    let meta = parse_metadata(source);
    assert!(meta.env_vars.is_empty());
}

#[test]
fn test_stop_at_non_header_line() {
    let source = r#"--- @module assay.test
--- @description Top-level description

local M = {}

--- @module assay.fake
--- @description This should NOT be parsed as a tag
function M.client(url, opts)
  local c = {}
  function c:do_thing() end
  return c
end
return M
"#;
    let meta = parse_metadata(source);
    assert_eq!(meta.module_name, "assay.test");
    assert_eq!(meta.description, "Top-level description");
    // The @module in the function body must NOT override
    assert_ne!(meta.module_name, "assay.fake");
    // But auto_functions still scans the whole file
    assert!(
        meta.auto_functions.contains(&"client".to_string()),
        "auto_functions should contain 'client'"
    );
    assert!(
        meta.auto_functions.contains(&"do_thing".to_string()),
        "auto_functions should contain 'do_thing'"
    );
}
