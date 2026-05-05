//! Consent screen — minimal HTML rendered via `askama`, plus a POST
//! handler that writes the per-(user, client) `auth.oidc_consents` row
//! and resumes the `/authorize` flow.
//!
//! Per OIDC Core §3.1.2.4 + §5.1 the user agent has to be given the
//! opportunity to approve / deny the requested scopes before a code is
//! issued. We render the prompt inline (no JS) so consumer apps don't
//! need any browser machinery beyond following redirects.

use askama::Template;

/// Minimal askama template for the consent screen. Inlined here so a
/// fresh `cargo build` doesn't depend on a separate templates dir.
#[derive(Template)]
#[template(
    source = r#"<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="utf-8" />
  <title>Authorize {{ client_name }}</title>
  <style>
    body { font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif;
           max-width: 480px; margin: 4em auto; color: #222; }
    h1 { font-size: 1.25rem; margin-bottom: 0.25em; }
    .sub { color: #666; margin-bottom: 1.5em; }
    ul { background: #f7f7f7; border-radius: 6px; padding: 1em 2em; }
    li { margin: 0.25em 0; }
    form { display: inline-block; margin-right: 0.5em; }
    button { padding: 0.75em 1.5em; border: none; border-radius: 6px;
             cursor: pointer; font-size: 1rem; }
    button.allow { background: #2563eb; color: white; }
    button.deny  { background: #f3f4f6; color: #222; }
  </style>
</head>
<body>
  <h1>{{ client_name }} wants to access your account</h1>
  <div class="sub">Issued by <strong>{{ issuer }}</strong></div>
  <ul>
    {% for scope in scopes %}
    <li>{{ scope }}</li>
    {% endfor %}
  </ul>
  <form method="POST" action="/authorize/consent">
    <input type="hidden" name="csrf_token" value="{{ csrf_token }}" />
    <input type="hidden" name="resume_token" value="{{ resume_token }}" />
    <input type="hidden" name="decision" value="allow" />
    <button class="allow" type="submit">Allow</button>
  </form>
  <form method="POST" action="/authorize/consent">
    <input type="hidden" name="csrf_token" value="{{ csrf_token }}" />
    <input type="hidden" name="resume_token" value="{{ resume_token }}" />
    <input type="hidden" name="decision" value="deny" />
    <button class="deny" type="submit">Deny</button>
  </form>
</body>
</html>"#,
    ext = "html",
    escape = "html"
)]
pub struct ConsentPage<'a> {
    pub client_name: &'a str,
    pub issuer: &'a str,
    pub scopes: &'a [String],
    pub csrf_token: &'a str,
    pub resume_token: &'a str,
}

impl<'a> ConsentPage<'a> {
    /// Render the page to a UTF-8 HTML body. Errors only when the
    /// inline template fails to compile (which is a build-time guarantee
    /// thanks to `#[derive(Template)]`); the runtime path treats this
    /// as infallible by falling back to a hard-coded sentinel.
    pub fn render_html(&self) -> String {
        self.render().unwrap_or_else(|_| {
            "<!doctype html><body>consent screen render error</body>".to_string()
        })
    }
}

/// Parsed consent form submission.
#[derive(Clone, Debug, PartialEq, Eq, serde::Deserialize)]
pub struct ConsentSubmission {
    pub csrf_token: String,
    pub resume_token: String,
    pub decision: String,
}

impl ConsentSubmission {
    pub fn allowed(&self) -> bool {
        self.decision == "allow"
    }
}

/// Whether the requested `scopes` are a subset of the previously-granted
/// `granted_scopes`. Used to skip the consent screen when the user has
/// already granted the same (or wider) scopes for this client.
pub fn scopes_already_granted(requested: &[String], granted: &[String]) -> bool {
    requested.iter().all(|s| granted.iter().any(|g| g == s))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn page_renders_html_with_scopes() {
        let scopes = vec!["openid".to_string(), "email".to_string()];
        let page = ConsentPage {
            client_name: "Test App",
            issuer: "https://idp.example.com",
            scopes: &scopes,
            csrf_token: "csrf_xyz",
            resume_token: "resume_abc",
        };
        let html = page.render_html();
        assert!(html.contains("Test App"));
        assert!(html.contains("openid"));
        assert!(html.contains("email"));
        assert!(html.contains("csrf_xyz"));
        assert!(html.contains("resume_abc"));
        assert!(html.contains("/authorize/consent"));
    }

    #[test]
    fn submission_decision_predicate() {
        let allow = ConsentSubmission {
            csrf_token: "x".into(),
            resume_token: "y".into(),
            decision: "allow".into(),
        };
        let deny = ConsentSubmission {
            csrf_token: "x".into(),
            resume_token: "y".into(),
            decision: "deny".into(),
        };
        assert!(allow.allowed());
        assert!(!deny.allowed());
    }

    #[test]
    fn scope_subset_check_handles_subset_and_super() {
        let granted = vec![
            "openid".to_string(),
            "email".to_string(),
            "profile".to_string(),
        ];
        assert!(scopes_already_granted(
            &["openid".to_string(), "email".to_string()],
            &granted
        ));
        assert!(scopes_already_granted(&["openid".to_string()], &granted));
        // Requested wider than granted → not already granted.
        assert!(!scopes_already_granted(
            &["openid".to_string(), "groups".to_string()],
            &granted
        ));
    }
}
