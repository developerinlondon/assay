//! SSE endpoint. Phase 7 (plan 13f) rewrites this to support cursor
//! replay, 410 Gone, and server-side filters. During phase 5 the
//! handler just subscribes to the engine-wide bus and forwards every
//! event on the requested namespace as-is so existing dashboards keep
//! working while the cutover is in flight.

use std::convert::Infallible;
use std::sync::Arc;

use axum::Router;
use axum::extract::State;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::routing::get;
use tokio_stream::StreamExt;
use tokio_stream::wrappers::BroadcastStream;

use crate::ctx::WorkflowCtx;
use crate::store::WorkflowStore;

pub fn router<S: WorkflowStore + 'static>() -> Router<Arc<WorkflowCtx<S>>> {
    Router::new().route("/events/stream", get(event_stream))
}

async fn event_stream<S: WorkflowStore>(
    State(state): State<Arc<WorkflowCtx<S>>>,
) -> Sse<impl tokio_stream::Stream<Item = Result<Event, Infallible>>> {
    let stream: std::pin::Pin<
        Box<dyn tokio_stream::Stream<Item = Result<Event, Infallible>> + Send>,
    > = if let Some(bus) = state.bus() {
        let rx = bus.inner().subscribe("main");
        let s = BroadcastStream::new(rx).filter_map(|result| {
            result.ok().map(|arc_ev| {
                let ev = (*arc_ev).clone();
                let data = serde_json::json!({
                    "event_type": ev.kind,
                    "workflow_id": ev.payload.get("workflow_id").and_then(|v| v.as_str()).unwrap_or_default(),
                    "payload": serde_json::json!({
                        "namespace": ev.namespace,
                    }).to_string(),
                })
                .to_string();
                Ok(Event::default().event(&ev.kind).data(data))
            })
        });
        Box::pin(s)
    } else {
        Box::pin(tokio_stream::empty())
    };

    Sse::new(stream).keep_alive(KeepAlive::default())
}
