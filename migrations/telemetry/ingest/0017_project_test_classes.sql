-- Logical stream restores can leave the physical class table installed.
CREATE TEMP TABLE project_test_classes_upgrade(replay INTEGER NOT NULL);
INSERT INTO project_test_classes_upgrade SELECT NOT EXISTS
 (SELECT 1 FROM sqlite_master WHERE type='table' AND name='codex_exec_classes');
-- Metadata-only classification; native-session retention and full backup.
CREATE TABLE IF NOT EXISTS codex_exec_classes (
 session_id TEXT NOT NULL, item_id TEXT NOT NULL,
 class TEXT CHECK(class IS NULL OR class='project_test'),
 classification_available INTEGER NOT NULL CHECK(classification_available IN (0,1)),
 PRIMARY KEY(session_id,item_id)
) STRICT;
-- Replay retained sources; command bytes are transient and never persisted.
UPDATE collect_offsets SET byte_offset=0 WHERE path_digest IN
 (SELECT path_digest FROM rollout_sources WHERE cli_version IS NOT NULL
  AND session_id NOT LIKE 'claude-code:%' AND session_id NOT LIKE 'muse:%'
  AND session_id NOT LIKE 'gemini:%' AND session_id NOT LIKE 'opencode:%')
 AND (SELECT replay FROM project_test_classes_upgrade)=1;
DROP TABLE project_test_classes_upgrade;
