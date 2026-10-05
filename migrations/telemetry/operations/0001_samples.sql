-- Metadata only. Launch samples are advisory; storage history is bounded.
CREATE TABLE IF NOT EXISTS operation_launch_load (
    attempt_id TEXT PRIMARY KEY,
    sampled_unix_ms INTEGER NOT NULL CHECK(sampled_unix_ms >= 0),
    load_1m TEXT,
    load_5m TEXT,
    load_15m TEXT,
    reason TEXT CHECK(reason IS NULL OR reason='unreadable_or_invalid')
) STRICT;
CREATE TABLE IF NOT EXISTS operation_storage_samples (
    sampled_unix_ms INTEGER PRIMARY KEY CHECK(sampled_unix_ms >= 0),
    state_bytes INTEGER CHECK(state_bytes >= 0),
    telemetry_bytes INTEGER CHECK(telemetry_bytes >= 0),
    worktrees_bytes INTEGER CHECK(worktrees_bytes >= 0),
    worker_output_bytes INTEGER CHECK(worker_output_bytes >= 0)
) STRICT;
