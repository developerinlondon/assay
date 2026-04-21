-- 02_activity_timer_signal_fields.surql
-- Add missing fields to activity, timer, and signal tables.
-- These align with the PG/SQLite schema fields needed for full lifecycle support.

-- Sequence counter table for numeric IDs (used by activity, timer, signal).
DEFINE TABLE IF NOT EXISTS _seq SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS name ON _seq TYPE string;
DEFINE FIELD IF NOT EXISTS val  ON _seq TYPE int DEFAULT 0;
DEFINE INDEX IF NOT EXISTS seq_name ON _seq COLUMNS name UNIQUE;

-- Seed the three counters (idempotent: CREATE only if not already present).
CREATE _seq:activity CONTENT { name: 'activity', val: 0 };
CREATE _seq:timer    CONTENT { name: 'timer',    val: 0 };
CREATE _seq:signal   CONTENT { name: 'signal',   val: 0 };

-- Activity: add the fields missing from 00_init
DEFINE FIELD IF NOT EXISTS id_num                 ON activity TYPE int DEFAULT 0;
DEFINE FIELD IF NOT EXISTS name                   ON activity TYPE string DEFAULT '';
DEFINE FIELD IF NOT EXISTS input                  ON activity TYPE option<string>;
DEFINE FIELD IF NOT EXISTS max_attempts           ON activity TYPE int DEFAULT 3;
DEFINE FIELD IF NOT EXISTS initial_interval_secs  ON activity TYPE float DEFAULT 1.0;
DEFINE FIELD IF NOT EXISTS backoff_coefficient    ON activity TYPE float DEFAULT 2.0;
DEFINE FIELD IF NOT EXISTS start_to_close_secs    ON activity TYPE float DEFAULT 300.0;
DEFINE FIELD IF NOT EXISTS heartbeat_timeout_secs ON activity TYPE option<float>;
DEFINE FIELD IF NOT EXISTS completed_at           ON activity TYPE option<float>;
DEFINE FIELD IF NOT EXISTS last_heartbeat         ON activity TYPE option<float>;
-- id_num index for get_activity(id: i64)
DEFINE INDEX IF NOT EXISTS activity_id_num ON activity COLUMNS id_num UNIQUE;

-- Timer: add id_num
DEFINE FIELD IF NOT EXISTS id_num ON timer TYPE int DEFAULT 0;
DEFINE INDEX IF NOT EXISTS timer_id_num ON timer COLUMNS id_num UNIQUE;

-- Signal: add id_num and payload as string (payload field was option<object>, add payload_str as option<string>)
DEFINE FIELD IF NOT EXISTS id_num      ON signal TYPE int DEFAULT 0;
DEFINE FIELD IF NOT EXISTS payload_str ON signal TYPE option<string>;
DEFINE FIELD IF NOT EXISTS received_at ON signal TYPE float DEFAULT 0.0;
DEFINE INDEX IF NOT EXISTS signal_id_num ON signal COLUMNS id_num UNIQUE;
