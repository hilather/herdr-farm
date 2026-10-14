-- Content-free incremental timing, retained with native sessions (90 days).
CREATE TABLE IF NOT EXISTS model_time_state (
 path_digest TEXT PRIMARY KEY, session_id TEXT NOT NULL, byte_offset INTEGER NOT NULL,
 first_ms INTEGER, last_ms INTEGER, trigger_ms INTEGER, pending_ms INTEGER,
 pending_ordinal INTEGER, model TEXT
) STRICT;
CREATE TABLE IF NOT EXISTS model_time_samples (
 path_digest TEXT NOT NULL, session_id TEXT NOT NULL, byte_offset INTEGER NOT NULL,
 ordinal INTEGER, aggregate INTEGER NOT NULL DEFAULT 0 CHECK(aggregate IN (0,1)), model TEXT, ended_ms INTEGER NOT NULL,
 duration_ms INTEGER NOT NULL CHECK(duration_ms > 0),
 output_tokens INTEGER NOT NULL CHECK(output_tokens >= 0),
 PRIMARY KEY(path_digest,byte_offset)
) STRICT;
CREATE INDEX IF NOT EXISTS model_time_samples_session ON model_time_samples(session_id);
CREATE UNIQUE INDEX IF NOT EXISTS model_time_samples_response ON model_time_samples(session_id,ordinal) WHERE ordinal IS NOT NULL;
