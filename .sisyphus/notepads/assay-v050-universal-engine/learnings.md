# Learnings

## 2026-02-23 Session Start

### Codebase State (v0.4.5 baseline)

- `src/main.rs`: 145 lines. Single `Cli` struct with positional `file: PathBuf`. Extension-based
  dispatch (yaml→YAML checks, lua→script). `run_yaml_checks()` and `run_lua_script()` are standalone
  async fns.
- `src/lib.rs`: 2 lines — only `pub mod lua;`
- `stdlib/`: 23 Lua files. None have LDoc headers yet. Pattern: `local M = {}`, then
  `M.client(url, opts)`, then `local function api_get/post/put/delete(self, path)`, then
  `function c:method()` client methods.
- `grafana.lua`: 110 lines, 10 client methods: health, datasources, datasource, search, dashboard,
  annotations, create_annotation, org, alert_rules, folders. Auth: api_key OR username+password.
- Cargo.toml: features `default = ["db", "server", "cli"]`. sqlx with sqlite feature already
  present.

### Key Conventions

- Error format: `"<module>: <METHOD> <path> HTTP <status>: <body>"`
- Strip trailing slashes: `url:gsub("/+$", "")`
- 404 = nil pattern: check `resp.status == 404`, return `nil`
- Tests use wiremock: `MockServer::start().await`,
  `Mock::given(method("GET")).and(path("/...")).respond_with(...).mount(&server).await`
- Test helper: `common::run_lua(script)` from `tests/common/mod.rs`

### LDoc Header Format

```lua
--- @module assay.grafana
--- @description Grafana monitoring and dashboards. Health, datasources, annotations, alerts, folders.
--- @keywords grafana, monitoring, dashboards, datasources, annotations, alerts, health
--- @env GRAFANA_URL, GRAFANA_API_KEY
--- @quickref c:health() -> {database, version, commit} | Check Grafana health
--- @quickref c:datasources() -> [{id, name, type, url}] | List all datasources
```

Rules: triple-dash `--- @`, header at TOP before `local M = {}`, @quickref only for
`function c:method()` (not local helpers).
