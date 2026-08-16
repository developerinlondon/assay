//! KV v2 read handlers — `data` reads, `metadata` reads, and LIST.

use std::collections::BTreeSet;

use axum::Router;
use axum::extract::{Extension, FromRef, Path, Query, State};
use axum::http::{Method, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{any, get};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::ctx::{DynKvStore, VaultCtx};
use crate::hashicorp_compat::{
    Mount, envelope, errors, kv_unconfigured, normalize_path, not_found, rfc3339, vault_error,
};
use crate::kv::{KvMeta, KvRead, KvService};

pub(super) fn router<S>() -> Router<S>
where
    S: Clone + Send + Sync + 'static,
    VaultCtx: FromRef<S>,
{
    Router::new()
        .route("/v1/{mount}/data/{*path}", get(read_data::<S>))
        .route("/v1/{mount}/metadata/{*path}", any(metadata::<S>))
        .route("/v1/{mount}/metadata", any(metadata_root::<S>))
}

#[derive(Deserialize)]
struct ReadQuery {
    version: Option<i64>,
}

#[derive(Deserialize)]
struct ListQuery {
    list: Option<String>,
}

impl ListQuery {
    fn wants_list(&self) -> bool {
        self.list
            .as_deref()
            .is_some_and(|v| v.eq_ignore_ascii_case("true") || v == "1")
    }
}

async fn read_data<S>(
    State(vault): State<VaultCtx>,
    Extension(mount): Extension<Mount>,
    Path((requested_mount, path)): Path<(String, String)>,
    Query(q): Query<ReadQuery>,
) -> Response
where
    S: Clone + Send + Sync + 'static,
    VaultCtx: FromRef<S>,
{
    let kv = match resolve(&vault, &requested_mount, &mount) {
        Ok(kv) => kv,
        Err(response) => return *response,
    };
    let path = normalize_path(&path);
    let read = match kv.get(path, q.version).await {
        Ok(read) => read,
        Err(e) => return vault_error(e),
    };
    let metadata = version_metadata(&read, custom_metadata(kv, path).await);

    // A soft-deleted version answers 404 while still describing itself, so a
    // caller can tell "deleted at T" from "never existed".
    if read.deleted_at.is_some() {
        let body = envelope(json!({ "data": Value::Null, "metadata": metadata }));
        return (StatusCode::NOT_FOUND, axum::Json(body)).into_response();
    }

    let payload = match String::from_utf8(read.plaintext) {
        Ok(text) => payload_object(text),
        Err(_) => {
            return errors(
                StatusCode::INTERNAL_SERVER_ERROR,
                &["stored payload is not valid UTF-8"],
            );
        }
    };
    axum::Json(envelope(json!({ "data": payload, "metadata": metadata }))).into_response()
}

async fn metadata<S>(
    method: Method,
    State(vault): State<VaultCtx>,
    Extension(mount): Extension<Mount>,
    Path((requested_mount, path)): Path<(String, String)>,
    Query(q): Query<ListQuery>,
) -> Response
where
    S: Clone + Send + Sync + 'static,
    VaultCtx: FromRef<S>,
{
    metadata_impl(method, vault, requested_mount, mount, path, q).await
}

async fn metadata_root<S>(
    method: Method,
    State(vault): State<VaultCtx>,
    Extension(mount): Extension<Mount>,
    Path(requested_mount): Path<String>,
    Query(q): Query<ListQuery>,
) -> Response
where
    S: Clone + Send + Sync + 'static,
    VaultCtx: FromRef<S>,
{
    metadata_impl(method, vault, requested_mount, mount, String::new(), q).await
}

async fn metadata_impl(
    method: Method,
    vault: VaultCtx,
    requested_mount: String,
    mount: Mount,
    path: String,
    q: ListQuery,
) -> Response {
    let is_list = method.as_str() == "LIST" || (method == Method::GET && q.wants_list());
    if !is_list && method != Method::GET {
        return StatusCode::METHOD_NOT_ALLOWED.into_response();
    }
    let kv = match resolve(&vault, &requested_mount, &mount) {
        Ok(kv) => kv,
        Err(response) => return *response,
    };
    let path = normalize_path(&path);
    if is_list {
        list_children(kv, path).await
    } else {
        read_metadata(kv, path).await
    }
}

async fn read_metadata(kv: &KvService<DynKvStore>, path: &str) -> Response {
    match kv.read_meta(path).await {
        Ok(meta) => axum::Json(envelope(path_metadata(&meta))).into_response(),
        Err(e) => vault_error(e),
    }
}

async fn list_children(kv: &KvService<DynKvStore>, path: &str) -> Response {
    let prefix = if path.is_empty() {
        String::new()
    } else {
        format!("{path}/")
    };
    let metas = match kv.list(&prefix).await {
        Ok(metas) => metas,
        Err(e) => return vault_error(e),
    };
    let keys = fold_children(&prefix, &metas);
    if keys.is_empty() {
        return not_found();
    }
    axum::Json(envelope(json!({ "keys": keys }))).into_response()
}

/// Vault lists one level at a time: a key with deeper segments collapses to
/// its first segment with a trailing slash, the way a directory listing reads.
fn fold_children(prefix: &str, metas: &[KvMeta]) -> Vec<String> {
    let mut keys = BTreeSet::new();
    for meta in metas {
        let Some(rest) = meta.path.strip_prefix(prefix) else {
            continue;
        };
        match rest.split_once('/') {
            Some((head, _)) if !head.is_empty() => keys.insert(format!("{head}/")),
            Some(_) => continue,
            None if rest.is_empty() => continue,
            None => keys.insert(rest.to_string()),
        };
    }
    keys.into_iter().collect()
}

/// assay stores an opaque UTF-8 payload per version; KV2 hands back an
/// object. A payload that is not a JSON object is still reachable, under the
/// `value` key, rather than being an unreadable secret.
fn payload_object(text: String) -> Value {
    match serde_json::from_str::<Value>(&text) {
        Ok(Value::Object(map)) => Value::Object(map),
        _ => json!({ "value": text }),
    }
}

fn version_metadata(read: &KvRead, custom_metadata: Value) -> Value {
    json!({
        "created_time": rfc3339(read.created_at),
        "custom_metadata": custom_metadata,
        "deletion_time": read.deleted_at.map(rfc3339).unwrap_or_default(),
        "destroyed": false,
        "version": read.version,
    })
}

/// assay records version history at the path level, not per version, so
/// `versions` describes the current version alone. `max_versions: 0` and
/// `cas_required: false` are Vault's own defaults for a mount with neither
/// configured, which is exactly this one.
fn path_metadata(meta: &KvMeta) -> Value {
    json!({
        "cas_required": false,
        "created_time": rfc3339(meta.created_at),
        "current_version": meta.latest_version,
        "custom_metadata": custom_md_or_null(meta.custom_md.clone()),
        "delete_version_after": "0s",
        "max_versions": 0,
        "oldest_version": 0,
        "updated_time": rfc3339(meta.updated_at),
        "versions": {
            meta.latest_version.to_string(): {
                "created_time": rfc3339(meta.updated_at),
                "deletion_time": "",
                "destroyed": false,
            }
        },
    })
}

async fn custom_metadata(kv: &KvService<DynKvStore>, path: &str) -> Value {
    match kv.read_meta(path).await {
        Ok(meta) => custom_md_or_null(meta.custom_md),
        Err(_) => Value::Null,
    }
}

fn custom_md_or_null(value: Value) -> Value {
    match value {
        Value::Object(map) if map.is_empty() => Value::Null,
        other => other,
    }
}

/// Boxed error, matching `assay_auth::gate` — an `axum::Response` is large
/// enough that clippy refuses it as a bare `Err` variant.
fn resolve<'a>(
    vault: &'a VaultCtx,
    requested: &str,
    mount: &Mount,
) -> Result<&'a KvService<DynKvStore>, Box<Response>> {
    if requested != mount.name() {
        return Err(Box::new(not_found()));
    }
    vault
        .kv
        .as_ref()
        .ok_or_else(|| Box::new(kv_unconfigured()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn meta(path: &str) -> KvMeta {
        KvMeta {
            path: path.to_string(),
            latest_version: 1,
            custom_md: json!({}),
            created_at: 0.0,
            updated_at: 0.0,
        }
    }

    #[test]
    fn list_collapses_deeper_keys_into_one_directory_entry() {
        let metas = [
            meta("platform/postgres"),
            meta("platform/redis"),
            meta("platform/immich/db"),
            meta("platform/immich/smtp"),
        ];
        assert_eq!(
            fold_children("platform/", &metas),
            vec!["immich/", "postgres", "redis"]
        );
    }

    #[test]
    fn list_at_the_root_reports_top_level_segments_only() {
        let metas = [meta("platform/postgres"), meta("apps/neutron/db")];
        assert_eq!(fold_children("", &metas), vec!["apps/", "platform/"]);
    }

    #[test]
    fn a_row_the_store_returned_outside_the_prefix_is_dropped() {
        let metas = [meta("platformx/postgres")];
        assert!(fold_children("platform/", &metas).is_empty());
    }

    #[test]
    fn a_json_object_payload_is_served_field_by_field() {
        let out = payload_object(r#"{"username":"app","password":"hunter2"}"#.to_string());
        assert_eq!(out["username"], json!("app"));
        assert_eq!(out["password"], json!("hunter2"));
    }

    #[test]
    fn a_payload_that_is_not_an_object_is_reachable_under_value() {
        assert_eq!(
            payload_object("sk_live_xxx".into()),
            json!({"value": "sk_live_xxx"})
        );
        assert_eq!(payload_object("[1,2]".into()), json!({"value": "[1,2]"}));
        assert_eq!(payload_object("42".into()), json!({"value": "42"}));
    }

    #[test]
    fn empty_custom_metadata_is_reported_the_way_vault_reports_it() {
        assert_eq!(custom_md_or_null(json!({})), Value::Null);
        assert_eq!(custom_md_or_null(json!({"owner": "sre"}))["owner"], "sre");
    }

    #[test]
    fn only_an_explicit_list_flag_turns_a_get_into_a_listing() {
        let q = |v: Option<&str>| ListQuery {
            list: v.map(str::to_string),
        };
        assert!(q(Some("true")).wants_list());
        assert!(q(Some("TRUE")).wants_list());
        assert!(q(Some("1")).wants_list());
        assert!(!q(Some("false")).wants_list());
        assert!(!q(None).wants_list());
    }
}
