mod common;

use common::run_lua;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn test_require_velero() {
    let script = r#"
        local velero = require("assay.velero")
        assert.not_nil(velero)
        assert.not_nil(velero.client)
    "#;
    run_lua(script).await.unwrap();
}

#[tokio::test]
async fn test_velero_backups_list() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/apis/velero.io/v1/namespaces/velero/backups"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "apiVersion": "velero.io/v1",
            "kind": "BackupList",
            "items": [
                {
                    "apiVersion": "velero.io/v1",
                    "kind": "Backup",
                    "metadata": {"name": "daily-20260210", "namespace": "velero"},
                    "status": {"phase": "Completed"}
                },
                {
                    "apiVersion": "velero.io/v1",
                    "kind": "Backup",
                    "metadata": {"name": "daily-20260209", "namespace": "velero"},
                    "status": {"phase": "Completed"}
                }
            ]
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local velero = require("assay.velero")
        local c = velero.client("{}", "fake-token")
        local backups = c:backups()
        assert.eq(#backups, 2)
        assert.eq(backups[1].metadata.name, "daily-20260210")
        assert.eq(backups[2].metadata.name, "daily-20260209")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_velero_backup_get() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(
            "/apis/velero.io/v1/namespaces/velero/backups/daily-20260210",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "apiVersion": "velero.io/v1",
            "kind": "Backup",
            "metadata": {"name": "daily-20260210", "namespace": "velero"},
            "spec": {
                "includedNamespaces": ["jeebon-test"],
                "storageLocation": "default",
                "ttl": "720h0m0s"
            },
            "status": {
                "phase": "Completed",
                "startTimestamp": "2026-02-10T02:00:00Z",
                "completionTimestamp": "2026-02-10T02:05:30Z",
                "expiration": "2026-03-12T02:00:00Z",
                "errors": 0,
                "warnings": 1,
                "itemsBackedUp": 142,
                "totalItems": 142
            }
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local velero = require("assay.velero")
        local c = velero.client("{}", "fake-token")
        local b = c:backup("daily-20260210")
        assert.eq(b.metadata.name, "daily-20260210")
        assert.eq(b.spec.storageLocation, "default")
        assert.eq(b.status.phase, "Completed")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_velero_backup_status() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(
            "/apis/velero.io/v1/namespaces/velero/backups/daily-20260210",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "apiVersion": "velero.io/v1",
            "kind": "Backup",
            "metadata": {"name": "daily-20260210", "namespace": "velero"},
            "status": {
                "phase": "Completed",
                "startTimestamp": "2026-02-10T02:00:00Z",
                "completionTimestamp": "2026-02-10T02:05:30Z",
                "expiration": "2026-03-12T02:00:00Z",
                "errors": 0,
                "warnings": 2,
                "itemsBackedUp": 142,
                "totalItems": 150
            }
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local velero = require("assay.velero")
        local c = velero.client("{}", "fake-token")
        local s = c:backup_status("daily-20260210")
        assert.eq(s.phase, "Completed")
        assert.eq(s.started, "2026-02-10T02:00:00Z")
        assert.eq(s.completed, "2026-02-10T02:05:30Z")
        assert.eq(s.expiration, "2026-03-12T02:00:00Z")
        assert.eq(s.errors, 0)
        assert.eq(s.warnings, 2)
        assert.eq(s.items_backed_up, 142)
        assert.eq(s.items_total, 150)
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_velero_is_backup_completed_true() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(
            "/apis/velero.io/v1/namespaces/velero/backups/daily-20260210",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "apiVersion": "velero.io/v1",
            "kind": "Backup",
            "metadata": {"name": "daily-20260210"},
            "status": {"phase": "Completed"}
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local velero = require("assay.velero")
        local c = velero.client("{}", "fake-token")
        assert.eq(c:is_backup_completed("daily-20260210"), true)
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_velero_is_backup_completed_false() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(
            "/apis/velero.io/v1/namespaces/velero/backups/daily-20260210",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "apiVersion": "velero.io/v1",
            "kind": "Backup",
            "metadata": {"name": "daily-20260210"},
            "status": {"phase": "InProgress"}
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local velero = require("assay.velero")
        local c = velero.client("{}", "fake-token")
        assert.eq(c:is_backup_completed("daily-20260210"), false)
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_velero_is_backup_failed() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(
            "/apis/velero.io/v1/namespaces/velero/backups/broken-backup",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "apiVersion": "velero.io/v1",
            "kind": "Backup",
            "metadata": {"name": "broken-backup"},
            "status": {"phase": "Failed"}
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local velero = require("assay.velero")
        local c = velero.client("{}", "fake-token")
        assert.eq(c:is_backup_failed("broken-backup"), true)
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_velero_is_backup_failed_partially() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(
            "/apis/velero.io/v1/namespaces/velero/backups/partial-backup",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "apiVersion": "velero.io/v1",
            "kind": "Backup",
            "metadata": {"name": "partial-backup"},
            "status": {"phase": "PartiallyFailed"}
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local velero = require("assay.velero")
        local c = velero.client("{}", "fake-token")
        assert.eq(c:is_backup_failed("partial-backup"), true)
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_velero_latest_backup() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/apis/velero.io/v1/namespaces/velero/backups"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "apiVersion": "velero.io/v1",
            "kind": "BackupList",
            "items": [
                {
                    "metadata": {
                        "name": "daily-20260208",
                        "creationTimestamp": "2026-02-08T02:00:00Z",
                        "labels": {"velero.io/schedule-name": "daily"}
                    },
                    "status": {"phase": "Completed"}
                },
                {
                    "metadata": {
                        "name": "daily-20260210",
                        "creationTimestamp": "2026-02-10T02:00:00Z",
                        "labels": {"velero.io/schedule-name": "daily"}
                    },
                    "status": {"phase": "Completed"}
                },
                {
                    "metadata": {
                        "name": "daily-20260209",
                        "creationTimestamp": "2026-02-09T02:00:00Z",
                        "labels": {"velero.io/schedule-name": "daily"}
                    },
                    "status": {"phase": "Completed"}
                },
                {
                    "metadata": {
                        "name": "weekly-20260207",
                        "creationTimestamp": "2026-02-07T02:00:00Z",
                        "labels": {"velero.io/schedule-name": "weekly"}
                    },
                    "status": {"phase": "Completed"}
                }
            ]
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local velero = require("assay.velero")
        local c = velero.client("{}", "fake-token")
        local latest = c:latest_backup("daily")
        assert.not_nil(latest)
        assert.eq(latest.metadata.name, "daily-20260210")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_velero_latest_backup_no_match() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/apis/velero.io/v1/namespaces/velero/backups"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "apiVersion": "velero.io/v1",
            "kind": "BackupList",
            "items": [
                {
                    "metadata": {
                        "name": "daily-20260210",
                        "creationTimestamp": "2026-02-10T02:00:00Z",
                        "labels": {"velero.io/schedule-name": "daily"}
                    },
                    "status": {"phase": "Completed"}
                }
            ]
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local velero = require("assay.velero")
        local c = velero.client("{}", "fake-token")
        local latest = c:latest_backup("nonexistent-schedule")
        assert.eq(latest, nil)
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_velero_restores_list() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/apis/velero.io/v1/namespaces/velero/restores"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "apiVersion": "velero.io/v1",
            "kind": "RestoreList",
            "items": [
                {
                    "apiVersion": "velero.io/v1",
                    "kind": "Restore",
                    "metadata": {"name": "restore-20260210", "namespace": "velero"},
                    "status": {"phase": "Completed"}
                }
            ]
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local velero = require("assay.velero")
        local c = velero.client("{}", "fake-token")
        local restores = c:restores()
        assert.eq(#restores, 1)
        assert.eq(restores[1].metadata.name, "restore-20260210")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_velero_restore_status() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(
            "/apis/velero.io/v1/namespaces/velero/restores/restore-20260210",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "apiVersion": "velero.io/v1",
            "kind": "Restore",
            "metadata": {"name": "restore-20260210", "namespace": "velero"},
            "status": {
                "phase": "Completed",
                "startTimestamp": "2026-02-10T10:00:00Z",
                "completionTimestamp": "2026-02-10T10:12:00Z",
                "errors": 0,
                "warnings": 3
            }
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local velero = require("assay.velero")
        local c = velero.client("{}", "fake-token")
        local s = c:restore_status("restore-20260210")
        assert.eq(s.phase, "Completed")
        assert.eq(s.started, "2026-02-10T10:00:00Z")
        assert.eq(s.completed, "2026-02-10T10:12:00Z")
        assert.eq(s.errors, 0)
        assert.eq(s.warnings, 3)
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_velero_is_restore_completed() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(
            "/apis/velero.io/v1/namespaces/velero/restores/restore-20260210",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "apiVersion": "velero.io/v1",
            "kind": "Restore",
            "metadata": {"name": "restore-20260210"},
            "status": {"phase": "Completed"}
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local velero = require("assay.velero")
        local c = velero.client("{}", "fake-token")
        assert.eq(c:is_restore_completed("restore-20260210"), true)
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_velero_is_restore_completed_false() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(
            "/apis/velero.io/v1/namespaces/velero/restores/restore-20260210",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "apiVersion": "velero.io/v1",
            "kind": "Restore",
            "metadata": {"name": "restore-20260210"},
            "status": {"phase": "InProgress"}
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local velero = require("assay.velero")
        local c = velero.client("{}", "fake-token")
        assert.eq(c:is_restore_completed("restore-20260210"), false)
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_velero_schedules_list() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/apis/velero.io/v1/namespaces/velero/schedules"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "apiVersion": "velero.io/v1",
            "kind": "ScheduleList",
            "items": [
                {
                    "apiVersion": "velero.io/v1",
                    "kind": "Schedule",
                    "metadata": {"name": "daily", "namespace": "velero"},
                    "spec": {"schedule": "0 2 * * *"},
                    "status": {"phase": "Enabled", "lastBackup": "2026-02-10T02:00:00Z"}
                },
                {
                    "apiVersion": "velero.io/v1",
                    "kind": "Schedule",
                    "metadata": {"name": "weekly", "namespace": "velero"},
                    "spec": {"schedule": "0 3 * * 0"},
                    "status": {"phase": "Enabled", "lastBackup": "2026-02-09T03:00:00Z"}
                }
            ]
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local velero = require("assay.velero")
        local c = velero.client("{}", "fake-token")
        local schedules = c:schedules()
        assert.eq(#schedules, 2)
        assert.eq(schedules[1].metadata.name, "daily")
        assert.eq(schedules[2].metadata.name, "weekly")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_velero_schedule_status() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(
            "/apis/velero.io/v1/namespaces/velero/schedules/daily",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "apiVersion": "velero.io/v1",
            "kind": "Schedule",
            "metadata": {"name": "daily", "namespace": "velero"},
            "status": {
                "phase": "Enabled",
                "lastBackup": "2026-02-10T02:00:00Z",
                "validationErrors": []
            }
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local velero = require("assay.velero")
        local c = velero.client("{}", "fake-token")
        local s = c:schedule_status("daily")
        assert.eq(s.phase, "Enabled")
        assert.eq(s.last_backup, "2026-02-10T02:00:00Z")
        assert.eq(#s.validation_errors, 0)
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_velero_is_schedule_enabled_true() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(
            "/apis/velero.io/v1/namespaces/velero/schedules/daily",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "apiVersion": "velero.io/v1",
            "kind": "Schedule",
            "metadata": {"name": "daily"},
            "status": {"phase": "Enabled"}
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local velero = require("assay.velero")
        local c = velero.client("{}", "fake-token")
        assert.eq(c:is_schedule_enabled("daily"), true)
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_velero_is_schedule_enabled_false() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(
            "/apis/velero.io/v1/namespaces/velero/schedules/paused",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "apiVersion": "velero.io/v1",
            "kind": "Schedule",
            "metadata": {"name": "paused"},
            "status": {"phase": "FailedValidation"}
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local velero = require("assay.velero")
        local c = velero.client("{}", "fake-token")
        assert.eq(c:is_schedule_enabled("paused"), false)
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_velero_backup_storage_locations() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(
            "/apis/velero.io/v1/namespaces/velero/backupstoragelocations",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "apiVersion": "velero.io/v1",
            "kind": "BackupStorageLocationList",
            "items": [
                {
                    "apiVersion": "velero.io/v1",
                    "kind": "BackupStorageLocation",
                    "metadata": {"name": "default", "namespace": "velero"},
                    "spec": {"provider": "aws", "objectStorage": {"bucket": "velero-backups"}},
                    "status": {"phase": "Available", "lastSyncedTime": "2026-02-10T12:00:00Z"}
                }
            ]
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local velero = require("assay.velero")
        local c = velero.client("{}", "fake-token")
        local bsls = c:backup_storage_locations()
        assert.eq(#bsls, 1)
        assert.eq(bsls[1].metadata.name, "default")
        assert.eq(bsls[1].spec.provider, "aws")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_velero_is_bsl_available_true() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(
            "/apis/velero.io/v1/namespaces/velero/backupstoragelocations/default",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "apiVersion": "velero.io/v1",
            "kind": "BackupStorageLocation",
            "metadata": {"name": "default"},
            "status": {"phase": "Available"}
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local velero = require("assay.velero")
        local c = velero.client("{}", "fake-token")
        assert.eq(c:is_bsl_available("default"), true)
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_velero_is_bsl_available_false() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(
            "/apis/velero.io/v1/namespaces/velero/backupstoragelocations/broken",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "apiVersion": "velero.io/v1",
            "kind": "BackupStorageLocation",
            "metadata": {"name": "broken"},
            "status": {"phase": "Unavailable"}
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local velero = require("assay.velero")
        local c = velero.client("{}", "fake-token")
        assert.eq(c:is_bsl_available("broken"), false)
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_velero_all_schedules_enabled() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/apis/velero.io/v1/namespaces/velero/schedules"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "apiVersion": "velero.io/v1",
            "kind": "ScheduleList",
            "items": [
                {
                    "metadata": {"name": "daily"},
                    "status": {"phase": "Enabled"}
                },
                {
                    "metadata": {"name": "weekly"},
                    "status": {"phase": "Enabled"}
                },
                {
                    "metadata": {"name": "broken-schedule"},
                    "status": {"phase": "FailedValidation"}
                }
            ]
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local velero = require("assay.velero")
        local c = velero.client("{}", "fake-token")
        local result = c:all_schedules_enabled()
        assert.eq(result.enabled, 2)
        assert.eq(result.disabled, 1)
        assert.eq(result.total, 3)
        assert.eq(result.disabled_names[1], "broken-schedule")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_velero_all_bsl_available() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(
            "/apis/velero.io/v1/namespaces/velero/backupstoragelocations",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "apiVersion": "velero.io/v1",
            "kind": "BackupStorageLocationList",
            "items": [
                {
                    "metadata": {"name": "default"},
                    "status": {"phase": "Available"}
                },
                {
                    "metadata": {"name": "secondary"},
                    "status": {"phase": "Unavailable"}
                }
            ]
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local velero = require("assay.velero")
        local c = velero.client("{}", "fake-token")
        local result = c:all_bsl_available()
        assert.eq(result.available, 1)
        assert.eq(result.unavailable, 1)
        assert.eq(result.total, 2)
        assert.eq(result.unavailable_names[1], "secondary")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_velero_custom_namespace() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(
            "/apis/velero.io/v1/namespaces/custom-ns/backups",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "apiVersion": "velero.io/v1",
            "kind": "BackupList",
            "items": [
                {
                    "metadata": {"name": "backup-1", "namespace": "custom-ns"},
                    "status": {"phase": "Completed"}
                }
            ]
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local velero = require("assay.velero")
        local c = velero.client("{}", "fake-token", "custom-ns")
        local backups = c:backups()
        assert.eq(#backups, 1)
        assert.eq(backups[1].metadata.name, "backup-1")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}
