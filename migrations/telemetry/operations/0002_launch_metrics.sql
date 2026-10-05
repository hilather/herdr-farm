-- MET-LAUNCH-1: identifiers, bounded classes and counts only.
CREATE TABLE IF NOT EXISTS operation_cli_targets (
 invocation_id TEXT PRIMARY KEY REFERENCES cli_invocations(invocation_id) ON DELETE CASCADE,
 task_id TEXT,
 attempt_id TEXT,
 force INTEGER NOT NULL CHECK(force IN (0,1))
) STRICT;
CREATE TABLE IF NOT EXISTS operation_ticker_errors (
 session TEXT NOT NULL,
 pass INTEGER NOT NULL CHECK(pass>=0),
 sampled_unix_ms INTEGER NOT NULL CHECK(sampled_unix_ms>=0),
 lock_contention INTEGER NOT NULL CHECK(lock_contention>=0),
 expired_inventory INTEGER NOT NULL CHECK(expired_inventory>=0),
 ambiguous_outcome INTEGER NOT NULL CHECK(ambiguous_outcome>=0),
 permanent_failure INTEGER NOT NULL CHECK(permanent_failure>=0),
 other INTEGER NOT NULL CHECK(other>=0),
 PRIMARY KEY(session,pass)
) STRICT;
CREATE TABLE IF NOT EXISTS operation_submission_diff (
 submission_id TEXT PRIMARY KEY,
 added INTEGER CHECK(added>=0),
 removed INTEGER CHECK(removed>=0),
 binary_files INTEGER CHECK(binary_files>=0),
 reason TEXT CHECK(reason IS NULL OR reason='diff_unavailable')
) STRICT;
-- Sidecar connections need not enable SQLite foreign keys. Always keep the
-- metadata lifetime coupled to CLI retention, including restore tombstones.
CREATE TRIGGER IF NOT EXISTS operation_cli_targets_prune AFTER DELETE ON cli_invocations
BEGIN DELETE FROM operation_cli_targets WHERE invocation_id=OLD.invocation_id; END;
