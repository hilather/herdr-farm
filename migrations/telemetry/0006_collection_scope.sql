-- Content-free negative discovery cache for shared execution homes.
CREATE TABLE rollout_scope_skips (
    path_digest TEXT PRIMARY KEY,
    size INTEGER NOT NULL,
    mtime INTEGER NOT NULL,
    mtime_nsec INTEGER NOT NULL
) STRICT;
PRAGMA user_version = 6;
