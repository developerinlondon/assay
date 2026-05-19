pub mod activities;
pub mod events;
pub mod namespaces;
pub mod openapi;
pub mod public;
pub mod queues;
pub mod schedules;
pub mod tasks;
pub mod workers;
pub mod workflow_tasks;
pub mod workflows;

use std::sync::Arc;

use axum::Router;

use crate::ctx::WorkflowCtx;
use crate::store::WorkflowStore;

/// Build the workflow HTTP API router. The `gate` argument is the
/// wire-boundary auth layer; the embedder supplies it as a closure
/// that wraps the authed portion of the router (typically
/// `|r| r.layer(my_auth_middleware)`). The type signature makes the
/// gate **non-optional** — you cannot construct an unauthenticated
/// workflow router.
///
/// The closure receives only the authed portion. `health`, `version`,
/// `openapi.json`, and `docs` are merged outside the gate so probes
/// can reach them without a bearer token.
pub fn router<S, F>(state: Arc<WorkflowCtx<S>>, gate: F) -> Router
where
    S: WorkflowStore,
    F: FnOnce(Router<Arc<WorkflowCtx<S>>>) -> Router<Arc<WorkflowCtx<S>>>,
{
    let authed_api = Router::new()
        .nest("/api/v1/engine/workflow", api_v1_router::<S>())
        .nest("/api/v1/engine/workflow", events::router::<S>());
    let gated = gate(authed_api);

    let public_api = Router::new().nest("/api/v1/engine/workflow", public::router::<S>());

    let app = gated.merge(public_api).merge(openapi::router::<S>());

    app.with_state(state)
}

fn api_v1_router<S: WorkflowStore>() -> Router<Arc<WorkflowCtx<S>>> {
    Router::new()
        .merge(workflows::router::<S>())
        .merge(activities::router::<S>())
        .merge(workflow_tasks::router::<S>())
        .merge(tasks::router::<S>())
        .merge(schedules::router::<S>())
        .merge(workers::router::<S>())
        .merge(namespaces::router::<S>())
        .merge(queues::router::<S>())
}
