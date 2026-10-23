//! A redirect is a request the script never named, so each hop is judged by
//! the policy and the gate's classifier before reqwest follows it. reqwest
//! decides inside the future the transport awaits, so the request travels to
//! that decision in a task-local: the client is shared and predates the VM.

use std::sync::Arc;

use reqwest::StatusCode;
use reqwest::redirect::{Attempt, Policy};

use crate::lua::builtins::gated::{self, HttpCall};
use crate::lua::policy;

const MAX_REDIRECTS: usize = 10;

tokio::task_local! {
    static GUARD: Guard;
}

/// The request whose redirects are being followed, and what judges them.
pub(crate) struct Guard {
    /// `readonly` or `approval` when a gate is on; `None` leaves only policy.
    pub gate: Option<&'static str>,
    pub policy: Option<Arc<policy::Policy>>,
    pub method: String,
    pub headers: Vec<(String, String)>,
    pub body: Option<String>,
}

/// The redirect policy every client assay builds carries. With no guard in
/// scope it behaves like reqwest's default.
pub fn policy() -> Policy {
    Policy::custom(|attempt: Attempt| {
        if attempt.previous().len() > MAX_REDIRECTS {
            return attempt.error("too many redirects");
        }
        let verdict = GUARD.try_with(|guard| guard.check(attempt.status(), attempt.url()));
        match verdict {
            Ok(Err(refusal)) => attempt.error(refusal),
            _ => attempt.follow(),
        }
    })
}

/// Run `send` with `guard` judging each redirect it follows.
pub(crate) async fn guarded<F: std::future::Future>(guard: Guard, send: F) -> F::Output {
    GUARD.scope(guard, send).await
}

impl Guard {
    fn check(&self, status: StatusCode, next: &url::Url) -> Result<(), String> {
        let method = hop_method(&self.method, status);
        if let Some(policy) = &self.policy {
            policy.check_http(&method, next.as_str())?;
        }
        let Some(gate) = self.gate else {
            return Ok(());
        };
        let keeps_body = method == self.method;
        // Every original header is assumed to travel. reqwest drops
        // `Authorization` on a cross-origin hop, but judging the hop as still
        // signed can only make it stricter.
        let call = HttpCall {
            url: Some(next.to_string()),
            headers: self.headers.clone(),
            headers_collide: false,
            body: self.body.clone().filter(|_| keeps_body),
            method,
        };
        if gated::call_is_read(self.policy.as_deref(), &call) {
            return Ok(());
        }
        let host = next.host_str().unwrap_or("?");
        let reason = gated::call_write_reason(self.policy.as_deref(), &call)
            .map(|reason| format!(" ({reason})"))
            .unwrap_or_default();
        Err(format!("{gate}: redirect to {host} blocked{reason}"))
    }
}

/// The method reqwest re-sends a hop with.
fn hop_method(method: &str, status: StatusCode) -> String {
    let to_get = match status {
        StatusCode::SEE_OTHER => !method.eq_ignore_ascii_case("head"),
        StatusCode::MOVED_PERMANENTLY | StatusCode::FOUND => method.eq_ignore_ascii_case("post"),
        _ => false,
    };
    if to_get {
        "get".to_string()
    } else {
        method.to_ascii_lowercase()
    }
}
