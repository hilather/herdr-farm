-- One fix attempt/submission/integration may repair multiple findings.
-- Preserve the append-only ledger and exact-candidate guards.
PRAGMA defer_foreign_keys = ON;
PRAGMA legacy_alter_table = ON;
CREATE TABLE repair_attempts_v70 (
    seq INTEGER PRIMARY KEY REFERENCES fix_log(seq),
    repair_seq INTEGER NOT NULL REFERENCES repair_opportunities(seq),
    attempt_id TEXT NOT NULL REFERENCES attempts(id),
    ordinal INTEGER NOT NULL CHECK (ordinal > 0),
    configuration_id TEXT REFERENCES agent_configurations(configuration_id),
    UNIQUE (repair_seq, ordinal),
    UNIQUE (repair_seq, attempt_id)
) STRICT;
INSERT INTO repair_attempts_v70 SELECT * FROM repair_attempts;
DROP TABLE repair_attempts;
ALTER TABLE repair_attempts_v70 RENAME TO repair_attempts;
CREATE TABLE fix_proposals_v70 (
    seq INTEGER PRIMARY KEY REFERENCES fix_log(seq),
    repair_seq INTEGER NOT NULL REFERENCES repair_opportunities(seq),
    submission_id TEXT NOT NULL REFERENCES result_submissions(submission_id),
    attempt_id TEXT NOT NULL REFERENCES attempts(id),
    candidate_oid TEXT NOT NULL CHECK (length(candidate_oid) IN (40, 64)),
    UNIQUE (repair_seq, submission_id)
) STRICT;
INSERT INTO fix_proposals_v70 SELECT * FROM fix_proposals;
DROP TABLE fix_proposals;
ALTER TABLE fix_proposals_v70 RENAME TO fix_proposals;
CREATE TABLE fix_integrations_v70 (
    seq INTEGER PRIMARY KEY REFERENCES fix_log(seq),
    proposal_seq INTEGER NOT NULL UNIQUE REFERENCES fix_proposals(seq),
    verification_seq INTEGER NOT NULL REFERENCES fix_verifications(seq),
    integrated_id TEXT NOT NULL REFERENCES integrated_commits(integrated_id),
    commit_oid TEXT NOT NULL CHECK (length(commit_oid) IN (40, 64)),
    integrated_unix_ms INTEGER NOT NULL
) STRICT;
INSERT INTO fix_integrations_v70 SELECT * FROM fix_integrations;
DROP TABLE fix_integrations;
ALTER TABLE fix_integrations_v70 RENAME TO fix_integrations;
PRAGMA legacy_alter_table = OFF;
CREATE INDEX repair_attempts_by_repair ON repair_attempts(repair_seq, seq);
CREATE INDEX fix_proposals_by_repair ON fix_proposals(repair_seq, seq);
CREATE TRIGGER repair_attempts_no_update BEFORE UPDATE ON repair_attempts BEGIN SELECT RAISE(ABORT, 'fix history is append-only'); END;
CREATE TRIGGER repair_attempts_no_delete BEFORE DELETE ON repair_attempts BEGIN SELECT RAISE(ABORT, 'fix history is append-only'); END;
CREATE TRIGGER fix_proposals_no_update BEFORE UPDATE ON fix_proposals BEGIN SELECT RAISE(ABORT, 'fix history is append-only'); END;
CREATE TRIGGER fix_proposals_no_delete BEFORE DELETE ON fix_proposals BEGIN SELECT RAISE(ABORT, 'fix history is append-only'); END;
CREATE TRIGGER fix_integrations_no_update BEFORE UPDATE ON fix_integrations BEGIN SELECT RAISE(ABORT, 'fix history is append-only'); END;
CREATE TRIGGER fix_integrations_no_delete BEFORE DELETE ON fix_integrations BEGIN SELECT RAISE(ABORT, 'fix history is append-only'); END;
CREATE TRIGGER repair_attempts_kind BEFORE INSERT ON repair_attempts
WHEN NOT EXISTS (SELECT 1 FROM fix_log l WHERE l.seq = NEW.seq AND l.kind = 'attempt_bound')
BEGIN SELECT RAISE(ABORT, 'fix attribution needs its history row'); END;
CREATE TRIGGER fix_proposals_kind BEFORE INSERT ON fix_proposals
WHEN NOT EXISTS (SELECT 1 FROM fix_log l WHERE l.seq = NEW.seq AND l.kind = 'proposed')
BEGIN SELECT RAISE(ABORT, 'fix attribution needs its history row'); END;
CREATE TRIGGER fix_integrations_kind BEFORE INSERT ON fix_integrations
WHEN NOT EXISTS (SELECT 1 FROM fix_log l WHERE l.seq = NEW.seq AND l.kind = 'integrated')
BEGIN SELECT RAISE(ABORT, 'fix attribution needs its history row'); END;
CREATE TRIGGER repair_attempts_before_outcome BEFORE INSERT ON repair_attempts
WHEN EXISTS (SELECT 1 FROM result_submissions s WHERE s.attempt_id = NEW.attempt_id)
    OR EXISTS (SELECT 1 FROM repair_closures c WHERE c.repair_seq = NEW.repair_seq)
BEGIN SELECT RAISE(ABORT, 'a repair attempt is bound to an open repair opportunity before it has any result'); END;
CREATE TRIGGER fix_proposals_exact BEFORE INSERT ON fix_proposals
WHEN NOT EXISTS (SELECT 1 FROM result_submissions s JOIN repair_attempts a ON a.attempt_id = s.attempt_id
        WHERE s.submission_id = NEW.submission_id AND s.attempt_id = NEW.attempt_id AND s.candidate_oid = NEW.candidate_oid AND a.repair_seq = NEW.repair_seq)
BEGIN SELECT RAISE(ABORT, 'a fix proposal is a result submission of an attempt bound to its repair, at its exact candidate'); END;
CREATE TRIGGER fix_integrations_exact BEFORE INSERT ON fix_integrations
WHEN NOT EXISTS (SELECT 1 FROM fix_verifications fv JOIN fix_proposals p ON p.seq = fv.proposal_seq
        JOIN integrated_commits ic ON ic.integrated_id = NEW.integrated_id
        JOIN integration_operations o ON o.operation_id = ic.operation_id
        JOIN verified_results v ON v.result_id = o.verified_result_id
        JOIN integration_candidates c ON c.candidate_id = ic.candidate_id
        WHERE fv.seq = NEW.verification_seq AND fv.proposal_seq = NEW.proposal_seq AND v.submission_id = p.submission_id
            AND v.commit_oid = p.candidate_oid AND c.parent_verified = p.candidate_oid AND ic.commit_oid = NEW.commit_oid
            AND ic.created_unix_ms = NEW.integrated_unix_ms)
BEGIN SELECT RAISE(ABORT, 'a fix is integrated only by an integration of its exact verified candidate'); END;
CREATE INDEX repair_attempts_by_attempt ON repair_attempts(attempt_id,repair_seq);
CREATE INDEX fix_proposals_by_submission ON fix_proposals(submission_id,repair_seq);
CREATE TABLE fix_launches (
    task_id TEXT PRIMARY KEY REFERENCES tasks(id),
    selection_json TEXT NOT NULL CHECK (json_valid(selection_json)),
    profile TEXT NOT NULL
) STRICT;
CREATE TRIGGER fix_launches_no_update BEFORE UPDATE ON fix_launches BEGIN SELECT RAISE(ABORT, 'fix launches are append-only'); END;
CREATE TRIGGER fix_launches_no_delete BEFORE DELETE ON fix_launches BEGIN SELECT RAISE(ABORT, 'fix launches are append-only'); END;
CREATE TABLE fix_launch_findings (
    task_id TEXT NOT NULL REFERENCES fix_launches(task_id),
    repair_seq INTEGER NOT NULL UNIQUE REFERENCES repair_opportunities(seq),
    PRIMARY KEY (task_id, repair_seq)
) STRICT;
CREATE TRIGGER fix_launch_findings_no_update BEFORE UPDATE ON fix_launch_findings BEGIN SELECT RAISE(ABORT, 'fix launch bindings are append-only'); END;
CREATE TRIGGER fix_launch_findings_no_delete BEFORE DELETE ON fix_launch_findings BEGIN SELECT RAISE(ABORT, 'fix launch bindings are append-only'); END;
-- SQLite retains deferred DROP-table violation counters even when the same
-- parent identities are restored. The migration caller checks foreign_key_check
-- before commit; clear only those counters after rebuilding the parent tables.
PRAGMA defer_foreign_keys = OFF;
UPDATE store_meta SET schema_version = 70;
PRAGMA user_version = 70;
