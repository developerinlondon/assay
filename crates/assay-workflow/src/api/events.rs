use std::convert::Infallible;
use std::sync::Arc;

use axum::extract::State;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::routing::get;
use axum::Router;
use tokio_stream::wrappers::BroadcastStream;
use tokio_stream::StreamExt;

use crate::ctx::{BroadcastEvent, WorkflowCtx};
use crate::store::WorkflowStore;

// Re-export so `api/mod.rs` can use it without a separate path.
pub use crate::ctx::BroadcastEvent as BroadcastEventAlias;

pub fn router<S: WorkflowStore + 'static>() -> Router<Arc<WorkflowCtx<S>>> {
    Router::new().route("/events/stream", get(event_stream))
}

async fn event_stream<S: WorkflowStore>(
    State(state): State<Arc<WorkflowCtx<S>>>,
) -> Sse<impl tokio_stream::Stream<Item = Result<Event, Infallible>>> {
    // Subscribe to the SSE sender stored in ctx; fall back to an empty
    // stream when no broadcaster is wired (tests that don't need SSE).
    let stream = if let Some(ref tx) = state.sse_tx {
        let rx = tx.subscribe();
        let s = BroadcastStream::new(rx).filter_map(|result: Result<BroadcastEvent, _>| {
            result.ok().map(|evt| {
                let data = serde_json::to_string(&evt).unwrap_or_default();
                Ok(Event::default().event(&evt.event_type).data(data))
            })
        });
        Box::pin(s) as std::pin::Pin<Box<dyn tokio_stream::Stream<Item = Result<Event, Infallible>> + Send>>
    } else {
        Box::pin(tokio_stream::empty()) as std::pin::Pin<Box<dyn tokio_stream::Stream<Item = Result<Event, Infallible>> + Send>>
    };

    Sse::new(stream).keep_alive(KeepAlive::default())
}
