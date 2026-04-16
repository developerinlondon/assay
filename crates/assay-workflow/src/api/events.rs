use std::convert::Infallible;
use std::sync::Arc;

use axum::extract::State;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::routing::get;
use axum::Router;
use serde::{Deserialize, Serialize};
use tokio_stream::wrappers::BroadcastStream;
use tokio_stream::StreamExt;

use crate::api::AppState;
use crate::store::WorkflowStore;

/// Event broadcast to SSE subscribers.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BroadcastEvent {
    pub event_type: String,
    pub workflow_id: String,
    pub payload: Option<String>,
}

pub fn router<S: WorkflowStore + 'static>() -> Router<Arc<AppState<S>>> {
    Router::new().route("/events/stream", get(event_stream))
}

async fn event_stream<S: WorkflowStore>(
    State(state): State<Arc<AppState<S>>>,
) -> Sse<impl tokio_stream::Stream<Item = Result<Event, Infallible>>> {
    let rx = state.event_tx.subscribe();

    let stream =
        BroadcastStream::new(rx).filter_map(|result: Result<BroadcastEvent, _>| {
            result.ok().map(|evt| {
                let data = serde_json::to_string(&evt).unwrap_or_default();
                Ok(Event::default().event(&evt.event_type).data(data))
            })
        });

    Sse::new(stream).keep_alive(KeepAlive::default())
}
