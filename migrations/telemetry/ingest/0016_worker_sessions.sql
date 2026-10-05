-- Logical stream restores may leave newer physical tables installed.
CREATE TEMP TABLE worker_sessions_upgrade(replay INTEGER NOT NULL);
INSERT INTO worker_sessions_upgrade SELECT count(*)<>3 FROM sqlite_master
 WHERE type='table' AND name IN ('codex_session_items','codex_session_turns','codex_session_clock');
-- Metadata only. All three tables follow native-session retention/full backup.
CREATE TABLE IF NOT EXISTS codex_session_items (
 session_id TEXT NOT NULL, item_id TEXT NOT NULL,
 item_type TEXT NOT NULL CHECK(item_type IN ('FileChange','ImageView','UserMessage')),
 status TEXT, changed_files INTEGER, completed_unix_ms INTEGER,
 PRIMARY KEY(session_id,item_id)
) STRICT;
CREATE TABLE IF NOT EXISTS codex_session_turns (
 session_id TEXT NOT NULL, turn_id TEXT NOT NULL, model_context_window INTEGER,
 PRIMARY KEY(session_id,turn_id)
) STRICT;
CREATE TABLE IF NOT EXISTS codex_session_clock (
 session_id TEXT PRIMARY KEY, last_record_unix_ms INTEGER, last_turn_id TEXT,
 last_turn_completed INTEGER NOT NULL CHECK(last_turn_completed IN (0,1))
) STRICT;
-- Neutral aliases reuse the retained command duration without duplicating rows.
CREATE VIEW IF NOT EXISTS codex_reported_exec_durations AS
 SELECT session_id,item_id,startup_duration_secs AS reported_duration_secs,
 startup_duration_nanos AS reported_duration_nanos FROM codex_exec_items;
-- Replay retained rollouts to derive new metadata. Existing rows dedupe.
UPDATE collect_offsets SET byte_offset=0 WHERE path_digest IN
 (SELECT path_digest FROM rollout_sources WHERE cli_version IS NOT NULL
  AND session_id NOT LIKE 'claude-code:%' AND session_id NOT LIKE 'muse:%'
  AND session_id NOT LIKE 'gemini:%' AND session_id NOT LIKE 'opencode:%')
 AND (SELECT replay FROM worker_sessions_upgrade)=1;
DROP TABLE worker_sessions_upgrade;
