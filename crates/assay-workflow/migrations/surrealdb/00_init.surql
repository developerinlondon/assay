-- 00_init.surql — initial schema for assay-workflow SurrealDB backend
-- Applied once; tracked via _assay_migrations.

-- Namespaces
DEFINE TABLE IF NOT EXISTS namespace SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS name ON namespace TYPE string;
DEFINE FIELD IF NOT EXISTS created_at ON namespace TYPE float;
DEFINE INDEX IF NOT EXISTS namespace_name ON namespace COLUMNS name UNIQUE;

-- Workflows
DEFINE TABLE IF NOT EXISTS workflow SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS id ON workflow TYPE string;
DEFINE FIELD IF NOT EXISTS namespace ON workflow TYPE string;
DEFINE FIELD IF NOT EXISTS status ON workflow TYPE string;
DEFINE FIELD IF NOT EXISTS workflow_type ON workflow TYPE string;
DEFINE FIELD IF NOT EXISTS search_attributes ON workflow TYPE option<object> FLEXIBLE;
DEFINE FIELD IF NOT EXISTS next_dispatch_at ON workflow TYPE option<float>;
DEFINE FIELD IF NOT EXISTS needs_dispatch ON workflow TYPE bool DEFAULT false;
DEFINE FIELD IF NOT EXISTS dispatch_claimed_by ON workflow TYPE option<string>;
DEFINE FIELD IF NOT EXISTS dispatch_last_heartbeat ON workflow TYPE option<float>;
DEFINE FIELD IF NOT EXISTS version ON workflow TYPE int DEFAULT 0;
DEFINE FIELD IF NOT EXISTS completed_at ON workflow TYPE option<float>;
DEFINE FIELD IF NOT EXISTS archived_at ON workflow TYPE option<float>;
DEFINE FIELD IF NOT EXISTS archive_uri ON workflow TYPE option<string>;
DEFINE FIELD IF NOT EXISTS parent_id ON workflow TYPE option<string>;
DEFINE FIELD IF NOT EXISTS created_at ON workflow TYPE float;
DEFINE INDEX IF NOT EXISTS workflow_dispatch ON workflow COLUMNS namespace, status, next_dispatch_at;
DEFINE INDEX IF NOT EXISTS workflow_parent ON workflow COLUMNS parent_id;

-- Events
DEFINE TABLE IF NOT EXISTS event SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS workflow_id ON event TYPE string;
DEFINE FIELD IF NOT EXISTS seq ON event TYPE int;
DEFINE FIELD IF NOT EXISTS event_type ON event TYPE string;
DEFINE FIELD IF NOT EXISTS payload ON event TYPE option<string>;
DEFINE FIELD IF NOT EXISTS created_at ON event TYPE float;
DEFINE INDEX IF NOT EXISTS event_workflow ON event COLUMNS workflow_id, seq;

-- Activities
DEFINE TABLE IF NOT EXISTS activity SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS workflow_id ON activity TYPE string;
DEFINE FIELD IF NOT EXISTS seq ON activity TYPE int;
DEFINE FIELD IF NOT EXISTS task_queue ON activity TYPE string;
DEFINE FIELD IF NOT EXISTS status ON activity TYPE string;
DEFINE FIELD IF NOT EXISTS attempt ON activity TYPE int DEFAULT 1;
DEFINE FIELD IF NOT EXISTS scheduled_at ON activity TYPE float;
DEFINE FIELD IF NOT EXISTS claimed_by ON activity TYPE option<string>;
DEFINE FIELD IF NOT EXISTS started_at ON activity TYPE option<float>;
DEFINE FIELD IF NOT EXISTS heartbeat_at ON activity TYPE option<float>;
DEFINE FIELD IF NOT EXISTS heartbeat_details ON activity TYPE option<string>;
DEFINE FIELD IF NOT EXISTS result ON activity TYPE option<string>;
DEFINE FIELD IF NOT EXISTS error ON activity TYPE option<string>;
DEFINE INDEX IF NOT EXISTS activity_claim ON activity COLUMNS task_queue, status, scheduled_at;
DEFINE INDEX IF NOT EXISTS activity_workflow_seq ON activity COLUMNS workflow_id, seq UNIQUE;

-- Timers
DEFINE TABLE IF NOT EXISTS timer SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS workflow_id ON timer TYPE string;
DEFINE FIELD IF NOT EXISTS seq ON timer TYPE int;
DEFINE FIELD IF NOT EXISTS fire_at ON timer TYPE float;
DEFINE FIELD IF NOT EXISTS fired ON timer TYPE bool DEFAULT false;
DEFINE INDEX IF NOT EXISTS timer_fire_at ON timer COLUMNS fire_at, fired;
DEFINE INDEX IF NOT EXISTS timer_workflow_seq ON timer COLUMNS workflow_id, seq UNIQUE;

-- Signals
DEFINE TABLE IF NOT EXISTS signal SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS workflow_id ON signal TYPE string;
DEFINE FIELD IF NOT EXISTS name ON signal TYPE string;
DEFINE FIELD IF NOT EXISTS payload ON signal TYPE option<object>;
DEFINE FIELD IF NOT EXISTS consumed ON signal TYPE bool DEFAULT false;
DEFINE FIELD IF NOT EXISTS created_at ON signal TYPE float;
DEFINE INDEX IF NOT EXISTS signal_consume ON signal COLUMNS workflow_id, name, consumed;

-- Schedules
DEFINE TABLE IF NOT EXISTS schedule SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS namespace ON schedule TYPE string;
DEFINE FIELD IF NOT EXISTS name ON schedule TYPE string;
DEFINE FIELD IF NOT EXISTS cron_expr ON schedule TYPE string;
DEFINE FIELD IF NOT EXISTS timezone ON schedule TYPE string;
DEFINE FIELD IF NOT EXISTS paused ON schedule TYPE bool DEFAULT false;
DEFINE FIELD IF NOT EXISTS last_run_at ON schedule TYPE option<float>;
DEFINE FIELD IF NOT EXISTS next_run_at ON schedule TYPE option<float>;
DEFINE INDEX IF NOT EXISTS schedule_ns_name ON schedule COLUMNS namespace, name UNIQUE;
DEFINE INDEX IF NOT EXISTS schedule_next_run ON schedule COLUMNS next_run_at;

-- Snapshots
DEFINE TABLE IF NOT EXISTS snapshot SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS workflow_id ON snapshot TYPE string;
DEFINE FIELD IF NOT EXISTS event_seq ON snapshot TYPE int;
DEFINE FIELD IF NOT EXISTS state_json ON snapshot TYPE string;
DEFINE INDEX IF NOT EXISTS snapshot_workflow_seq ON snapshot COLUMNS workflow_id, event_seq;

-- Workers
DEFINE TABLE IF NOT EXISTS worker SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS id ON worker TYPE string;
DEFINE FIELD IF NOT EXISTS namespace ON worker TYPE string;
DEFINE FIELD IF NOT EXISTS task_queues ON worker TYPE array<string>;
DEFINE FIELD IF NOT EXISTS heartbeat_at ON worker TYPE float;
DEFINE INDEX IF NOT EXISTS worker_ns ON worker COLUMNS namespace;

-- API keys
DEFINE TABLE IF NOT EXISTS api_key SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS key_hash ON api_key TYPE string;
DEFINE FIELD IF NOT EXISTS prefix ON api_key TYPE string;
DEFINE FIELD IF NOT EXISTS label ON api_key TYPE option<string>;
DEFINE FIELD IF NOT EXISTS created_at ON api_key TYPE float;
DEFINE INDEX IF NOT EXISTS api_key_prefix ON api_key COLUMNS prefix UNIQUE;
DEFINE INDEX IF NOT EXISTS api_key_hash ON api_key COLUMNS key_hash UNIQUE;

-- Scheduler lock (for leader election)
DEFINE TABLE IF NOT EXISTS scheduler_lock SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS holder ON scheduler_lock TYPE string;
DEFINE FIELD IF NOT EXISTS expires_at ON scheduler_lock TYPE float;

-- Seed: default namespace (mirrors PG/SQLite migration behaviour)
CREATE namespace:main CONTENT {
    name: 'main',
    created_at: 0.0
};
