//! HTTP client for the workflow engine REST API.
//!
//! Every method corresponds to one endpoint. Returns `anyhow::Result`
//! with the HTTP status and response body folded into the error on
//! non-2xx responses, so CLI callers can surface a useful message.

use anyhow::{anyhow, Context, Result};
use serde_json::Value;

use crate::cli::GlobalOpts;

pub struct EngineClient {
    base: String,
    http: reqwest::Client,
    api_key: Option<String>,
    namespace: String,
}

impl EngineClient {
    pub fn new(opts: &GlobalOpts) -> Self {
        Self {
            base: format!("{}/api/v1", opts.engine_url.trim_end_matches('/')),
            http: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(30))
                .build()
                .expect("building reqwest client"),
            api_key: opts.api_key.clone(),
            namespace: opts.namespace.clone(),
        }
    }

    fn with_auth(&self, mut req: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        if let Some(ref key) = self.api_key {
            req = req.header("Authorization", format!("Bearer {key}"));
        }
        req
    }

    async fn send(&self, req: reqwest::RequestBuilder, ctx: &str) -> Result<Value> {
        let resp = self
            .with_auth(req)
            .send()
            .await
            .with_context(|| format!("{ctx}: engine unreachable at {}", self.base))?;
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        if !status.is_success() {
            return Err(anyhow!(
                "{ctx}: HTTP {status}: {}",
                if body.is_empty() { "<empty>" } else { &body }
            ));
        }
        if body.is_empty() {
            return Ok(Value::Null);
        }
        serde_json::from_str(&body).with_context(|| format!("{ctx}: parsing response body"))
    }

    // ── Workflows ──────────────────────────────────────────

    pub async fn workflow_start(
        &self,
        workflow_type: &str,
        workflow_id: &str,
        input: Option<&Value>,
        task_queue: Option<&str>,
        search_attributes: Option<&Value>,
    ) -> Result<Value> {
        let url = format!("{}/workflows", self.base);
        let mut body = serde_json::json!({
            "namespace": self.namespace,
            "workflow_type": workflow_type,
            "workflow_id": workflow_id,
            "task_queue": task_queue.unwrap_or("default"),
        });
        if let Some(v) = input {
            body["input"] = v.clone();
        }
        if let Some(v) = search_attributes {
            body["search_attributes"] = v.clone();
        }
        self.send(self.http.post(&url).json(&body), "workflow start")
            .await
    }

    pub async fn workflow_list(
        &self,
        status: Option<&str>,
        workflow_type: Option<&str>,
        search_attrs: Option<&Value>,
        limit: Option<i64>,
    ) -> Result<Value> {
        let mut url = format!("{}/workflows?namespace={}", self.base, self.namespace);
        if let Some(s) = status {
            url.push_str(&format!("&status={s}"));
        }
        if let Some(t) = workflow_type {
            url.push_str(&format!("&type={t}"));
        }
        if let Some(l) = limit {
            url.push_str(&format!("&limit={l}"));
        }
        if let Some(attrs) = search_attrs {
            let encoded = urlencoding_encode(&attrs.to_string());
            url.push_str(&format!("&search_attrs={encoded}"));
        }
        self.send(self.http.get(&url), "workflow list").await
    }

    pub async fn workflow_describe(&self, id: &str) -> Result<Value> {
        let url = format!("{}/workflows/{id}", self.base);
        self.send(self.http.get(&url), "workflow describe").await
    }

    pub async fn workflow_state(&self, id: &str, name: Option<&str>) -> Result<Value> {
        let url = match name {
            Some(n) => format!("{}/workflows/{id}/state/{n}", self.base),
            None => format!("{}/workflows/{id}/state", self.base),
        };
        self.send(self.http.get(&url), "workflow state").await
    }

    pub async fn workflow_events(&self, id: &str) -> Result<Value> {
        let url = format!("{}/workflows/{id}/events", self.base);
        self.send(self.http.get(&url), "workflow events").await
    }

    pub async fn workflow_children(&self, id: &str) -> Result<Value> {
        let url = format!("{}/workflows/{id}/children", self.base);
        self.send(self.http.get(&url), "workflow children").await
    }

    pub async fn workflow_signal(
        &self,
        id: &str,
        name: &str,
        payload: Option<&Value>,
    ) -> Result<()> {
        let url = format!("{}/workflows/{id}/signal/{name}", self.base);
        let body = serde_json::json!({ "payload": payload });
        let _ = self
            .send(self.http.post(&url).json(&body), "workflow signal")
            .await?;
        Ok(())
    }

    pub async fn workflow_cancel(&self, id: &str) -> Result<()> {
        let url = format!("{}/workflows/{id}/cancel", self.base);
        let _ = self
            .send(self.http.post(&url), "workflow cancel")
            .await?;
        Ok(())
    }

    pub async fn workflow_terminate(&self, id: &str, reason: Option<&str>) -> Result<()> {
        let url = format!("{}/workflows/{id}/terminate", self.base);
        let body = serde_json::json!({ "reason": reason });
        let _ = self
            .send(self.http.post(&url).json(&body), "workflow terminate")
            .await?;
        Ok(())
    }

    pub async fn workflow_continue_as_new(
        &self,
        id: &str,
        input: Option<&Value>,
    ) -> Result<Value> {
        let url = format!("{}/workflows/{id}/continue-as-new", self.base);
        let body = serde_json::json!({ "input": input });
        self.send(
            self.http.post(&url).json(&body),
            "workflow continue-as-new",
        )
        .await
    }

    // ── Schedules ──────────────────────────────────────────

    pub async fn schedule_list(&self) -> Result<Value> {
        let url = format!("{}/schedules?namespace={}", self.base, self.namespace);
        self.send(self.http.get(&url), "schedule list").await
    }

    pub async fn schedule_describe(&self, name: &str) -> Result<Value> {
        let url = format!(
            "{}/schedules/{name}?namespace={}",
            self.base, self.namespace
        );
        self.send(self.http.get(&url), "schedule describe").await
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn schedule_create(
        &self,
        name: &str,
        workflow_type: &str,
        cron: &str,
        timezone: Option<&str>,
        input: Option<&Value>,
        queue: Option<&str>,
    ) -> Result<Value> {
        let url = format!("{}/schedules", self.base);
        let mut body = serde_json::json!({
            "name": name,
            "namespace": self.namespace,
            "workflow_type": workflow_type,
            "cron_expr": cron,
        });
        if let Some(tz) = timezone {
            body["timezone"] = Value::String(tz.to_string());
        }
        if let Some(q) = queue {
            body["task_queue"] = Value::String(q.to_string());
        }
        if let Some(i) = input {
            body["input"] = i.clone();
        }
        self.send(self.http.post(&url).json(&body), "schedule create")
            .await
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn schedule_patch(
        &self,
        name: &str,
        cron: Option<&str>,
        timezone: Option<&str>,
        input: Option<&Value>,
        queue: Option<&str>,
        overlap: Option<&str>,
    ) -> Result<Value> {
        let url = format!(
            "{}/schedules/{name}?namespace={}",
            self.base, self.namespace
        );
        let mut body = serde_json::Map::new();
        if let Some(v) = cron {
            body.insert("cron_expr".into(), Value::String(v.to_string()));
        }
        if let Some(v) = timezone {
            body.insert("timezone".into(), Value::String(v.to_string()));
        }
        if let Some(v) = input {
            body.insert("input".into(), v.clone());
        }
        if let Some(v) = queue {
            body.insert("task_queue".into(), Value::String(v.to_string()));
        }
        if let Some(v) = overlap {
            body.insert("overlap_policy".into(), Value::String(v.to_string()));
        }
        self.send(
            self.http.patch(&url).json(&Value::Object(body)),
            "schedule patch",
        )
        .await
    }

    pub async fn schedule_pause(&self, name: &str) -> Result<Value> {
        let url = format!(
            "{}/schedules/{name}/pause?namespace={}",
            self.base, self.namespace
        );
        self.send(self.http.post(&url), "schedule pause").await
    }

    pub async fn schedule_resume(&self, name: &str) -> Result<Value> {
        let url = format!(
            "{}/schedules/{name}/resume?namespace={}",
            self.base, self.namespace
        );
        self.send(self.http.post(&url), "schedule resume").await
    }

    pub async fn schedule_delete(&self, name: &str) -> Result<()> {
        let url = format!(
            "{}/schedules/{name}?namespace={}",
            self.base, self.namespace
        );
        let _ = self
            .send(self.http.delete(&url), "schedule delete")
            .await?;
        Ok(())
    }

    // ── Namespaces ─────────────────────────────────────────

    pub async fn namespace_create(&self, name: &str) -> Result<()> {
        let url = format!("{}/namespaces", self.base);
        let body = serde_json::json!({ "name": name });
        let _ = self
            .send(self.http.post(&url).json(&body), "namespace create")
            .await?;
        Ok(())
    }

    pub async fn namespace_list(&self) -> Result<Value> {
        let url = format!("{}/namespaces", self.base);
        self.send(self.http.get(&url), "namespace list").await
    }

    pub async fn namespace_stats(&self, name: &str) -> Result<Value> {
        let url = format!("{}/namespaces/{name}", self.base);
        self.send(self.http.get(&url), "namespace describe").await
    }

    pub async fn namespace_delete(&self, name: &str) -> Result<()> {
        let url = format!("{}/namespaces/{name}", self.base);
        let _ = self
            .send(self.http.delete(&url), "namespace delete")
            .await?;
        Ok(())
    }

    // ── Workers ────────────────────────────────────────────

    pub async fn worker_list(&self) -> Result<Value> {
        let url = format!("{}/workers?namespace={}", self.base, self.namespace);
        self.send(self.http.get(&url), "worker list").await
    }

    // ── Queues ─────────────────────────────────────────────

    pub async fn queue_stats(&self) -> Result<Value> {
        let url = format!("{}/queues?namespace={}", self.base, self.namespace);
        self.send(self.http.get(&url), "queue stats").await
    }
}

/// Tiny URL encoder — just enough for JSON values in query params
/// (no external crate dep).
fn urlencoding_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.bytes() {
        match c {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(c as char)
            }
            _ => out.push_str(&format!("%{c:02X}")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::urlencoding_encode;

    #[test]
    fn encodes_json_like_strings() {
        let encoded = urlencoding_encode(r#"{"env":"prod"}"#);
        assert_eq!(encoded, "%7B%22env%22%3A%22prod%22%7D");
    }

    #[test]
    fn leaves_safe_chars_alone() {
        assert_eq!(urlencoding_encode("abc-123_XYZ"), "abc-123_XYZ");
    }
}
