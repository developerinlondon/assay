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
//! | `ASSAY_WHITELABEL_CSS_URL`      | Extra stylesheet loaded after assay's own CSS        | — (none)                     |
//! | `ASSAY_WHITELABEL_SUBTITLE`     | Small muted line shown under the brand name          | — (none)                     |
//! | `ASSAY_WHITELABEL_MARK`         | Single glyph for the always-visible badge square     | First char of NAME (upper)   |
//! | `ASSAY_WHITELABEL_FAVICON_URL`  | Replace the browser-tab icon                         | Built-in `A`-mark SVG        |
//! | `ASSAY_WHITELABEL_DEFAULT_NAMESPACE` | Namespace the dashboard opens on                | `main`                       |
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
//!
//! ## Theming via `ASSAY_WHITELABEL_CSS_URL`
//!
//! Assay's dashboard styles are token-driven — every colour, radius,
//! and shadow is a CSS custom property on `:root`. An extra stylesheet
//! loaded at the end of `<head>` can therefore override any design
//! token without touching the assay source:
//!
//! ```css
//! :root {
//!   --bg:      hsl(0 0% 98%);
//!   --surface: hsl(0 0% 100%);
//!   --accent:  #009999;
//!   --accent-hover: #007a7a;
//!   --text:    hsl(222 84% 5%);
//!   --border:  hsl(214 32% 91%);
//! }
//! ```
//!
//! Tokens documented in `docs/modules/workflow.md#dashboard-whitelabel`.
//! Operators can additionally override any assay selector in the same
//! file — specificity + source order ensures the later sheet wins.

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
    /// Single-letter (or short) glyph rendered inside the always-visible
    /// badge square at the top of the sidebar. Derived from the first
    /// character of `name` unless `ASSAY_WHITELABEL_MARK` overrides.
    pub mark: String,
    /// Muted subtitle rendered beneath the brand name (empty → no
    /// subtitle line). Gives operators the canonical two-line brand
    /// block without needing a custom logo SVG.
    pub subtitle: String,
    pub logo_url: Option<String>,
    pub page_title: String,
    pub parent_url: Option<String>,
    pub parent_name: String,
    /// `Some(url)` → render the link pointing at `url`.
    /// `None`      → hide the link entirely.
    pub api_docs_url: Option<String>,
    /// Optional stylesheet URL loaded after assay's own CSS. Operators
    /// use this to re-skin the dashboard by overriding CSS custom
    /// properties or specific selectors without touching the source.
    pub css_url: Option<String>,
    /// Optional favicon URL. When `None` the built-in SVG `A`-mark is
    /// served at `/workflow/favicon.svg`; when `Some(url)` the template
    /// points `<link rel="icon">` at the operator's URL instead.
    pub favicon_url: Option<String>,
    /// Namespace the dashboard opens on by default. Operators running
    /// assay single-tenant (all workflows in one non-`main` namespace)
    /// shouldn't force every user to change the dropdown on first load.
    pub default_namespace: String,
}

impl WhitelabelConfig {
    /// `true` when any operator-facing identity field has been overridden
    /// from the default. Drives the "Powered by …" prefix in the footer
    /// version line — attribution without burying the engine on the
    /// standalone dashboard.
    pub fn is_customised(&self) -> bool {
        self.name != "Assay"
            || !self.subtitle.is_empty()
            || self.logo_url.is_some()
            || self.css_url.is_some()
            || self.favicon_url.is_some()
    }
}

impl WhitelabelConfig {
    /// Read every knob from env, applying defaults and the
    /// "set-to-empty means hide" convention for `api_docs_url`.
    pub fn from_env() -> Self {
        let name =
            std::env::var("ASSAY_WHITELABEL_NAME").unwrap_or_else(|_| "Assay".to_string());
        // MARK falls back to the first character of NAME (uppercased) so
        // operators who only set NAME get a sensible badge glyph
        // automatically. Explicit override handles cases where the name
        // doesn't begin with the intended letter (e.g. NAME="Acme Inc",
        // MARK="A" is the auto-default anyway; NAME="SIMONS Command
        // Center", MARK="S" is fine; NAME="The Platform", MARK="P"
        // needs the override to drop the article).
        let mark = std::env::var("ASSAY_WHITELABEL_MARK")
            .ok()
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| {
                name.chars()
                    .next()
                    .map(|c| c.to_uppercase().to_string())
                    .unwrap_or_else(|| "A".to_string())
            });
        let subtitle =
            std::env::var("ASSAY_WHITELABEL_SUBTITLE").unwrap_or_default();
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
        let css_url = std::env::var("ASSAY_WHITELABEL_CSS_URL")
            .ok()
            .filter(|s| !s.is_empty());
        let favicon_url = std::env::var("ASSAY_WHITELABEL_FAVICON_URL")
            .ok()
            .filter(|s| !s.is_empty());
        let default_namespace = std::env::var("ASSAY_WHITELABEL_DEFAULT_NAMESPACE")
            .ok()
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "main".to_string());
        Self {
            name,
            mark,
            subtitle,
            logo_url,
            page_title,
            parent_url,
            parent_name,
            api_docs_url,
            css_url,
            favicon_url,
            default_namespace,
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

    // Subtitle is rendered as a distinct span under the brand name so
    // operators get a real, accessible, translatable line of text —
    // not a baked-into-SVG image.
    let subtitle = if wl.subtitle.is_empty() {
        String::new()
    } else {
        format!(
            r#"<span class="logo-subtitle">{}</span>"#,
            html_escape(&wl.subtitle)
        )
    };

    // Footer version-line: vanilla dashboards keep the current
    // "Assay Workflow Engine vX.Y.Z" wording. Whitelabel deployments
    // get a "Powered by Assay vX.Y.Z" line — short, not redundant with
    // a subtitle that may already say "Workflow Engine", still links
    // to assay.rs for discovery. The version span is populated by the
    // existing dashboard JS in both variants.
    let engine_footer = if wl.is_customised() {
        r#"Powered by <a class="assay-attribution" href="https://assay.rs" target="_blank" rel="noopener noreferrer">Assay</a> <span id="status-version">—</span>"#.to_string()
    } else {
        r#"Assay Workflow Engine <span id="status-version">—</span>"#.to_string()
    };

    // Favicon — operator-supplied URL when set, otherwise assay's own
    // inline SVG served from /workflow/favicon.svg. Emitted as a
    // full <link> tag so the operator URL can be absolute, a path,
    // or a data: URI without the template having to know.
    let favicon_link = match &wl.favicon_url {
        Some(url) => format!(
            r#"<link rel="icon" href="{}">"#,
            html_escape(url)
        ),
        None => r#"<link rel="icon" type="image/svg+xml" href="/workflow/favicon.svg">"#.to_string(),
    };

    // Default namespace — threaded into the template as a data-attribute
    // the dashboard JS picks up on first load. Operators running a
    // single-tenant assay-as-a-product shouldn't force every user to
    // change the namespace dropdown on first visit.
    let default_namespace_attr = format!(
        r#" data-default-namespace="{}""#,
        html_escape(&wl.default_namespace)
    );

    // Emitted at the end of <head>, after assay's own theme.css + style.css,
    // so operator overrides win on source order + specificity. Asset-version
    // is appended to the URL so a redeploy that changes the stylesheet
    // forces a browser re-fetch (same pattern as assay's own assets).
    let extra_css = match &wl.css_url {
        Some(url) => {
            let sep = if url.contains('?') { '&' } else { '?' };
            format!(
                r#"<link rel="stylesheet" href="{}{}v={}">"#,
                html_escape(url),
                sep,
                asset_version
            )
        }
        None => String::new(),
    };

    template
        .replace("__ASSETV__", asset_version)
        .replace("__PAGE_TITLE__", &html_escape(&wl.page_title))
        .replace("__BRAND_NAME__", &html_escape(&wl.name))
        .replace("__BRAND_MARK__", &html_escape(&wl.mark))
        .replace("__BRAND_LOGO_IMG__", &logo_img)
        .replace("__BRAND_SUBTITLE__", &subtitle)
        .replace("__ENGINE_FOOTER__", &engine_footer)
        .replace("__PARENT_BACK_LINK__", &back_link)
        .replace("__API_DOCS_LINK__", &api_docs_link)
        .replace("__EXTRA_CSS_LINK__", &extra_css)
        .replace("__FAVICON_LINK__", &favicon_link)
        .replace("__DEFAULT_NAMESPACE_ATTR__", &default_namespace_attr)
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEMPLATE: &str = r#"<title>__PAGE_TITLE__</title>
<head>__FAVICON_LINK__ __EXTRA_CSS_LINK__</head>
<body__DEFAULT_NAMESPACE_ATTR__>
<span>__BRAND_NAME__</span><span>__BRAND_MARK__</span>
__BRAND_SUBTITLE__
__BRAND_LOGO_IMG__
__PARENT_BACK_LINK__
__API_DOCS_LINK__
<footer>__ENGINE_FOOTER__</footer>
v=__ASSETV__
</body>"#;

    fn default_cfg() -> WhitelabelConfig {
        WhitelabelConfig {
            name: "Assay".into(),
            mark: "A".into(),
            subtitle: String::new(),
            logo_url: None,
            page_title: "Assay Workflow Dashboard".into(),
            parent_url: None,
            parent_name: "Back".into(),
            api_docs_url: Some("/api/v1/docs".into()),
            css_url: None,
            favicon_url: None,
            default_namespace: "main".into(),
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
        assert!(!out.contains("logo-subtitle"));
        // Vanilla footer — no "Powered by" prefix, no attribution link.
        assert!(out.contains("Assay Workflow Engine"));
        assert!(!out.contains("Powered by"));
        assert!(!out.contains("assay-attribution"));
        // Default API Docs link present and pointing at the engine path.
        assert!(out.contains("href=\"/api/v1/docs\""));
        // Default favicon points at the built-in SVG.
        assert!(out.contains(r#"href="/workflow/favicon.svg""#));
        // Default namespace attr exposes "main" for the dashboard JS.
        assert!(out.contains(r#"data-default-namespace="main""#));
        // Asset version substitution still works.
        assert!(out.contains("v=42"));
    }

    #[test]
    fn subtitle_renders_as_muted_span_when_set() {
        let mut cfg = default_cfg();
        cfg.name = "SIMONS".into();
        cfg.subtitle = "Command Center".into();
        let out = render_index(TEMPLATE, "v", &cfg);
        assert!(out.contains(r#"<span class="logo-subtitle">Command Center</span>"#));
    }

    #[test]
    fn subtitle_unset_emits_nothing() {
        let out = render_index(TEMPLATE, "v", &default_cfg());
        assert!(!out.contains("logo-subtitle"));
    }

    #[test]
    fn mark_override_wins_over_name_initial() {
        let mut cfg = default_cfg();
        cfg.name = "The Platform".into();
        cfg.mark = "P".into();
        let out = render_index(TEMPLATE, "v", &cfg);
        // Mark is rendered explicitly, not auto-derived.
        assert!(out.contains("<span>The Platform</span><span>P</span>"));
    }

    #[test]
    fn whitelabel_footer_includes_powered_by_and_attribution_link() {
        // Any customised identity flips the footer to the attributed
        // variant — a simple NAME change is enough.
        let mut cfg = default_cfg();
        cfg.name = "Acme Workflows".into();
        cfg.mark = "A".into();
        let out = render_index(TEMPLATE, "v", &cfg);
        assert!(out.contains("Powered by"));
        assert!(out.contains(r#">Assay</a>"#), "short 'Assay' attribution text");
        assert!(!out.contains("Workflow Engine</a>"), "should not say 'Assay Workflow Engine' in link");
        assert!(out.contains(r#"href="https://assay.rs""#));
        assert!(out.contains(r#"target="_blank""#));
        assert!(out.contains(r#"rel="noopener noreferrer""#));
        // Version span is still emitted for the existing JS populator.
        assert!(out.contains(r#"<span id="status-version">—</span>"#));
    }

    #[test]
    fn favicon_url_override_emits_operator_link_tag() {
        let mut cfg = default_cfg();
        cfg.favicon_url = Some("/static/acme-favicon.ico".into());
        let out = render_index(TEMPLATE, "v", &cfg);
        assert!(out.contains(r#"href="/static/acme-favicon.ico""#));
        assert!(!out.contains(r#"href="/workflow/favicon.svg""#));
    }

    #[test]
    fn default_namespace_override_threads_into_data_attr() {
        let mut cfg = default_cfg();
        cfg.default_namespace = "deployments".into();
        let out = render_index(TEMPLATE, "v", &cfg);
        assert!(out.contains(r#"data-default-namespace="deployments""#));
    }

    #[test]
    fn favicon_override_flips_footer_attribution_too() {
        // A favicon override alone counts as "customised" — the footer
        // should switch to the "Powered by Assay" variant.
        let mut cfg = default_cfg();
        cfg.favicon_url = Some("/f.ico".into());
        let out = render_index(TEMPLATE, "v", &cfg);
        assert!(out.contains("Powered by"));
    }

    #[test]
    fn customised_detection_requires_more_than_defaults() {
        let base = default_cfg();
        assert!(!base.is_customised(), "stock defaults must not read as customised");

        let mut with_name = default_cfg();
        with_name.name = "Acme".into();
        assert!(with_name.is_customised());

        let mut with_subtitle = default_cfg();
        with_subtitle.subtitle = "Something".into();
        assert!(with_subtitle.is_customised());

        let mut with_logo = default_cfg();
        with_logo.logo_url = Some("/l.svg".into());
        assert!(with_logo.is_customised());

        let mut with_css = default_cfg();
        with_css.css_url = Some("/t.css".into());
        assert!(with_css.is_customised());
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
    fn css_url_unset_emits_no_extra_stylesheet() {
        let out = render_index(TEMPLATE, "42", &default_cfg());
        assert!(
            !out.contains("rel=\"stylesheet\""),
            "no extra stylesheet should render when ASSAY_WHITELABEL_CSS_URL is unset"
        );
    }

    #[test]
    fn css_url_emits_cache_busted_link_tag() {
        let mut cfg = default_cfg();
        cfg.css_url = Some("/static/cc-theme.css".into());
        let out = render_index(TEMPLATE, "42", &cfg);
        assert!(out.contains(r#"<link rel="stylesheet" href="/static/cc-theme.css?v=42">"#));
    }

    #[test]
    fn css_url_with_existing_query_string_uses_ampersand() {
        // Operators may want to version their stylesheet themselves
        // (`?rev=abc`) — we still tack the asset-version on without
        // breaking the query string.
        let mut cfg = default_cfg();
        cfg.css_url = Some("/static/cc-theme.css?rev=abc".into());
        let out = render_index(TEMPLATE, "42", &cfg);
        assert!(out.contains("href=\"/static/cc-theme.css?rev=abc&v=42\""));
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
