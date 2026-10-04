-- USAGE-NEWER-1: retain explicit format drift without inventing counters.
-- Same logical table and retention/backup class; copy every historical row.
CREATE TABLE codex_usage_0005 (
    session_id TEXT NOT NULL,
    ordinal INTEGER NOT NULL CHECK (ordinal > 0),
    path_digest TEXT NOT NULL,
    response_id TEXT,
    turn_id TEXT,
    model TEXT,
    effort TEXT,
    payload_digest TEXT NOT NULL,
    input_tokens INTEGER,
    cached_input_tokens INTEGER,
    cache_write_input_tokens INTEGER,
    output_tokens INTEGER,
    reasoning_output_tokens INTEGER,
    total_tokens INTEGER,
    accepted INTEGER NOT NULL CHECK (accepted IN (0, 1)),
    reason TEXT CHECK (reason IN ('invariant_violation', 'cli_version_uncertified', 'schema_unrecognized')),
    observed_unix_ms INTEGER NOT NULL,
    PRIMARY KEY (session_id, ordinal),
    CHECK ((accepted = 1) = (reason IS NULL)),
    CHECK (accepted = 1 OR total_tokens IS NULL)
) STRICT;
INSERT INTO codex_usage_0005 SELECT * FROM codex_usage;
DROP TABLE codex_usage;
ALTER TABLE codex_usage_0005 RENAME TO codex_usage;
CREATE INDEX codex_usage_by_path ON codex_usage(path_digest, accepted, reason);
CREATE INDEX codex_usage_by_turn ON codex_usage(session_id, turn_id, model);
CREATE INDEX codex_usage_by_response ON codex_usage(session_id,response_id,unhex(substr(payload_digest,8)),accepted,ordinal);
PRAGMA user_version = 5;
