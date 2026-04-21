-- 03_schedules_workers_snapshots.surql
-- Add missing fields to schedule, worker, and snapshot tables for full
-- WorkflowStore parity with PG/SQLite.  All DEFINE FIELD statements use
-- IF NOT EXISTS so this migration is safe to replay.

-- Schedule: add fields present in PG schema but missing from 00_init
DEFINE FIELD IF NOT EXISTS workflow_type   ON schedule TYPE string DEFAULT '';
DEFINE FIELD IF NOT EXISTS input           ON schedule TYPE option<string>;
DEFINE FIELD IF NOT EXISTS task_queue      ON schedule TYPE string DEFAULT 'main';
DEFINE FIELD IF NOT EXISTS overlap_policy  ON schedule TYPE string DEFAULT 'skip';
DEFINE FIELD IF NOT EXISTS last_workflow_id ON schedule TYPE option<string>;
DEFINE FIELD IF NOT EXISTS created_at      ON schedule TYPE float DEFAULT 0.0;

-- Worker: replace/add fields to match PG workflow_workers schema.
-- The existing 00_init worker table only has id, namespace, task_queues,
-- heartbeat_at.  We add the full field set here.
-- Redefine task_queues WITH a DEFAULT so SCHEMAFULL inserts don't silently fail.
-- OVERWRITE is used (SurrealDB v3) to replace the existing field definition from
-- 00_init which lacks a DEFAULT.
DEFINE FIELD OVERWRITE task_queues ON worker TYPE array<string> DEFAULT [];
DEFINE FIELD IF NOT EXISTS identity                    ON worker TYPE string DEFAULT '';
DEFINE FIELD IF NOT EXISTS task_queue                  ON worker TYPE string DEFAULT 'main';
DEFINE FIELD IF NOT EXISTS workflows                   ON worker TYPE option<string>;
DEFINE FIELD IF NOT EXISTS activities                  ON worker TYPE option<string>;
DEFINE FIELD IF NOT EXISTS max_concurrent_workflows    ON worker TYPE int DEFAULT 10;
DEFINE FIELD IF NOT EXISTS max_concurrent_activities   ON worker TYPE int DEFAULT 10;
DEFINE FIELD IF NOT EXISTS active_tasks                ON worker TYPE int DEFAULT 0;
DEFINE FIELD IF NOT EXISTS last_heartbeat              ON worker TYPE float DEFAULT 0.0;
DEFINE FIELD IF NOT EXISTS registered_at               ON worker TYPE float DEFAULT 0.0;
-- heartbeat_at existed in 00_init without a DEFAULT; OVERWRITE to add one.
DEFINE FIELD OVERWRITE heartbeat_at ON worker TYPE float DEFAULT 0.0;

-- Snapshot: add created_at (required for WorkflowSnapshot struct)
DEFINE FIELD IF NOT EXISTS created_at ON snapshot TYPE float DEFAULT 0.0;
-- Unique index so create_snapshot is idempotent (SELECT-then-CREATE)
DEFINE INDEX IF NOT EXISTS snapshot_wf_seq_unique ON snapshot COLUMNS workflow_id, event_seq UNIQUE;
