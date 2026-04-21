-- 01_workflow_fields.surql — add missing fields to workflow and event tables
-- These fields were omitted from 00_init.surql; added here as additive migration.

-- Workflow: additional fields mirroring the PG/SQLite schema
DEFINE FIELD IF NOT EXISTS run_id ON workflow TYPE string DEFAULT '';
DEFINE FIELD IF NOT EXISTS task_queue ON workflow TYPE string DEFAULT 'main';
DEFINE FIELD IF NOT EXISTS input ON workflow TYPE option<string>;
DEFINE FIELD IF NOT EXISTS result ON workflow TYPE option<string>;
DEFINE FIELD IF NOT EXISTS error ON workflow TYPE option<string>;
DEFINE FIELD IF NOT EXISTS claimed_by ON workflow TYPE option<string>;
DEFINE FIELD IF NOT EXISTS updated_at ON workflow TYPE float DEFAULT 0.0;

