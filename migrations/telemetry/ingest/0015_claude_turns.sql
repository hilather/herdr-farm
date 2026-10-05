-- Metadata only; follows normalized session retention and full backup.
CREATE TABLE IF NOT EXISTS claude_turn_lines (
    session_id TEXT NOT NULL,
    path_digest TEXT NOT NULL,
    byte_offset INTEGER NOT NULL CHECK(byte_offset >= 0),
    occurred_unix_ms INTEGER,
    line_type TEXT NOT NULL,
    is_prompt INTEGER NOT NULL CHECK(is_prompt IN (0,1)),
    metadata TEXT NOT NULL CHECK(json_valid(metadata)),
    PRIMARY KEY(session_id,path_digest,byte_offset)
) STRICT;
UPDATE collect_offsets SET byte_offset=0 WHERE path_digest IN
    (SELECT path_digest FROM rollout_sources WHERE session_id LIKE 'claude-code:%');
