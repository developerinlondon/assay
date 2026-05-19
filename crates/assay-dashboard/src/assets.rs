//! Typed asset bundle for the workflow dashboard.
//!
//! Every asset is baked into the binary at compile time via `include_str!`.
//! Served by the router in `crate::router`.

pub const INDEX_HTML: &str = include_str!("../assets/workflow/index.html");
pub const THEME_CSS: &str = include_str!("../assets/workflow/theme.css");

// style.css is split across numbered section files for maintainability.
// Concat order is cascade order — filenames are numbered to make that obvious.
// Served as a single /workflow/style.css so the browser does one HTTP fetch.
pub const STYLE_CSS: &str = concat!(
    include_str!("../assets/workflow/styles/00-base.css"),
    include_str!("../assets/workflow/styles/10-sidebar.css"),
    include_str!("../assets/workflow/styles/11-status-bar.css"),
    include_str!("../assets/workflow/styles/20-workflow-rows.css"),
    include_str!("../assets/workflow/styles/21-tables.css"),
    include_str!("../assets/workflow/styles/30-detail-panel.css"),
    include_str!("../assets/workflow/styles/40-modal.css"),
    include_str!("../assets/workflow/styles/41-row-actions.css"),
    include_str!("../assets/workflow/styles/42-select.css"),
    include_str!("../assets/workflow/styles/43-links.css"),
    include_str!("../assets/workflow/styles/50-pipeline.css"),
    include_str!("../assets/workflow/styles/51-events.css"),
    include_str!("../assets/workflow/styles/60-buttons.css"),
    include_str!("../assets/workflow/styles/61-forms.css"),
    include_str!("../assets/workflow/styles/62-cards.css"),
    include_str!("../assets/workflow/styles/63-toolbar.css"),
    include_str!("../assets/workflow/styles/70-feedback.css"),
    include_str!("../assets/workflow/styles/71-toast.css"),
    include_str!("../assets/workflow/styles/80-mobile.css"),
);

pub const APP_JS: &str = include_str!("../assets/workflow/app.js");
pub const WORKFLOWS_JS: &str = include_str!("../assets/workflow/components/workflows.js");
pub const DETAIL_JS: &str = include_str!("../assets/workflow/components/detail.js");
pub const SCHEDULES_JS: &str = include_str!("../assets/workflow/components/schedules.js");
pub const WORKERS_JS: &str = include_str!("../assets/workflow/components/workers.js");
pub const QUEUES_JS: &str = include_str!("../assets/workflow/components/queues.js");
pub const SETTINGS_JS: &str = include_str!("../assets/workflow/components/settings.js");
pub const MODAL_JS: &str = include_str!("../assets/workflow/components/modal.js");
pub const ACTIONS_JS: &str = include_str!("../assets/workflow/components/actions.js");
pub const SELECT_JS: &str = include_str!("../assets/workflow/components/select.js");

/// Inline SVG favicon — single accent-coloured "A" mark on a dark surface.
pub const FAVICON_SVG: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 64 64"><rect width="64" height="64" rx="12" fill="#0d1117"/><text x="32" y="46" font-family="-apple-system,BlinkMacSystemFont,Segoe UI,Helvetica,Arial,sans-serif" font-size="44" font-weight="800" fill="#e6662a" text-anchor="middle">A</text></svg>"##;

// =====================================================================
//   Auth console assets (feature = "auth")
// =====================================================================

pub const AUTH_INDEX_HTML: &str = include_str!("../assets/auth/index.html");
pub const AUTH_STYLE_CSS: &str = include_str!("../assets/auth/style.css");
pub const AUTH_APP_JS: &str = include_str!("../assets/auth/app.js");
pub const AUTH_API_JS: &str = include_str!("../assets/auth/components/api.js");
pub const AUTH_USERS_JS: &str = include_str!("../assets/auth/components/users.js");
pub const AUTH_SESSIONS_JS: &str = include_str!("../assets/auth/components/sessions.js");
pub const AUTH_OIDC_CLIENTS_JS: &str = include_str!("../assets/auth/components/oidc_clients.js");
pub const AUTH_OIDC_UPSTREAM_JS: &str = include_str!("../assets/auth/components/oidc_upstream.js");
pub const AUTH_ZANZIBAR_JS: &str = include_str!("../assets/auth/components/zanzibar.js");
pub const AUTH_KEYS_JS: &str = include_str!("../assets/auth/components/keys.js");
pub const AUTH_AUDIT_JS: &str = include_str!("../assets/auth/components/audit.js");

// Public login landing. The engine merges this asset router at root,
// so `/auth/login` sits alongside the admin console under the same
// `/auth/*` namespace as the OIDC spec endpoints.
pub const AUTH_LOGIN_HTML: &str = include_str!("../assets/auth/login.html");
pub const AUTH_LOGIN_CSS: &str = include_str!("../assets/auth/login.css");
pub const AUTH_LOGIN_JS: &str = include_str!("../assets/auth/login.js");

/// Provider-icon sprite (single SVG with `<symbol id="slug">` per
/// well-known upstream IdP — Google, GitHub, GitLab, Microsoft, Apple,
/// Discord, Slack, plus a generic fallback). Sourced from Simple Icons
/// (CC0). Referenced by login.js via `<use href="/auth/icons.svg#slug"/>`.
pub const AUTH_ICONS_SVG: &str = include_str!("../assets/auth/icons.svg");

// =====================================================================
//   Engine console assets (always present — engine-core is always on)
// =====================================================================

pub const ENGINE_INDEX_HTML: &str = include_str!("../assets/engine/index.html");
pub const ENGINE_STYLE_CSS: &str = include_str!("../assets/engine/style.css");
pub const ENGINE_APP_JS: &str = include_str!("../assets/engine/app.js");
pub const ENGINE_API_JS: &str = include_str!("../assets/engine/components/api.js");
pub const ENGINE_INFO_JS: &str = include_str!("../assets/engine/components/info.js");
pub const ENGINE_MODULES_JS: &str = include_str!("../assets/engine/components/modules.js");
pub const ENGINE_INSTANCES_JS: &str = include_str!("../assets/engine/components/instances.js");
pub const ENGINE_AUDIT_JS: &str = include_str!("../assets/engine/components/audit.js");
pub const ENGINE_CONFIG_JS: &str = include_str!("../assets/engine/components/config.js");

// =====================================================================
//   Vault console assets (Phase 7 — gated by `vault` feature on the
//   engine; the asset bundle itself ships unconditionally so a console
//   asset can be link-ref'd from a slim engine without compile churn)
// =====================================================================

pub const VAULT_INDEX_HTML: &str = include_str!("../assets/vault/index.html");
pub const VAULT_STYLE_CSS: &str = include_str!("../assets/vault/style.css");
pub const VAULT_APP_JS: &str = include_str!("../assets/vault/app.js");

// =====================================================================
//   Shared cross-console nav strip — included by every console shell
// =====================================================================

pub const CROSS_NAV_CSS: &str = include_str!("../assets/shared/cross-nav.css");
pub const CROSS_NAV_JS: &str = include_str!("../assets/shared/cross-nav.js");
