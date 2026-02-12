mod common;

use common::run_lua;
use wiremock::matchers::{header, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn test_temporal_client_creation() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/health"))
        .respond_with(ResponseTemplate::new(200).set_body_string("OK"))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local temporal = require("assay.temporal")
        local c = temporal.client("{}")
        local ok = c:health()
        assert.eq(ok, true)
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_temporal_health() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/health"))
        .respond_with(ResponseTemplate::new(200).set_body_string("OK"))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local temporal = require("assay.temporal")
        local c = temporal.client("{}")
        assert.eq(c:health(), true)
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_temporal_health_unhealthy() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/health"))
        .respond_with(ResponseTemplate::new(503).set_body_string("Service Unavailable"))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local temporal = require("assay.temporal")
        local c = temporal.client("{}")
        assert.eq(c:health(), false)
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_temporal_system_info() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/system-info"))
        .and(header("Content-Type", "application/json"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "serverVersion": "1.24.0",
            "capabilities": {
                "signalAndQueryHeader": true,
                "internalErrorDifferentiation": true,
                "activityFailureIncludeHeartbeat": true,
                "supportsSchedules": true
            }
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local temporal = require("assay.temporal")
        local c = temporal.client("{}")
        local info = c:system_info()
        assert.eq(info.serverVersion, "1.24.0")
        assert.eq(info.capabilities.supportsSchedules, true)
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_temporal_namespaces() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/namespaces"))
        .and(header("Content-Type", "application/json"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "namespaces": [
                {
                    "namespaceInfo": {
                        "name": "default",
                        "state": "NAMESPACE_STATE_REGISTERED",
                        "description": "Default namespace"
                    }
                },
                {
                    "namespaceInfo": {
                        "name": "production",
                        "state": "NAMESPACE_STATE_REGISTERED",
                        "description": "Production namespace"
                    }
                }
            ]
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local temporal = require("assay.temporal")
        local c = temporal.client("{}")
        local result = c:namespaces()
        assert.eq(#result.namespaces, 2)
        assert.eq(result.namespaces[1].namespaceInfo.name, "default")
        assert.eq(result.namespaces[2].namespaceInfo.name, "production")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_temporal_namespace() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/namespaces/default"))
        .and(header("Content-Type", "application/json"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "namespaceInfo": {
                "name": "default",
                "state": "NAMESPACE_STATE_REGISTERED",
                "description": "Default namespace"
            },
            "config": {
                "workflowExecutionRetentionTtl": "259200s",
                "historyArchivalState": "ARCHIVAL_STATE_DISABLED"
            },
            "replicationConfig": {
                "activeClusterName": "active"
            },
            "isGlobalNamespace": false
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local temporal = require("assay.temporal")
        local c = temporal.client("{}")
        local ns = c:namespace("default")
        assert.eq(ns.namespaceInfo.name, "default")
        assert.eq(ns.namespaceInfo.state, "NAMESPACE_STATE_REGISTERED")
        assert.eq(ns.config.workflowExecutionRetentionTtl, "259200s")
        assert.eq(ns.isGlobalNamespace, false)
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_temporal_workflows() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/namespaces/default/workflows"))
        .and(header("Content-Type", "application/json"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "executions": [
                {
                    "execution": {
                        "workflowId": "order-processing-001",
                        "runId": "run-abc-123"
                    },
                    "type": {
                        "name": "OrderWorkflow"
                    },
                    "status": "WORKFLOW_EXECUTION_STATUS_RUNNING",
                    "startTime": "2025-01-15T10:00:00Z"
                },
                {
                    "execution": {
                        "workflowId": "order-processing-002",
                        "runId": "run-def-456"
                    },
                    "type": {
                        "name": "OrderWorkflow"
                    },
                    "status": "WORKFLOW_EXECUTION_STATUS_COMPLETED",
                    "startTime": "2025-01-15T09:00:00Z"
                }
            ]
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local temporal = require("assay.temporal")
        local c = temporal.client("{}")
        local result = c:workflows()
        assert.eq(#result.executions, 2)
        assert.eq(result.executions[1].execution.workflowId, "order-processing-001")
        assert.eq(result.executions[1].status, "WORKFLOW_EXECUTION_STATUS_RUNNING")
        assert.eq(result.executions[2].status, "WORKFLOW_EXECUTION_STATUS_COMPLETED")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_temporal_workflow() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(
            "/api/v1/namespaces/default/workflows/order-processing-001",
        ))
        .and(header("Content-Type", "application/json"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "workflowExecutionInfo": {
                "execution": {
                    "workflowId": "order-processing-001",
                    "runId": "run-abc-123"
                },
                "type": {
                    "name": "OrderWorkflow"
                },
                "status": "WORKFLOW_EXECUTION_STATUS_RUNNING",
                "startTime": "2025-01-15T10:00:00Z",
                "historyLength": "42",
                "taskQueue": "order-queue"
            }
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local temporal = require("assay.temporal")
        local c = temporal.client("{}")
        local wf = c:workflow("order-processing-001")
        assert.eq(wf.workflowExecutionInfo.execution.workflowId, "order-processing-001")
        assert.eq(wf.workflowExecutionInfo.execution.runId, "run-abc-123")
        assert.eq(wf.workflowExecutionInfo.status, "WORKFLOW_EXECUTION_STATUS_RUNNING")
        assert.eq(wf.workflowExecutionInfo.taskQueue, "order-queue")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_temporal_workflow_with_run_id() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(
            "/api/v1/namespaces/default/workflows/order-processing-001",
        ))
        .and(query_param("runId", "run-abc-123"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "workflowExecutionInfo": {
                "execution": {
                    "workflowId": "order-processing-001",
                    "runId": "run-abc-123"
                },
                "type": {
                    "name": "OrderWorkflow"
                },
                "status": "WORKFLOW_EXECUTION_STATUS_COMPLETED"
            }
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local temporal = require("assay.temporal")
        local c = temporal.client("{}")
        local wf = c:workflow("order-processing-001", "run-abc-123")
        assert.eq(wf.workflowExecutionInfo.execution.runId, "run-abc-123")
        assert.eq(wf.workflowExecutionInfo.status, "WORKFLOW_EXECUTION_STATUS_COMPLETED")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_temporal_workflow_history() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(
            "/api/v1/namespaces/default/workflows/order-processing-001/history",
        ))
        .and(header("Content-Type", "application/json"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "history": {
                "events": [
                    {
                        "eventId": "1",
                        "eventType": "EVENT_TYPE_WORKFLOW_EXECUTION_STARTED",
                        "eventTime": "2025-01-15T10:00:00Z",
                        "workflowExecutionStartedEventAttributes": {
                            "workflowType": { "name": "OrderWorkflow" },
                            "taskQueue": { "name": "order-queue" }
                        }
                    },
                    {
                        "eventId": "2",
                        "eventType": "EVENT_TYPE_WORKFLOW_TASK_SCHEDULED",
                        "eventTime": "2025-01-15T10:00:00.100Z"
                    },
                    {
                        "eventId": "3",
                        "eventType": "EVENT_TYPE_WORKFLOW_TASK_STARTED",
                        "eventTime": "2025-01-15T10:00:00.200Z"
                    }
                ]
            }
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local temporal = require("assay.temporal")
        local c = temporal.client("{}")
        local result = c:workflow_history("order-processing-001")
        assert.eq(#result.history.events, 3)
        assert.eq(result.history.events[1].eventType, "EVENT_TYPE_WORKFLOW_EXECUTION_STARTED")
        assert.eq(result.history.events[2].eventType, "EVENT_TYPE_WORKFLOW_TASK_SCHEDULED")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_temporal_signal_workflow() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path(
            "/api/v1/namespaces/default/workflows/order-processing-001/signal",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({})))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local temporal = require("assay.temporal")
        local c = temporal.client("{}")
        local result = c:signal_workflow("order-processing-001", "approve-order", {{ approved = true }})
        assert.not_nil(result)
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_temporal_terminate_workflow() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path(
            "/api/v1/namespaces/default/workflows/order-processing-001/terminate",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({})))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local temporal = require("assay.temporal")
        local c = temporal.client("{}")
        local result = c:terminate_workflow("order-processing-001", "manual termination by operator")
        assert.not_nil(result)
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_temporal_task_queue() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/namespaces/default/task-queues/order-queue"))
        .and(header("Content-Type", "application/json"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "pollers": [
                {
                    "lastAccessTime": "2025-01-15T10:05:00Z",
                    "identity": "worker-1@host-a",
                    "ratePerSecond": 100000.0,
                    "workerVersionCapabilities": {
                        "buildId": "v1.2.3"
                    }
                },
                {
                    "lastAccessTime": "2025-01-15T10:04:55Z",
                    "identity": "worker-2@host-b",
                    "ratePerSecond": 100000.0
                }
            ],
            "taskQueueStatus": {
                "backlogCountHint": "0",
                "readLevel": "5000",
                "ackLevel": "5000",
                "ratePerSecond": 0.0
            }
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local temporal = require("assay.temporal")
        local c = temporal.client("{}")
        local tq = c:task_queue("order-queue")
        assert.eq(#tq.pollers, 2)
        assert.eq(tq.pollers[1].identity, "worker-1@host-a")
        assert.eq(tq.pollers[2].identity, "worker-2@host-b")
        assert.eq(tq.taskQueueStatus.backlogCountHint, "0")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_temporal_schedules() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/namespaces/default/schedules"))
        .and(header("Content-Type", "application/json"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "schedules": [
                {
                    "scheduleId": "daily-report",
                    "info": {
                        "spec": {
                            "structuredCalendar": [{ "hour": [{ "start": 9 }] }]
                        },
                        "workflowType": { "name": "ReportWorkflow" },
                        "recentActions": [],
                        "futureActionTimes": ["2025-01-16T09:00:00Z"]
                    }
                },
                {
                    "scheduleId": "hourly-cleanup",
                    "info": {
                        "spec": {
                            "interval": [{ "interval": "3600s" }]
                        },
                        "workflowType": { "name": "CleanupWorkflow" },
                        "recentActions": [],
                        "futureActionTimes": ["2025-01-15T11:00:00Z"]
                    }
                }
            ]
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local temporal = require("assay.temporal")
        local c = temporal.client("{}")
        local result = c:schedules()
        assert.eq(#result.schedules, 2)
        assert.eq(result.schedules[1].scheduleId, "daily-report")
        assert.eq(result.schedules[2].scheduleId, "hourly-cleanup")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_temporal_is_workflow_running_true() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(
            "/api/v1/namespaces/default/workflows/order-processing-001",
        ))
        .and(header("Content-Type", "application/json"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "workflowExecutionInfo": {
                "execution": {
                    "workflowId": "order-processing-001",
                    "runId": "run-abc-123"
                },
                "type": { "name": "OrderWorkflow" },
                "status": "WORKFLOW_EXECUTION_STATUS_RUNNING"
            }
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local temporal = require("assay.temporal")
        local c = temporal.client("{}")
        assert.eq(c:is_workflow_running("order-processing-001"), true)
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_temporal_is_workflow_running_false() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(
            "/api/v1/namespaces/default/workflows/order-processing-002",
        ))
        .and(header("Content-Type", "application/json"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "workflowExecutionInfo": {
                "execution": {
                    "workflowId": "order-processing-002",
                    "runId": "run-def-456"
                },
                "type": { "name": "OrderWorkflow" },
                "status": "WORKFLOW_EXECUTION_STATUS_COMPLETED"
            }
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local temporal = require("assay.temporal")
        local c = temporal.client("{}")
        assert.eq(c:is_workflow_running("order-processing-002"), false)
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_temporal_api_key_auth() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/system-info"))
        .and(header("Authorization", "Bearer temporal-api-key-12345"))
        .and(header("Content-Type", "application/json"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "serverVersion": "1.24.0",
            "capabilities": {}
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local temporal = require("assay.temporal")
        local c = temporal.client("{}", {{ api_key = "temporal-api-key-12345" }})
        local info = c:system_info()
        assert.eq(info.serverVersion, "1.24.0")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_temporal_custom_namespace() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/namespaces/production/workflows"))
        .and(header("Content-Type", "application/json"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "executions": [
                {
                    "execution": {
                        "workflowId": "prod-wf-001",
                        "runId": "run-prod-001"
                    },
                    "type": { "name": "ProdWorkflow" },
                    "status": "WORKFLOW_EXECUTION_STATUS_RUNNING"
                }
            ]
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local temporal = require("assay.temporal")
        local c = temporal.client("{}", {{ namespace = "production" }})
        local result = c:workflows()
        assert.eq(#result.executions, 1)
        assert.eq(result.executions[1].execution.workflowId, "prod-wf-001")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_temporal_namespace_override() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/namespaces/staging/workflows"))
        .and(header("Content-Type", "application/json"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "executions": []
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local temporal = require("assay.temporal")
        local c = temporal.client("{}")
        local result = c:workflows({{ namespace = "staging" }})
        assert.eq(#result.executions, 0)
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_temporal_schedule() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/namespaces/default/schedules/daily-report"))
        .and(header("Content-Type", "application/json"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "schedule": {
                "spec": {
                    "structuredCalendar": [{ "hour": [{ "start": 9 }] }]
                },
                "action": {
                    "startWorkflow": {
                        "workflowType": { "name": "ReportWorkflow" },
                        "taskQueue": { "name": "report-queue" }
                    }
                },
                "state": {
                    "paused": false,
                    "notes": "Daily report at 9 AM"
                }
            },
            "info": {
                "recentActions": [],
                "futureActionTimes": ["2025-01-16T09:00:00Z"]
            }
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local temporal = require("assay.temporal")
        local c = temporal.client("{}")
        local sched = c:schedule("daily-report")
        assert.eq(sched.schedule.state.paused, false)
        assert.eq(sched.schedule.state.notes, "Daily report at 9 AM")
        assert.eq(sched.schedule.action.startWorkflow.taskQueue.name, "report-queue")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_temporal_search() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/namespaces/default/workflows"))
        .and(query_param("query", "WorkflowType='OrderWorkflow'"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "executions": [
                {
                    "execution": {
                        "workflowId": "order-processing-001",
                        "runId": "run-abc-123"
                    },
                    "type": { "name": "OrderWorkflow" },
                    "status": "WORKFLOW_EXECUTION_STATUS_RUNNING"
                }
            ]
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local temporal = require("assay.temporal")
        local c = temporal.client("{}")
        local result = c:search("WorkflowType='OrderWorkflow'")
        assert.eq(#result.executions, 1)
        assert.eq(result.executions[1].type.name, "OrderWorkflow")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_temporal_cancel_workflow() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path(
            "/api/v1/namespaces/default/workflows/order-processing-001/cancel",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({})))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local temporal = require("assay.temporal")
        local c = temporal.client("{}")
        local result = c:cancel_workflow("order-processing-001")
        assert.not_nil(result)
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}
