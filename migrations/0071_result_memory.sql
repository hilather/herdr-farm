-- Durable Remember intake; proposal bytes and review history use the existing stores.
CREATE TABLE result_memory_candidates (
    proposal_id TEXT PRIMARY KEY NOT NULL,
    attempt_id TEXT NOT NULL REFERENCES attempts(id),
    task_id TEXT NOT NULL REFERENCES tasks(id),
    submission_id TEXT NOT NULL REFERENCES result_submissions(submission_id),
    snapshot_id TEXT NOT NULL REFERENCES memory_snapshots(id),
    content_digest TEXT NOT NULL CHECK(length(content_digest)=64),
    remember TEXT NOT NULL CHECK(length(remember)>0),
    captured INTEGER NOT NULL DEFAULT 0 CHECK(captured IN (0,1)),
    UNIQUE(attempt_id,content_digest)
) STRICT;
CREATE INDEX result_memory_candidates_pending ON result_memory_candidates(proposal_id) WHERE captured=0;
CREATE TABLE result_memory_decisions (
    proposal_id TEXT PRIMARY KEY NOT NULL REFERENCES memory_proposals(id),
    decision_id TEXT NOT NULL UNIQUE REFERENCES review_decisions(id),
    principal TEXT NOT NULL CHECK(principal='coordinator'),
    delegation TEXT NOT NULL,
    notification_id TEXT NOT NULL UNIQUE REFERENCES operations(id)
) STRICT;
CREATE TRIGGER result_memory_candidates_evidence_no_update
BEFORE UPDATE OF proposal_id,attempt_id,task_id,submission_id,snapshot_id,content_digest,remember ON result_memory_candidates
BEGIN SELECT RAISE(ABORT,'Remember provenance is immutable'); END;
CREATE TRIGGER result_memory_candidates_no_delete BEFORE DELETE ON result_memory_candidates
BEGIN SELECT RAISE(ABORT,'Remember evidence is retained'); END;
CREATE TRIGGER result_memory_decisions_no_update BEFORE UPDATE ON result_memory_decisions
BEGIN SELECT RAISE(ABORT,'delegated memory decision is immutable'); END;
CREATE TRIGGER result_memory_decisions_no_delete BEFORE DELETE ON result_memory_decisions
BEGIN SELECT RAISE(ABORT,'delegated memory decision is retained'); END;
UPDATE store_meta SET schema_version=71 WHERE singleton=1;
PRAGMA user_version=71;
