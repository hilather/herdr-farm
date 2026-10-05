-- Product CLI metadata only; independent of canonical command effects.
CREATE TABLE IF NOT EXISTS cli_invocations (
    invocation_id TEXT PRIMARY KEY,
    command_path TEXT NOT NULL,
    outcome TEXT NOT NULL CHECK(outcome IN ('ok','error','help','version','usage_error')),
    exit_code INTEGER NOT NULL,
    duration_ms INTEGER NOT NULL CHECK(duration_ms >= 0),
    project_slug TEXT,
    caller TEXT NOT NULL CHECK(caller IN ('operator','worker','coordinator','plugin','ticker')),
    trust TEXT NOT NULL CHECK(trust IN ('local','worker_reported')),
    recorded_unix_ms INTEGER NOT NULL
) STRICT;
CREATE INDEX IF NOT EXISTS cli_invocations_time ON cli_invocations(recorded_unix_ms, invocation_id);
