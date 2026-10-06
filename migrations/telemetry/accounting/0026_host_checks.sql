-- Coordinator checks are observational metadata, never acceptance evidence.
CREATE TABLE IF NOT EXISTS host_checks (
    run_id TEXT PRIMARY KEY,
    task TEXT NOT NULL,
    started_unix_ms INTEGER NOT NULL,
    finished_unix_ms INTEGER,
    payload TEXT NOT NULL CHECK(json_valid(payload))
) STRICT;
CREATE INDEX IF NOT EXISTS host_checks_task_time ON host_checks(task, started_unix_ms DESC);
