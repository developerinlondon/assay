//! Sample-data seeder for `assay-engine seed-sample`.
//!
//! Drives a running engine over HTTP (admin api-key required) so the
//! seeder works against any deployment topology — local SQLite, remote
//! PG, multi-instance — without coupling to the storage layer.
//! Idempotent by design: every "create" path either checks for an
//! existing row first or relies on `ON CONFLICT DO NOTHING`-style
//! semantics from the underlying API, so re-running is a no-op.
//!
//! Requires the `server` feature (which pulls in `reqwest`).

use std::time::Duration;

use anyhow::{Context, Result};
use reqwest::{Client, StatusCode};
use serde_json::{Value, json};

/// One thing we tried to seed and what happened to it. The CLI prints
/// these as a table at the end so operators can see what's idempotent
/// (existed → "exists") vs what was minted ("created").
#[derive(Debug)]
pub struct SeedReport {
    pub kind: String,
    pub name: String,
    pub status: SeedStatus,
}

#[derive(Debug, PartialEq)]
pub enum SeedStatus {
    Created,
    Exists,
    Skipped(String),
    Failed(String),
}

impl SeedStatus {
    pub fn label(&self) -> &str {
        match self {
            SeedStatus::Created => "created",
            SeedStatus::Exists => "exists",
            SeedStatus::Skipped(_) => "skipped",
            SeedStatus::Failed(_) => "failed",
        }
    }
}

/// Run the full seed against `base_url` using `admin_key` as the
/// `Authorization: Bearer` token. Returns the per-item report so the
/// caller can render a summary.
pub async fn run(base_url: &str, admin_key: &str) -> Result<Vec<SeedReport>> {
    let client = Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .context("build reqwest client")?;
    let base = base_url.trim_end_matches('/').to_string();
    let mut report = Vec::new();

    // Fetch /api/v1/modules so we can skip auth-only seeding when the
    // auth module isn't enabled (no point hitting /admin/auth/*).
    let modules = fetch_modules(&client, &base).await.unwrap_or_default();
    let auth_on = modules.iter().any(|m| m == "auth");

    seed_workflow_namespaces(&client, &base, &mut report).await;
    seed_workflows(&client, &base, &mut report).await;

    if auth_on {
        seed_users(&client, &base, admin_key, &mut report).await;
        seed_oidc_clients(&client, &base, admin_key, &mut report).await;
        seed_oidc_upstream(&client, &base, admin_key, &mut report).await;
        seed_zanzibar(&client, &base, admin_key, &mut report).await;
    } else {
        report.push(SeedReport {
            kind: "auth-suite".into(),
            name: "(all)".into(),
            status: SeedStatus::Skipped("auth module not enabled".into()),
        });
    }

    Ok(report)
}

// =====================================================================
//   workflow seeding (no-auth endpoints)
// =====================================================================

async fn fetch_modules(client: &Client, base: &str) -> Result<Vec<String>> {
    let r = client.get(format!("{base}/api/v1/modules")).send().await?;
    if !r.status().is_success() {
        return Ok(Vec::new());
    }
    let v: Value = r.json().await?;
    Ok(v.get("modules")
        .and_then(|m| m.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default())
}

async fn seed_workflow_namespaces(client: &Client, base: &str, report: &mut Vec<SeedReport>) {
    for ns in ["demo", "prod"] {
        let r = client
            .post(format!("{base}/api/v1/namespaces"))
            .json(&json!({"name": ns}))
            .send()
            .await;
        match r {
            Ok(resp) if resp.status().is_success() => report.push(SeedReport {
                kind: "namespace".into(),
                name: ns.into(),
                status: SeedStatus::Created,
            }),
            Ok(resp) if resp.status() == StatusCode::CONFLICT
                || resp.status() == StatusCode::BAD_REQUEST =>
            {
                // assay-workflow returns 400/409 for an existing namespace
                // — treat both as "already there".
                report.push(SeedReport {
                    kind: "namespace".into(),
                    name: ns.into(),
                    status: SeedStatus::Exists,
                });
            }
            Ok(resp) => report.push(SeedReport {
                kind: "namespace".into(),
                name: ns.into(),
                status: SeedStatus::Failed(format!("HTTP {}", resp.status())),
            }),
            Err(e) => report.push(SeedReport {
                kind: "namespace".into(),
                name: ns.into(),
                status: SeedStatus::Failed(e.to_string()),
            }),
        }
    }
}

async fn seed_workflows(client: &Client, base: &str, report: &mut Vec<SeedReport>) {
    // Stable workflow IDs so re-runs don't duplicate. The API rejects
    // duplicate IDs in the same namespace as a conflict — we map that to
    // "Exists" the same way we treat namespaces.
    let specs: Vec<(&str, &str, &str, Value)> = vec![
        (
            "demo-greet-1",
            "demo.greet",
            "demo",
            json!({"name": "alice"}),
        ),
        (
            "demo-greet-2",
            "demo.greet",
            "demo",
            json!({"name": "bob"}),
        ),
        (
            "demo-greet-3",
            "demo.greet",
            "demo",
            json!({"name": "cousin"}),
        ),
    ];
    for (id, wf_type, ns, input) in specs {
        let body = json!({
            "workflow_id": id,
            "workflow_type": wf_type,
            "namespace": ns,
            "task_queue": "default",
            "input": input.to_string(),
        });
        let r = client
            .post(format!("{base}/api/v1/workflows"))
            .json(&body)
            .send()
            .await;
        match r {
            Ok(resp) if resp.status().is_success() => report.push(SeedReport {
                kind: "workflow".into(),
                name: id.into(),
                status: SeedStatus::Created,
            }),
            Ok(resp) if resp.status() == StatusCode::CONFLICT
                || resp.status() == StatusCode::BAD_REQUEST =>
            {
                report.push(SeedReport {
                    kind: "workflow".into(),
                    name: id.into(),
                    status: SeedStatus::Exists,
                });
            }
            Ok(resp) => report.push(SeedReport {
                kind: "workflow".into(),
                name: id.into(),
                status: SeedStatus::Failed(format!("HTTP {}", resp.status())),
            }),
            Err(e) => report.push(SeedReport {
                kind: "workflow".into(),
                name: id.into(),
                status: SeedStatus::Failed(e.to_string()),
            }),
        }
    }
}

// =====================================================================
//   auth users (admin)
// =====================================================================

#[derive(Debug, Clone, Copy)]
struct UserSpec {
    email: &'static str,
    display: &'static str,
    verified: bool,
    password: Option<&'static str>,
}

const USER_FIXTURES: &[UserSpec] = &[
    UserSpec {
        email: "alice@example.com",
        display: "Alice Demo",
        verified: true,
        password: Some("assay-demo"),
    },
    UserSpec {
        email: "bob@example.com",
        display: "Bob Demo",
        verified: true,
        password: Some("assay-demo"),
    },
    UserSpec {
        email: "cousin@example.com",
        display: "Cousin Demo",
        verified: false,
        password: None,
    },
    UserSpec {
        email: "admin@example.com",
        display: "Admin Demo",
        verified: true,
        password: Some("assay-demo"),
    },
];

async fn seed_users(
    client: &Client,
    base: &str,
    admin_key: &str,
    report: &mut Vec<SeedReport>,
) {
    // Look up the existing user list so we can short-circuit
    // already-seeded fixtures without minting duplicate rows.
    let existing = match list_users(client, base, admin_key).await {
        Ok(v) => v,
        Err(e) => {
            report.push(SeedReport {
                kind: "users".into(),
                name: "(list)".into(),
                status: SeedStatus::Failed(e.to_string()),
            });
            return;
        }
    };
    for spec in USER_FIXTURES {
        let already = existing
            .iter()
            .any(|u| u.get("email").and_then(|x| x.as_str()) == Some(spec.email));
        if already {
            report.push(SeedReport {
                kind: "user".into(),
                name: spec.email.into(),
                status: SeedStatus::Exists,
            });
            continue;
        }
        let body = json!({
            "email": spec.email,
            "display_name": spec.display,
            "email_verified": spec.verified,
            "password": spec.password,
        });
        let r = client
            .post(format!("{base}/admin/auth/users"))
            .bearer_auth(admin_key)
            .json(&body)
            .send()
            .await;
        match r {
            Ok(resp) if resp.status().is_success() => report.push(SeedReport {
                kind: "user".into(),
                name: spec.email.into(),
                status: SeedStatus::Created,
            }),
            Ok(resp) => report.push(SeedReport {
                kind: "user".into(),
                name: spec.email.into(),
                status: SeedStatus::Failed(format!("HTTP {}", resp.status())),
            }),
            Err(e) => report.push(SeedReport {
                kind: "user".into(),
                name: spec.email.into(),
                status: SeedStatus::Failed(e.to_string()),
            }),
        }
    }
}

async fn list_users(client: &Client, base: &str, admin_key: &str) -> Result<Vec<Value>> {
    let r = client
        .get(format!("{base}/admin/auth/users?limit=500"))
        .bearer_auth(admin_key)
        .send()
        .await?;
    if !r.status().is_success() {
        anyhow::bail!("list users: HTTP {}", r.status());
    }
    let v: Value = r.json().await?;
    Ok(v.get("items")
        .and_then(|x| x.as_array())
        .cloned()
        .unwrap_or_default())
}

// =====================================================================
//   OIDC clients (admin)
// =====================================================================

async fn seed_oidc_clients(
    client: &Client,
    base: &str,
    admin_key: &str,
    report: &mut Vec<SeedReport>,
) {
    // Stable client_ids so re-runs are idempotent. The OIDC admin API
    // returns 409 on duplicate client_id (or treats it as a conflict);
    // we map that to "Exists".
    let specs: &[(&str, Value)] = &[
        (
            "demo-spa",
            json!({
                "client_id": "demo-spa",
                "name": "Demo SPA",
                "redirect_uris": ["http://localhost:3001/callback"],
                "token_endpoint_auth_method": "none",
                "default_scopes": ["openid", "profile"],
                "pkce_required": true,
                "grant_types": ["authorization_code", "refresh_token"],
                "response_types": ["code"]
            }),
        ),
        (
            "demo-service",
            json!({
                "client_id": "demo-service",
                "name": "Demo Service",
                "redirect_uris": ["http://localhost:3002/callback"],
                "token_endpoint_auth_method": "client_secret_basic",
                "default_scopes": ["openid"],
                "pkce_required": false,
                "grant_types": ["authorization_code", "refresh_token"],
                "response_types": ["code"]
            }),
        ),
    ];
    let existing = list_oidc_clients(client, base, admin_key)
        .await
        .unwrap_or_default();
    for (name, body) in specs {
        let id_match = body.get("client_id").and_then(|x| x.as_str()).unwrap_or("");
        let already = existing
            .iter()
            .any(|c| c.get("client_id").and_then(|x| x.as_str()) == Some(id_match));
        if already {
            report.push(SeedReport {
                kind: "oidc_client".into(),
                name: (*name).into(),
                status: SeedStatus::Exists,
            });
            continue;
        }
        let r = client
            .post(format!("{base}/admin/oidc/clients"))
            .bearer_auth(admin_key)
            .json(body)
            .send()
            .await;
        match r {
            Ok(resp) if resp.status().is_success() => report.push(SeedReport {
                kind: "oidc_client".into(),
                name: (*name).into(),
                status: SeedStatus::Created,
            }),
            Ok(resp) => report.push(SeedReport {
                kind: "oidc_client".into(),
                name: (*name).into(),
                status: SeedStatus::Failed(format!("HTTP {}", resp.status())),
            }),
            Err(e) => report.push(SeedReport {
                kind: "oidc_client".into(),
                name: (*name).into(),
                status: SeedStatus::Failed(e.to_string()),
            }),
        }
    }
}

async fn list_oidc_clients(client: &Client, base: &str, admin_key: &str) -> Result<Vec<Value>> {
    let r = client
        .get(format!("{base}/admin/oidc/clients"))
        .bearer_auth(admin_key)
        .send()
        .await?;
    if !r.status().is_success() {
        anyhow::bail!("list clients: HTTP {}", r.status());
    }
    let v: Value = r.json().await?;
    // The list endpoint returns the array directly (not wrapped in items).
    Ok(v.as_array().cloned().unwrap_or_default())
}

async fn seed_oidc_upstream(
    client: &Client,
    base: &str,
    admin_key: &str,
    report: &mut Vec<SeedReport>,
) {
    let body = json!({
        "slug": "example",
        "issuer": "https://accounts.example.com",
        "client_id": "demo-upstream-client",
        "client_secret": "demo-upstream-secret-change-me",
        "display_name": "Example IdP",
        "icon_url": null,
        "enabled": true
    });
    // The upsert path is naturally idempotent — re-POSTing the same slug
    // updates in place. Always reports as "created" / "updated".
    let r = client
        .post(format!("{base}/admin/oidc/upstream"))
        .bearer_auth(admin_key)
        .json(&body)
        .send()
        .await;
    match r {
        Ok(resp) if resp.status().is_success() => report.push(SeedReport {
            kind: "oidc_upstream".into(),
            name: "example".into(),
            status: SeedStatus::Created,
        }),
        Ok(resp) => report.push(SeedReport {
            kind: "oidc_upstream".into(),
            name: "example".into(),
            status: SeedStatus::Failed(format!("HTTP {}", resp.status())),
        }),
        Err(e) => report.push(SeedReport {
            kind: "oidc_upstream".into(),
            name: "example".into(),
            status: SeedStatus::Failed(e.to_string()),
        }),
    }
}

// =====================================================================
//   Zanzibar tuples (admin)
// =====================================================================

async fn seed_zanzibar(
    client: &Client,
    base: &str,
    admin_key: &str,
    report: &mut Vec<SeedReport>,
) {
    // Mirror the jeebon plan example: family + circle namespaces with
    // `member` / `admin` relations on alice + bob. write_tuple is
    // additive — the underlying store treats a duplicate (object,
    // relation, subject) as a no-op success.
    let tuples: &[Value] = &[
        json!({
            "object_type": "family", "object_id": "alice",
            "relation": "member",
            "subject_type": "user", "subject_id": "alice", "subject_rel": null
        }),
        json!({
            "object_type": "family", "object_id": "alice",
            "relation": "admin",
            "subject_type": "user", "subject_id": "alice", "subject_rel": null
        }),
        json!({
            "object_type": "family", "object_id": "bob",
            "relation": "member",
            "subject_type": "user", "subject_id": "bob", "subject_rel": null
        }),
        json!({
            "object_type": "family", "object_id": "bob",
            "relation": "admin",
            "subject_type": "user", "subject_id": "bob", "subject_rel": null
        }),
        json!({
            "object_type": "circle", "object_id": "inner",
            "relation": "member",
            "subject_type": "user", "subject_id": "alice", "subject_rel": null
        }),
        json!({
            "object_type": "circle", "object_id": "inner",
            "relation": "member",
            "subject_type": "user", "subject_id": "bob", "subject_rel": null
        }),
    ];
    for t in tuples {
        let label = format!(
            "{}:{} {} {}:{}",
            t["object_type"].as_str().unwrap_or("?"),
            t["object_id"].as_str().unwrap_or("?"),
            t["relation"].as_str().unwrap_or("?"),
            t["subject_type"].as_str().unwrap_or("?"),
            t["subject_id"].as_str().unwrap_or("?"),
        );
        let r = client
            .post(format!("{base}/admin/auth/zanzibar/tuples"))
            .bearer_auth(admin_key)
            .json(t)
            .send()
            .await;
        match r {
            Ok(resp) if resp.status().is_success() => report.push(SeedReport {
                kind: "zanzibar_tuple".into(),
                name: label,
                status: SeedStatus::Created,
            }),
            Ok(resp) => report.push(SeedReport {
                kind: "zanzibar_tuple".into(),
                name: label,
                status: SeedStatus::Failed(format!("HTTP {}", resp.status())),
            }),
            Err(e) => report.push(SeedReport {
                kind: "zanzibar_tuple".into(),
                name: label,
                status: SeedStatus::Failed(e.to_string()),
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seed_status_labels_are_stable() {
        assert_eq!(SeedStatus::Created.label(), "created");
        assert_eq!(SeedStatus::Exists.label(), "exists");
        assert_eq!(SeedStatus::Skipped("x".into()).label(), "skipped");
        assert_eq!(SeedStatus::Failed("y".into()).label(), "failed");
    }
}
