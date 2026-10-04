-- CHILD-COST-1: a thread_spawn subagent whose fork origin is its spawning
-- parent is a spawned child (inclusion separate), not an uncertified fork.
-- The session graph is stored at sync, so rebuild it once.
UPDATE accounting_stream SET invalidated='schema_upgrade' WHERE singleton=1;
