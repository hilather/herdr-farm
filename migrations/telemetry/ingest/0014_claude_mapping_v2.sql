-- Claude mapping v2. Columns follow codex_usage session retention/full backup.
ALTER TABLE codex_usage ADD COLUMN cache_write_5m_tokens INTEGER CHECK (cache_write_5m_tokens BETWEEN 0 AND 9007199254740992);
ALTER TABLE codex_usage ADD COLUMN cache_write_1h_tokens INTEGER CHECK (cache_write_1h_tokens BETWEEN 0 AND 9007199254740992);
ALTER TABLE codex_usage ADD COLUMN reasoning_reported INTEGER NOT NULL DEFAULT 0 CHECK (reasoning_reported IN (0,1));
-- Disposable v1 usage must be re-derived before comparing payload digests.
-- Preserve sources/bindings and retained raw evidence; re-read their original files.
DELETE FROM codex_usage_times WHERE session_id LIKE 'claude-code:%';
DELETE FROM codex_quarantine WHERE session_id LIKE 'claude-code:%';
DELETE FROM codex_usage WHERE session_id LIKE 'claude-code:%';
DELETE FROM claude_messages;
UPDATE collect_offsets SET byte_offset=0,records=0,model=NULL,effort=NULL
WHERE path_digest IN (SELECT path_digest FROM rollout_sources WHERE session_id LIKE 'claude-code:%');
