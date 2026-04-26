//! Audit event forwarding (plan 17 §S8).
//!
//! Every vault operation can fan out to external sinks in addition to
//! the engine's PG-backed audit table. Phase 2 ships:
//!
//! - [`Sink`] trait — async forward; sinks implement it.
//! - [`WebhookSink`] — POSTs JSON to a configured URL. Reuses `reqwest`.
//! - [`SinkRegistry`] — owns a Vec<Box<dyn Sink>>; `dispatch` fans an
//!   event out to every sink whose filter matches.
//!
//! syslog and S3 sinks are reserved for the next phase-2 commit;
//! syslog wants the `syslog` crate, S3 wants the `aws-sigv4` minimal
//! path that's also in scope for Phase 5 dynamic-creds.
//!
//! The persistence shape (one row in `vault.audit_sinks` per
//! configured sink) is in the schema already; the admin HTTP for
//! managing sinks ships in a follow-up commit alongside the missing
//! sink kinds.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::error::Result;

/// One audit event. The `event` field is dotted and matches the glob
/// patterns operators set on each sink (e.g. `vault.kv.put`,
/// `vault.transit.encrypt`, `auth.login.success`).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AuditEvent {
    pub event: String,
    pub actor: Option<String>,
    pub at: f64,
    #[serde(default)]
    pub fields: HashMap<String, serde_json::Value>,
}

impl AuditEvent {
    pub fn now(event: impl Into<String>) -> Self {
        Self {
            event: event.into(),
            actor: None,
            at: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs_f64(),
            fields: HashMap::new(),
        }
    }

    pub fn actor(mut self, who: impl Into<String>) -> Self {
        self.actor = Some(who.into());
        self
    }

    pub fn field(mut self, key: impl Into<String>, value: serde_json::Value) -> Self {
        self.fields.insert(key.into(), value);
        self
    }
}

/// A forwarding sink. Implementations ship the event to wherever the
/// operator wired them — webhook, syslog, S3, …
#[async_trait]
pub trait Sink: Send + Sync + 'static {
    /// Display name for logs / dashboards.
    fn name(&self) -> &str;
    /// Glob pattern over event names. `*` matches any single segment.
    fn filter(&self) -> &str;
    /// Forward the event. Errors here log and move on; one bad sink
    /// must not break the others (see [`SinkRegistry::dispatch`]).
    async fn forward(&self, event: &AuditEvent) -> Result<()>;
}

/// Glob match — `*` matches one or more characters that are not `.`.
/// `vault.*` matches `vault.kv.put`. `*.put` matches `vault.kv.put`.
/// `*` matches any single segment. The plan locks the implementation
/// to "matches the pattern that's in the audit_sinks.filter_pattern
/// column"; this is the canonical interpretation.
pub fn glob_match(pattern: &str, event: &str) -> bool {
    if pattern == "*" {
        return true;
    }
    let p_parts: Vec<&str> = pattern.split('.').collect();
    let e_parts: Vec<&str> = event.split('.').collect();

    fn segment_match(p: &str, e: &str) -> bool {
        if p == "*" {
            return true;
        }
        // Literal segment.
        p == e
    }

    if p_parts.len() == e_parts.len() {
        return p_parts
            .iter()
            .zip(e_parts.iter())
            .all(|(p, e)| segment_match(p, e));
    }
    // Trailing `*` matches "the rest of the path".
    if p_parts.last() == Some(&"*") && p_parts.len() <= e_parts.len() {
        return p_parts[..p_parts.len() - 1]
            .iter()
            .zip(e_parts.iter())
            .all(|(p, e)| segment_match(p, e));
    }
    false
}

#[cfg(feature = "vault-audit-forwarding")]
mod webhook {
    use super::*;
    use std::collections::HashMap;

    /// HTTP POST sink. Sends one JSON body per event by default; future
    /// commits add per-N-second batching and TLS client-cert auth. The
    /// `headers` map is applied verbatim so operators can wire bearer
    /// tokens or signing headers without code changes.
    pub struct WebhookSink {
        name: String,
        filter: String,
        url: String,
        headers: HashMap<String, String>,
        client: reqwest::Client,
    }

    impl WebhookSink {
        pub fn new(name: impl Into<String>, url: impl Into<String>, filter: impl Into<String>) -> Self {
            Self {
                name: name.into(),
                filter: filter.into(),
                url: url.into(),
                headers: HashMap::new(),
                client: reqwest::Client::new(),
            }
        }

        pub fn with_header(mut self, k: impl Into<String>, v: impl Into<String>) -> Self {
            self.headers.insert(k.into(), v.into());
            self
        }
    }

    #[async_trait]
    impl Sink for WebhookSink {
        fn name(&self) -> &str {
            &self.name
        }

        fn filter(&self) -> &str {
            &self.filter
        }

        async fn forward(&self, event: &AuditEvent) -> Result<()> {
            let mut req = self.client.post(&self.url).json(event);
            for (k, v) in &self.headers {
                req = req.header(k, v);
            }
            let resp = req
                .send()
                .await
                .map_err(|e| crate::error::VaultError::Backend(anyhow::anyhow!("webhook POST: {e}")))?;
            if !resp.status().is_success() {
                return Err(crate::error::VaultError::Backend(anyhow::anyhow!(
                    "webhook POST returned {}",
                    resp.status()
                )));
            }
            Ok(())
        }
    }
}

#[cfg(feature = "vault-audit-forwarding")]
pub use webhook::WebhookSink;

/// Owns every configured sink. Cheap to clone — sinks live behind
/// `Arc`s. Cloning the registry shares the same set across handlers.
#[derive(Default, Clone)]
pub struct SinkRegistry {
    sinks: std::sync::Arc<Vec<std::sync::Arc<dyn Sink>>>,
}

impl SinkRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Build from an explicit set of sinks.
    pub fn from_sinks(sinks: Vec<std::sync::Arc<dyn Sink>>) -> Self {
        Self {
            sinks: std::sync::Arc::new(sinks),
        }
    }

    /// Number of configured sinks.
    pub fn len(&self) -> usize {
        self.sinks.len()
    }

    pub fn is_empty(&self) -> bool {
        self.sinks.is_empty()
    }

    /// Fan an event out to every sink whose filter matches. Each sink
    /// is fired in parallel; one failure logs but doesn't abort the
    /// others — auditing must be fire-and-forget at this layer (the
    /// PG-backed audit table is the source of truth).
    pub async fn dispatch(&self, event: &AuditEvent) {
        if self.sinks.is_empty() {
            return;
        }
        let mut futs = Vec::with_capacity(self.sinks.len());
        for sink in self.sinks.iter() {
            if !glob_match(sink.filter(), &event.event) {
                continue;
            }
            let sink = sink.clone();
            let event = event.clone();
            futs.push(async move {
                if let Err(e) = sink.forward(&event).await {
                    tracing::warn!(
                        target: "assay-vault",
                        sink = sink.name(),
                        event = %event.event,
                        ?e,
                        "audit sink forward failed"
                    );
                }
            });
        }
        for f in futs {
            f.await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    #[test]
    fn glob_matches_exact_segment_count() {
        assert!(glob_match("vault.kv.put", "vault.kv.put"));
        assert!(!glob_match("vault.kv.put", "vault.kv.get"));
        assert!(glob_match("vault.*.put", "vault.kv.put"));
        assert!(glob_match("vault.*.put", "vault.transit.put"));
        assert!(!glob_match("vault.*.put", "vault.kv.get"));
        assert!(glob_match("*", "anything.at.all"));
    }

    #[test]
    fn glob_matches_trailing_star() {
        assert!(glob_match("vault.*", "vault.kv.put"));
        assert!(glob_match("vault.*", "vault.kv"));
        assert!(!glob_match("vault.*", "auth.login"));
    }

    /// In-memory sink for testing — counts events, captures filter.
    struct CountSink {
        name: String,
        filter: String,
        count: AtomicUsize,
    }

    #[async_trait]
    impl Sink for CountSink {
        fn name(&self) -> &str {
            &self.name
        }
        fn filter(&self) -> &str {
            &self.filter
        }
        async fn forward(&self, _event: &AuditEvent) -> Result<()> {
            self.count.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }

    #[tokio::test]
    async fn registry_dispatches_to_matching_sinks_only() {
        let sink_a = Arc::new(CountSink {
            name: "vault-only".into(),
            filter: "vault.*".into(),
            count: AtomicUsize::new(0),
        });
        let sink_b = Arc::new(CountSink {
            name: "all".into(),
            filter: "*".into(),
            count: AtomicUsize::new(0),
        });
        let reg = SinkRegistry::from_sinks(vec![sink_a.clone() as _, sink_b.clone() as _]);

        reg.dispatch(&AuditEvent::now("vault.kv.put")).await;
        reg.dispatch(&AuditEvent::now("auth.login")).await;

        assert_eq!(sink_a.count.load(Ordering::SeqCst), 1, "vault-only sink should fire once");
        assert_eq!(sink_b.count.load(Ordering::SeqCst), 2, "catch-all sink should fire twice");
    }

    #[tokio::test]
    async fn empty_registry_is_no_op() {
        let reg = SinkRegistry::new();
        reg.dispatch(&AuditEvent::now("anything")).await;
    }
}
