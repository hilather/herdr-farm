-- Runway reads previously scanned all observations per historical window and
-- all session observations per attribution interval. Indexes retain no extra
-- evidence: retention deletes them with their rows; sidecar backups include them.
CREATE INDEX IF NOT EXISTS quota_read_window ON quota_window_observations(window_id,trust,observed_unix_ms,session_id,ordinal);
CREATE INDEX IF NOT EXISTS quota_read_previous ON quota_window_observations(session_id,limit_id,window_kind,trust,observed_unix_ms);
CREATE INDEX IF NOT EXISTS quota_read_usage ON usage_entries(session_id,position,basis);
