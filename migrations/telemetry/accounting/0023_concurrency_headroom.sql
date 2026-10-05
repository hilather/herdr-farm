-- Rebuild historical headroom and fleet inputs for the v7 definitions.
-- No new tables; existing derived retention/backup classifications apply.
UPDATE accounting_stream SET invalidated='schema_upgrade' WHERE singleton=1;
DELETE FROM accounting_dispatch_headroom;
DELETE FROM accounting_dispatch_frontier;
DELETE FROM accounting_fleet_snapshot;
