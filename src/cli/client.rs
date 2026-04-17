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

    pub async fn workflow_list(
        &self,
        status: Option<&str>,
        workflow_type: Option<&str>,
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

    // ── Schedules ──────────────────────────────────────────

    pub async fn schedule_list(&self) -> Result<Value> {
        let url = format!("{}/schedules?namespace={}", self.base, self.namespace);
        self.send(self.http.get(&url), "schedule list").await
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
}
