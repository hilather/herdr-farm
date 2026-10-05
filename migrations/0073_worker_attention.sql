-- Advisory observations, retained with canonical state and its backups.
CREATE TABLE worker_idle_stretches (
    attempt_id TEXT PRIMARY KEY REFERENCES attempts(id),
    generation INTEGER NOT NULL DEFAULT 0,
    idle_since_ms INTEGER,
    observed_ms INTEGER NOT NULL,
    notified INTEGER NOT NULL DEFAULT 0 CHECK(notified IN (0,1))
) STRICT;
UPDATE store_meta SET schema_version=73 WHERE singleton=1;
PRAGMA user_version=73;
