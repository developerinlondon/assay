//! Dashboard whitelabel configuration.
//!
//! Lets operators rebrand the embedded `/workflow` dashboard per
//! deployment without forking the binary. Every knob is optional —
//! an unset env var falls back to assay's own identity, so the
//! standalone experience is unchanged.
//!
//! # Env vars
//!
//! | Variable                        | Purpose                                              | Default                      |
//! | ------------------------------- | ---------------------------------------------------- | ---------------------------- |
//! | `ASSAY_WHITELABEL_NAME`         | Text in the sidebar header + footer                  | `Assay`                      |
//! | `ASSAY_WHITELABEL_LOGO_URL`     | Image URL rendered before the brand text             | — (no image)                 |
//! | `ASSAY_WHITELABEL_PAGE_TITLE`   | Browser tab title                                    | `Assay Workflow Dashboard`   |
//! | `ASSAY_WHITELABEL_PARENT_URL`   | When set, adds a back-link in the sidebar footer     | — (hidden)                   |
//! | `ASSAY_WHITELABEL_PARENT_NAME`  | Label for the back-link                              | `Back`                       |
//! | `ASSAY_WHITELABEL_API_DOCS_URL` | Override (or hide) the API Docs sidebar link         | `/api/v1/docs`               |
//!
//! ## Hiding vs overriding
//!
//! For `ASSAY_WHITELABEL_API_DOCS_URL` we distinguish "unset" from
//! "set to empty string": an unset env var keeps the default link
//! pointing at the built-in OpenAPI UI, whereas explicitly setting it
//! to `""` hides the link entirely. This matters when an embedding
//! app's ingress doesn't route the OpenAPI path and you'd rather not
//! show a dead link.
//!
//! Hosting the logo: if assay is mounted on the same origin as the
//! embedding app (e.g. behind a reverse proxy at `/workflow/*`), a
//! path-absolute URL like `/static/my-logo.svg` loads from the host
//! app with no CORS plumbing.

use std::sync::LazyLock;

/// Shared config read once from env on first dashboard request. The
/// `LazyLock` ensures we never re-read env mid-flight if operators
/// restart without changing values, and matches the pattern used by
/// `ASSET_VERSION` in the dashboard module.
pub static WHITELABEL: LazyLock<WhitelabelConfig> = LazyLock::new(WhitelabelConfig::from_env);

/// Operator-configurable dashboard identity. Construct via
/// [`WhitelabelConfig::from_env`] in production; tests can build
/// instances directly.
#[derive(Debug, Clone)]
pub struct WhitelabelConfig {
    pub name: String,
    /// Single-letter glyph shown in the collapsed sidebar; derived
    /// from `name` unless overridden in future.
    pub mark: String,
    pub logo_url: Option<String>,
    pub page_title: String,
    pub parent_url: Option<String>,
    pub parent_name: String,
    /// `Some(url)` → render the link pointing at `url`.
    /// `None`      → hide the link entirely.
    pub api_docs_url: Option<String>,
}

impl WhitelabelConfig {
    /// Read every knob from env, applying defaults and the
    /// "set-to-empty means hide" convention for `api_docs_url`.
    pub fn from_env() -> Self {
        let name =
            std::env::var("ASSAY_WHITELABEL_NAME").unwrap_or_else(|_| "Assay".to_string());
        let mark = name
            .chars()
            .next()
            .map(|c| c.to_uppercase().to_string())
            .unwrap_or_else(|| "A".to_string());
        let logo_url = std::env::var("ASSAY_WHITELABEL_LOGO_URL")
            .ok()
            .filter(|s| !s.is_empty());
        let page_title = std::env::var("ASSAY_WHITELABEL_PAGE_TITLE")
            .unwrap_or_else(|_| "Assay Workflow Dashboard".to_string());
        let parent_url = std::env::var("ASSAY_WHITELABEL_PARENT_URL")
            .ok()
            .filter(|s| !s.is_empty());
        let parent_name =
            std::env::var("ASSAY_WHITELABEL_PARENT_NAME").unwrap_or_else(|_| "Back".to_string());
        let api_docs_url = match std::env::var("ASSAY_WHITELABEL_API_DOCS_URL") {
            Ok(s) if s.is_empty() => None,
            Ok(s) => Some(s),
            Err(_) => Some("/api/v1/docs".to_string()),
        };
        Self {
            name,
            mark,
            logo_url,
            page_title,
            parent_url,
            parent_name,
            api_docs_url,
        }
    }
}

/// Escape a value destined for HTML text or attributes. Every
/// whitelabel value comes from operator-controlled env, so if someone
/// puts a `"` or `<` in a brand name we don't want to break the page.
fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

/// Render the dashboard HTML template with the operator's whitelabel
/// values substituted in. The template contains placeholders like
/// `__BRAND_NAME__` / `__PARENT_BACK_LINK__` that this function fills
/// in (or replaces with empty strings for optional bits).
///
/// Separated from the HTTP handler so unit tests can assert the
/// substitutions without booting the server.
pub fn render_index(template: &str, asset_version: &str, wl: &WhitelabelConfig) -> String {
    let back_link = match &wl.parent_url {
        Some(url) => format!(
            r#"<a href="{}" class="nav-link nav-link-back" title="{}">
          <span class="nav-icon">&larr;</span> <span class="nav-label">{}</span>
        </a>"#,
            html_escape(url),
            html_escape(&wl.parent_name),
            html_escape(&wl.parent_name)
        ),
        None => String::new(),
    };

    let api_docs_link = match &wl.api_docs_url {
        Some(url) => format!(
            r#"<a href="{}" class="nav-link nav-link-grow" target="_blank">
          <span class="nav-icon">&#128196;</span> <span class="nav-label">API Docs</span>
        </a>"#,
            html_escape(url)
        ),
        None => String::new(),
    };

    let logo_img = match &wl.logo_url {
        Some(url) => format!(
            r#"<img class="logo-img" src="{}" alt="{}" />"#,
            html_escape(url),
            html_escape(&wl.name)
        ),
        None => String::new(),
    };

    template
        .replace("__ASSETV__", asset_version)
        .replace("__PAGE_TITLE__", &html_escape(&wl.page_title))
        .replace("__BRAND_NAME__", &html_escape(&wl.name))
        .replace("__BRAND_MARK__", &html_escape(&wl.mark))
        .replace("__BRAND_LOGO_IMG__", &logo_img)
        .replace("__PARENT_BACK_LINK__", &back_link)
        .replace("__API_DOCS_LINK__", &api_docs_link)
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEMPLATE: &str = r#"<title>__PAGE_TITLE__</title>
<span>__BRAND_NAME__</span><span>__BRAND_MARK__</span>
__BRAND_LOGO_IMG__
__PARENT_BACK_LINK__
__API_DOCS_LINK__
v=__ASSETV__"#;

    fn default_cfg() -> WhitelabelConfig {
        WhitelabelConfig {
            name: "Assay".into(),
            mark: "A".into(),
            logo_url: None,
            page_title: "Assay Workflow Dashboard".into(),
            parent_url: None,
            parent_name: "Back".into(),
            api_docs_url: Some("/api/v1/docs".into()),
        }
    }

    #[test]
    fn default_render_matches_standalone_identity() {
        let out = render_index(TEMPLATE, "42", &default_cfg());
        assert!(out.contains("<title>Assay Workflow Dashboard</title>"));
        assert!(out.contains("<span>Assay</span><span>A</span>"));
        // Optional bits are absent/empty by default.
        assert!(!out.contains("nav-link-back"));
        assert!(!out.contains("logo-img"));
        // Default API Docs link present and pointing at the engine path.
        assert!(out.contains("href=\"/api/v1/docs\""));
        // Asset version substitution still works.
        assert!(out.contains("v=42"));
    }

    #[test]
    fn whitelabel_name_and_mark_are_substituted() {
        let mut cfg = default_cfg();
        cfg.name = "Command Center".into();
        cfg.mark = "C".into();
        let out = render_index(TEMPLATE, "v", &cfg);
        assert!(out.contains("<span>Command Center</span><span>C</span>"));
    }

    #[test]
    fn logo_url_renders_img_tag() {
        let mut cfg = default_cfg();
        cfg.logo_url = Some("/static/siemens-logo.png".into());
        cfg.name = "CC".into();
        let out = render_index(TEMPLATE, "v", &cfg);
        assert!(out.contains(r#"src="/static/siemens-logo.png""#));
        assert!(out.contains(r#"alt="CC""#));
    }

    #[test]
    fn parent_url_renders_back_link() {
        let mut cfg = default_cfg();
        cfg.parent_url = Some("https://command.example/".into());
        cfg.parent_name = "Command Center".into();
        let out = render_index(TEMPLATE, "v", &cfg);
        assert!(out.contains(r#"href="https://command.example/""#));
        assert!(out.contains("Command Center"));
        assert!(out.contains("nav-link-back"));
    }

    #[test]
    fn api_docs_empty_hides_the_link() {
        let mut cfg = default_cfg();
        cfg.api_docs_url = None;
        let out = render_index(TEMPLATE, "v", &cfg);
        assert!(!out.contains("API Docs"));
        assert!(!out.contains("/api/v1/docs"));
    }

    #[test]
    fn api_docs_override_retargets_the_link() {
        let mut cfg = default_cfg();
        cfg.api_docs_url = Some("https://docs.example/api".into());
        let out = render_index(TEMPLATE, "v", &cfg);
        assert!(out.contains(r#"href="https://docs.example/api""#));
        assert!(out.contains("API Docs"));
    }

    #[test]
    fn html_in_brand_name_is_escaped() {
        let mut cfg = default_cfg();
        cfg.name = "Acme <Inc>".into();
        let out = render_index(TEMPLATE, "v", &cfg);
        assert!(out.contains("Acme &lt;Inc&gt;"));
        assert!(!out.contains("<Inc>"), "raw angle brackets must not land in the HTML");
    }
}
