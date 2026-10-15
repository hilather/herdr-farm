-- One bounded summary per attempt, never an append-only error journal.
CREATE TABLE launch_job_errors (
    attempt TEXT PRIMARY KEY,
    job TEXT NOT NULL,
    count INTEGER NOT NULL,
    first_error_unix_ms INTEGER NOT NULL,
    last_error_unix_ms INTEGER NOT NULL,
    error TEXT NOT NULL
) STRICT;
