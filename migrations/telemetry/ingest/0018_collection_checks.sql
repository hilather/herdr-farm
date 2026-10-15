-- One bounded, content-free receipt per project, including empty collections.
CREATE TABLE IF NOT EXISTS collection_check (
    singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
    checked_unix_ms INTEGER NOT NULL,
    progress_unix_ms INTEGER,
    due_since_unix_ms INTEGER,
    pending INTEGER NOT NULL CHECK (pending IN (0, 1))
) STRICT;
