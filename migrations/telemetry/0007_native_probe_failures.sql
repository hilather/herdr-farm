-- Redacted, bounded observations only; never launch authority.
CREATE TABLE native_probe_failures (
    sequence INTEGER PRIMARY KEY,
    profile TEXT NOT NULL,
    payload TEXT NOT NULL CHECK (length(CAST(payload AS BLOB)) <= 4096)
) STRICT;
CREATE INDEX native_probe_failures_profile ON native_probe_failures(profile, sequence DESC);
PRAGMA user_version = 7;
