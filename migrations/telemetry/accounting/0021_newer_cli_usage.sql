-- USAGE-NEWER-1: the Rust version helper owns compatibility acceptance.
-- The marker is collector-created, never copied from an exporter attribute.
-- Preserve mapping identity and atomic normalized-counter checks.
-- No retained tables added; existing OTLP/session retention and backup apply.
DROP VIEW IF EXISTS otlp_ledger_sources;
CREATE VIEW otlp_ledger_sources AS
SELECT identity AS path_digest, adapter || ':' || identity AS session_id,
 adapter, attempt_id, observed_unix_ms,
 CAST(json_extract(record,'$.timeUnixNano') AS INTEGER)/1000000 AS session_unix_ms,
 substr(adapter,6) || '/' || json_extract(record,'$.cli_version') AS cli_version,
 json_extract(record,'$.attributes.model') AS model,
 json_extract(record,'$.attributes.input_tokens') AS input_tokens,
 json_extract(record,'$.attributes.cached_input_tokens') AS cached_input_tokens,
 json_extract(record,'$.attributes.cache_write_input_tokens') AS cache_write_input_tokens,
 json_extract(record,'$.attributes.output_tokens') AS output_tokens,
 json_extract(record,'$.attributes.reasoning_output_tokens') AS reasoning_output_tokens,
 json_extract(record,'$.attributes.total_tokens') AS total_tokens, 1 AS accepted, NULL AS reason
FROM otlp_records
WHERE binding='exact' AND attempt_id IS NOT NULL AND kind='usage'
 AND json_extract(record,'$.usage_authority') IN ('api_request','model_call')
 AND coalesce(json_extract(record,'$.mapping_certified'),'fixture') <> 'none'
 AND ((adapter='otlp:grok' AND (json_extract(record,'$.cli_version')='1.0.46' OR json_extract(record,'$.certification')='newer_than_certified')
       AND json_extract(record,'$.native_name')='grok_code.api_request'
       AND json_extract(record,'$.usage_authority')='api_request'
       AND json_extract(record,'$.usage_source_key') IS NOT NULL)
   OR (adapter='otlp:claude-code' AND (json_extract(record,'$.cli_version')='2.1.286' OR json_extract(record,'$.certification')='newer_than_certified')
       AND json_extract(record,'$.native_name')='claude_code.api_request')
   OR (adapter='otlp:muse' AND (json_extract(record,'$.cli_version')='1.4.0-R4161.1' OR json_extract(record,'$.certification')='newer_than_certified')
       AND json_extract(record,'$.native_name')='model_call')
   OR (adapter='otlp:devin' AND (json_extract(record,'$.cli_version')='3000.11.3' OR json_extract(record,'$.certification')='newer_than_certified')
       AND json_extract(record,'$.native_name')='api_request'
       AND json_extract(record,'$.usage_authority')='api_request'
       AND json_extract(record,'$.usage_source_key') IS NOT NULL))
 AND json_extract(record,'$.attributes.total_tokens') BETWEEN 0 AND 9007199254740992
 AND json_extract(record,'$.attributes.cached_input_tokens') + json_extract(record,'$.attributes.cache_write_input_tokens') <= json_extract(record,'$.attributes.input_tokens')
 AND json_extract(record,'$.attributes.reasoning_output_tokens') <= json_extract(record,'$.attributes.output_tokens')
UNION ALL
SELECT identity, adapter || ':' || identity, adapter, attempt_id, observed_unix_ms,
 CAST(json_extract(record,'$.timeUnixNano') AS INTEGER)/1000000,
 substr(adapter,6) || '/' || json_extract(record,'$.cli_version'),
 NULL,NULL,NULL,NULL,NULL,NULL,NULL,0,'schema_unrecognized'
FROM otlp_records
WHERE binding='exact' AND attempt_id IS NOT NULL AND kind='unmapped'
 AND adapter IN ('otlp:grok','otlp:claude-code','otlp:muse','otlp:devin')
 AND json_extract(record,'$.certification')='newer_than_certified'
 AND json_extract(record,'$.reason') IN ('schema_unrecognized','missing_usage_identity');

DROP TRIGGER IF EXISTS accounting_otlp_insert;
DROP TRIGGER IF EXISTS accounting_otlp_update;
DROP TRIGGER IF EXISTS accounting_otlp_delete;
CREATE TRIGGER IF NOT EXISTS accounting_otlp_insert AFTER INSERT ON otlp_records
WHEN NEW.identity IN (SELECT path_digest FROM otlp_ledger_sources) BEGIN
UPDATE accounting_stream SET sequence=sequence+1;
INSERT OR IGNORE INTO accounting_dirty_sessions SELECT session_id FROM otlp_ledger_sources WHERE path_digest=NEW.identity;
INSERT OR IGNORE INTO rollout_sources(path_digest,home_digest,session_id,session_unix_ms,cwd,cli_version,originator,source,records,binding,attempt_id,observed_unix_ms)
SELECT path_digest,'otlp',session_id,session_unix_ms,'',cli_version,adapter,adapter,1,'bound',attempt_id,observed_unix_ms FROM otlp_ledger_sources WHERE path_digest=NEW.identity;
INSERT OR IGNORE INTO codex_usage(session_id,ordinal,path_digest,response_id,model,payload_digest,input_tokens,cached_input_tokens,cache_write_input_tokens,output_tokens,reasoning_output_tokens,total_tokens,accepted,reason,observed_unix_ms)
SELECT session_id,1,path_digest,path_digest,model,path_digest,input_tokens,cached_input_tokens,cache_write_input_tokens,output_tokens,reasoning_output_tokens,total_tokens,accepted,reason,observed_unix_ms FROM otlp_ledger_sources WHERE path_digest=NEW.identity;
INSERT OR IGNORE INTO codex_usage_times(session_id,ordinal,record_unix_ms) SELECT session_id,1,session_unix_ms FROM otlp_ledger_sources WHERE path_digest=NEW.identity;
END;
CREATE TRIGGER IF NOT EXISTS accounting_otlp_update AFTER UPDATE ON otlp_records
WHEN NEW.identity IN (SELECT path_digest FROM otlp_ledger_sources)
 OR OLD.identity IN (SELECT path_digest FROM rollout_sources WHERE originator LIKE 'otlp:%') BEGIN
UPDATE accounting_stream SET sequence=sequence+1;
INSERT OR IGNORE INTO accounting_dirty_sessions VALUES(OLD.adapter || ':' || OLD.identity);
DELETE FROM codex_usage_times WHERE session_id=OLD.adapter || ':' || OLD.identity;
DELETE FROM codex_usage WHERE path_digest=OLD.identity;
DELETE FROM rollout_sources WHERE path_digest=OLD.identity;
INSERT OR IGNORE INTO accounting_dirty_sessions SELECT session_id FROM otlp_ledger_sources WHERE path_digest=NEW.identity;
INSERT OR IGNORE INTO rollout_sources(path_digest,home_digest,session_id,session_unix_ms,cwd,cli_version,originator,source,records,binding,attempt_id,observed_unix_ms)
SELECT path_digest,'otlp',session_id,session_unix_ms,'',cli_version,adapter,adapter,1,'bound',attempt_id,observed_unix_ms FROM otlp_ledger_sources WHERE path_digest=NEW.identity;
INSERT OR IGNORE INTO codex_usage(session_id,ordinal,path_digest,response_id,model,payload_digest,input_tokens,cached_input_tokens,cache_write_input_tokens,output_tokens,reasoning_output_tokens,total_tokens,accepted,reason,observed_unix_ms)
SELECT session_id,1,path_digest,path_digest,model,path_digest,input_tokens,cached_input_tokens,cache_write_input_tokens,output_tokens,reasoning_output_tokens,total_tokens,accepted,reason,observed_unix_ms FROM otlp_ledger_sources WHERE path_digest=NEW.identity;
INSERT OR IGNORE INTO codex_usage_times(session_id,ordinal,record_unix_ms) SELECT session_id,1,session_unix_ms FROM otlp_ledger_sources WHERE path_digest=NEW.identity;
END;
CREATE TRIGGER IF NOT EXISTS accounting_otlp_delete AFTER DELETE ON otlp_records
WHEN OLD.identity IN (SELECT path_digest FROM rollout_sources WHERE originator LIKE 'otlp:%') BEGIN
UPDATE accounting_stream SET sequence=sequence+1;
INSERT OR IGNORE INTO accounting_dirty_sessions VALUES(OLD.adapter || ':' || OLD.identity);
DELETE FROM codex_usage_times WHERE session_id=OLD.adapter || ':' || OLD.identity;
DELETE FROM codex_usage WHERE path_digest=OLD.identity;
DELETE FROM rollout_sources WHERE path_digest=OLD.identity;
END;

INSERT OR IGNORE INTO rollout_sources(path_digest,home_digest,session_id,session_unix_ms,cwd,cli_version,originator,source,records,binding,attempt_id,observed_unix_ms)
SELECT path_digest,'otlp',session_id,session_unix_ms,'',cli_version,adapter,adapter,1,'bound',attempt_id,observed_unix_ms FROM otlp_ledger_sources;
INSERT OR IGNORE INTO codex_usage(session_id,ordinal,path_digest,response_id,model,payload_digest,input_tokens,cached_input_tokens,cache_write_input_tokens,output_tokens,reasoning_output_tokens,total_tokens,accepted,reason,observed_unix_ms)
SELECT session_id,1,path_digest,path_digest,model,path_digest,input_tokens,cached_input_tokens,cache_write_input_tokens,output_tokens,reasoning_output_tokens,total_tokens,accepted,reason,observed_unix_ms FROM otlp_ledger_sources;
INSERT OR IGNORE INTO codex_usage_times(session_id,ordinal,record_unix_ms) SELECT session_id,1,session_unix_ms FROM otlp_ledger_sources;
UPDATE accounting_stream SET invalidated='schema_upgrade' WHERE singleton=1;
