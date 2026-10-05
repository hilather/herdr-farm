-- Informational SQLite wall-clock times; sequence remains the event order.
-- Historical events deliberately receive no inferred timestamp.
CREATE TABLE event_times (
    sequence INTEGER PRIMARY KEY REFERENCES events(sequence),
    recorded_unix_ms INTEGER NOT NULL
) STRICT;
CREATE TRIGGER events_record_time AFTER INSERT ON events
BEGIN
    INSERT INTO event_times(sequence,recorded_unix_ms)
    VALUES(NEW.sequence,CAST(unixepoch('subsec')*1000 AS INTEGER));
END;
CREATE TRIGGER event_times_no_update BEFORE UPDATE ON event_times
BEGIN SELECT RAISE(ABORT,'event times are immutable'); END;
CREATE TRIGGER event_times_no_delete BEFORE DELETE ON event_times
BEGIN SELECT RAISE(ABORT,'event times are retained'); END;
CREATE TABLE task_lineage (
    task_id TEXT PRIMARY KEY REFERENCES tasks(id),
    work_item TEXT NOT NULL,
    role TEXT NOT NULL CHECK(role IN ('build','fix','review','skeptic','recheck','merge','plan','other')),
    supersedes_task TEXT REFERENCES tasks(id),
    recorded_unix_ms INTEGER NOT NULL
) STRICT;
CREATE TRIGGER task_lineage_no_update BEFORE UPDATE ON task_lineage
BEGIN SELECT RAISE(ABORT,'task lineage is immutable'); END;
CREATE TRIGGER task_lineage_no_delete BEFORE DELETE ON task_lineage
BEGIN SELECT RAISE(ABORT,'task lineage is retained'); END;
UPDATE store_meta SET schema_version=72 WHERE singleton=1;
PRAGMA user_version=72;
