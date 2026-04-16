use std::sync::Arc;

use axum::extract::{Path, State};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;
use utoipa::ToSchema;

use crate::api::workflows::AppError;
use crate::api::AppState;
use crate::store::WorkflowStore;

pub fn router<S: WorkflowStore + 'static>() -> Router<Arc<AppState<S>>> {
    Router::new()
        .route("/namespaces", post(create_namespace).get(list_namespaces))
        .route(
            "/namespaces/{name}",
            get(get_namespace_stats).delete(delete_namespace),
        )
}

#[derive(Deserialize, ToSchema)]
pub struct CreateNamespaceRequest {
    pub name: String,
}

#[utoipa::path(
    post, path = "/api/v1/namespaces",
    tag = "namespaces",
    request_body = CreateNamespaceRequest,
    responses(
        (status = 201, description = "Namespace created"),
        (status = 500, description = "Internal error"),
    ),
)]
pub async fn create_namespace<S: WorkflowStore>(
    State(state): State<Arc<AppState<S>>>,
    Json(req): Json<CreateNamespaceRequest>,
) -> Result<axum::http::StatusCode, AppError> {
    state.engine.create_namespace(&req.name).await?;
    Ok(axum::http::StatusCode::CREATED)
}

#[utoipa::path(
    get, path = "/api/v1/namespaces",
    tag = "namespaces",
    responses(
        (status = 200, description = "List of namespaces", body = Vec<NamespaceRecord>),
    ),
)]
pub async fn list_namespaces<S: WorkflowStore>(
    State(state): State<Arc<AppState<S>>>,
) -> Result<Json<Vec<serde_json::Value>>, AppError> {
    let namespaces = state.engine.list_namespaces().await?;
    let json: Vec<serde_json::Value> = namespaces
        .into_iter()
        .map(|n| serde_json::to_value(n).unwrap_or_default())
        .collect();
    Ok(Json(json))
}

#[utoipa::path(
    get, path = "/api/v1/namespaces/{name}",
    tag = "namespaces",
    params(("name" = String, Path, description = "Namespace name")),
    responses(
        (status = 200, description = "Namespace statistics", body = NamespaceStats),
    ),
)]
pub async fn get_namespace_stats<S: WorkflowStore>(
    State(state): State<Arc<AppState<S>>>,
    Path(name): Path<String>,
) -> Result<Json<serde_json::Value>, AppError> {
    let stats = state.engine.get_namespace_stats(&name).await?;
    Ok(Json(serde_json::to_value(stats)?))
}

#[utoipa::path(
    delete, path = "/api/v1/namespaces/{name}",
    tag = "namespaces",
    params(("name" = String, Path, description = "Namespace name")),
    responses(
        (status = 200, description = "Namespace deleted"),
        (status = 404, description = "Namespace not found or is 'main'"),
    ),
)]
pub async fn delete_namespace<S: WorkflowStore>(
    State(state): State<Arc<AppState<S>>>,
    Path(name): Path<String>,
) -> Result<axum::http::StatusCode, AppError> {
    if name == "main" {
        return Err(AppError::NotFound(
            "cannot delete the 'main' namespace".to_string(),
        ));
    }
    let deleted = state.engine.delete_namespace(&name).await?;
    if deleted {
        Ok(axum::http::StatusCode::OK)
    } else {
        Err(AppError::NotFound(format!("namespace {name}")))
    }
}

use crate::store::{NamespaceRecord, NamespaceStats};
