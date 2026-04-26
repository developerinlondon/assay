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

/// Build the workflow HTTP API router. Auth is enforced at the engine
/// layer (via [`assay_auth::gate`] in `assay_engine::server`); this
/// router carries no gate of its own. `health`, `version`, `openapi.json`,
/// and `docs` are always public so probes can reach them without a
/// bearer token.
pub fn router<S: WorkflowStore>(state: Arc<WorkflowCtx<S>>) -> Router {
    let authed_api = Router::new()
        .nest("/api/v1/engine/workflow", api_v1_router::<S>())
        .nest("/api/v1/engine/workflow", events::router::<S>());

    let public_api = Router::new().nest("/api/v1/engine/workflow", public::router::<S>());

    let app = authed_api.merge(public_api).merge(openapi::router::<S>());

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
