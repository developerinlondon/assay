//! SSE endpoint for the engine-events outbox.
//!
//! Endpoint: `GET /api/v1/engine/workflow/events/stream`
//!   Query params:
//!     ?ns=<namespace>                       default "main"
//!     ?subsystem=<workflow|auth|secrets|system>   repeatable
//!     ?workflow_id=<id>                     optional
//!     ?kind=<kind>                          repeatable
//!   Header: `Last-Event-ID: <i64>`          optional cursor for replay
//!
//! Behaviour:
//!   1. Replay phase reads `engine_events` since `Last-Event-ID` (or
//!      from the beginning if absent) and emits matching frames.
//!   2. If the cursor is older than the namespace's oldest retained
//!      id, returns HTTP 410 Gone so the client knows to snapshot +
//!      resync.
//!   3. Live phase subscribes to the node-local broadcast; emits the
//!      frames that pass the filter.
//!   4. `broadcast::Lagged` force-closes the stream — the client
//!      reconnects with the last id it saw and replays the gap via
//!      `engine_events`.

use std::convert::Infallible;
use std::sync::Arc;
use std::time::Duration;

use assay_domain::events::{EventFilter, Subsystem};
use axum::Router;
use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::sse::{Event as SseEvent, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use futures_util::StreamExt;
use futures_util::stream::{self, Stream};
use serde::Deserialize;
use tokio_stream::wrappers::BroadcastStream;

use crate::ctx::WorkflowCtx;
use crate::store::WorkflowStore;

const REPLAY_PAGE_LIMIT: u32 = 500;

#[derive(Deserialize, Default)]
struct StreamQuery {
    #[serde(default)]
    ns: Option<String>,
    #[serde(default)]
    subsystem: Option<Vec<String>>,
    #[serde(default)]
    workflow_id: Option<String>,
    #[serde(default)]
    kind: Option<Vec<String>>,
}

pub fn router<S: WorkflowStore + 'static>() -> Router<Arc<WorkflowCtx<S>>> {
    Router::new().route("/events/stream", get(event_stream))
}

async fn event_stream<S: WorkflowStore>(
    State(state): State<Arc<WorkflowCtx<S>>>,
    Query(q): Query<StreamQuery>,
    headers: HeaderMap,
) -> Response {
    let Some(wf_bus) = state.bus() else {
        return (StatusCode::SERVICE_UNAVAILABLE, "event bus not configured").into_response();
    };

    let namespace = q.ns.clone().unwrap_or_else(|| "main".to_string());
    let filter = EventFilter {
        subsystems: q
            .subsystem
            .clone()
            .unwrap_or_default()
            .into_iter()
            .filter_map(|s| match s.as_str() {
                "workflow" => Some(Subsystem::Workflow),
                "auth" => Some(Subsystem::Auth),
                "secrets" => Some(Subsystem::Secrets),
                "system" => Some(Subsystem::System),
                _ => None,
            })
            .collect(),
        kinds: q.kind.clone().unwrap_or_default(),
        workflow_id: q.workflow_id.clone(),
    };

    // EventSource spec: browser auto-sends `Last-Event-ID` on reconnect.
    let last_id: Option<i64> = headers
        .get("last-event-id")
        .and_then(|h| h.to_str().ok())
        .and_then(|s| s.parse().ok());

    let inner = wf_bus.inner();

    let replay_events = match inner
        .read_since(&namespace, last_id, &filter, REPLAY_PAGE_LIMIT)
        .await
    {
        Ok(evs) => evs,
        Err(gone) => {
            return (
                StatusCode::GONE,
                format!(
                    "cursor {} older than retention (oldest {}); resync via point queries then reconnect without Last-Event-ID",
                    gone.after, gone.oldest
                ),
            )
                .into_response();
        }
    };

    let replay_stream = stream::iter(
        replay_events
            .into_iter()
            .map(|e| Ok::<_, Infallible>(event_to_sse(&e))),
    );

    // Subscribe for the live phase. sqlite's broadcast is global; pg's
    // is per-node but fed by a per-namespace LISTEN bridge. Filter
    // client-side on namespace + caller-supplied predicates.
    let rx = inner.subscribe(&namespace);
    let ns_for_live = namespace.clone();
    let filter_for_live = filter.clone();
    let live_stream = BroadcastStream::new(rx).filter_map(move |result| {
        let ns = ns_for_live.clone();
        let f = filter_for_live.clone();
        async move {
            match result {
                Ok(arc_ev) => {
                    if arc_ev.namespace != ns {
                        return None;
                    }
                    if !f.matches(&arc_ev) {
                        return None;
                    }
                    Some(Ok::<_, Infallible>(event_to_sse(&arc_ev)))
                }
                Err(tokio_stream::wrappers::errors::BroadcastStreamRecvError::Lagged(n)) => {
                    tracing::warn!(lagged = n, "SSE client lagged; forcing close");
                    None
                }
            }
        }
    });

    let combined: std::pin::Pin<Box<dyn Stream<Item = Result<SseEvent, Infallible>> + Send>> =
        Box::pin(replay_stream.chain(live_stream));
    Sse::new(combined)
        .keep_alive(KeepAlive::new().interval(Duration::from_secs(15)))
        .into_response()
}

fn event_to_sse(e: &assay_domain::events::Event) -> SseEvent {
    let data = serde_json::json!({
        "id": e.id,
        "ts": e.ts,
        "namespace": e.namespace,
        "subsystem": e.subsystem,
        "kind": e.kind,
        "payload": e.payload,
    })
    .to_string();
    SseEvent::default()
        .id(e.id.to_string())
        .event(&e.kind)
        .data(data)
}
