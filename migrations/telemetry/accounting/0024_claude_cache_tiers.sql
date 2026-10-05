-- Existing session projections retain follows_sources retention/full backup.
ALTER TABLE usage_entries ADD COLUMN cache_write_5m_tokens INTEGER CHECK (cache_write_5m_tokens BETWEEN 0 AND 9007199254740992);
ALTER TABLE usage_entries ADD COLUMN cache_write_1h_tokens INTEGER CHECK (cache_write_1h_tokens BETWEEN 0 AND 9007199254740992);
UPDATE accounting_stream SET invalidated='claude_mapping_v2' WHERE singleton=1;
