-- Retain the approved wall budget independently of later configuration edits.
-- Historical workers remain recoverable as process_exit without inferred budgets.
ALTER TABLE attempt_inputs ADD COLUMN max_wall_seconds INTEGER
    CHECK(max_wall_seconds BETWEEN 1 AND 604800);
DROP TRIGGER attempt_inputs_no_update;
CREATE TRIGGER attempt_inputs_no_update BEFORE UPDATE ON attempt_inputs
WHEN NEW.attempt_id IS NOT OLD.attempt_id OR NEW.operation_id IS NOT OLD.operation_id
    OR NEW.payload IS NOT OLD.payload OR NEW.payload_hash IS NOT OLD.payload_hash
    OR OLD.max_wall_seconds IS NOT NULL OR NEW.max_wall_seconds IS NULL
BEGIN SELECT RAISE(ABORT,'attempt inputs are immutable'); END;
UPDATE store_meta SET schema_version=73;
PRAGMA user_version = 73;
