# Claude Code 2.1.286 live certification

**Certified live on 2026-10-01** by the steward's owner-approved run of
`tests/telemetry_live.rs::claude_live` on branch `telemetry/lc3-claude-live`.
Two turns (`-p … --output-format json --tools "" --model claude-haiku-4-5-20251001`,
then `-c -p …`) in a disposable execution home.

Authentication (owner decision): a long-lived `claude setup-token` token passed
through `HERDR_LIVE_TOKEN_FILE` / `CLAUDE_CODE_OAUTH_TOKEN` to the Claude
process only. The owner's OAuth credentials were never copied. The home was
deleted after the run.

Original normalization (`claude-code-v1`, DG4b): total input = native `input_tokens` +
`cache_read_input_tokens` + `cache_creation_input_tokens`. Mapping v2 now reads reported thinking counters; absent counters remain
`reasoning_tokens_not_reported`, never zero-as-measured.

| | Input (total) | Cache read | Cache write | Output | Total |
| --- | ---: | ---: | ---: | ---: | ---: |
| Turn 1 (ledger) | 6,960 | 0 | 6,950 | 96 | 7,056 |
| Turn 2 (ledger) | 7,120 | 6,950 | 160 | 47 | 7,167 |
| Ledger total | 14,080 | 6,950 | 7,110 | 143 | 14,223 |
| Claude's own stdout `usage`, normalized | 14,080 | 6,950 | 7,110 | 143 | 14,223 |
| Difference | 0 | 0 | 0 | 0 | 0 |

Model `claude-haiku-4-5-20251001` was preserved verbatim (LC3-b `ModelId`). Binding was
`bound`. The privacy marker scan over telemetry.db, including
WAL/SHM, found 0 hits. Unmapped stdout usage keys (keys only):
`fallback_credit`, `inference_geo`, `iterations`, `server_tool_use`, `service_tier`, `speed`.

## Findings this certification fixed

1. **Discovery (LC3):** Claude Code names project directories by replacing
   *every* non-alphanumeric character of the cwd with `-` (`/.state` → `--state`).
   DG4b replaced only `/`, so it never found sessions under
   `.state/worktrees/…`, where every attempt worktree lives. The first live run
   was unbound with zero records. Discovery now walks all project directories
   and binds only from each line's absolute `cwd`.
2. **Model ids (LC3-b):** the generic long-token masker redacted dated model ids.
   The new strict `ModelId` class keeps model identifiers verbatim and still
   rejects secret-shaped values.

## Scope

`certified_versions` = 2.1.286. Live fields: `sessionId`, `timestamp`, `cwd`
(binding only), `version`, `type`, `message.model`, `message.id` and the four
`message.usage` counters. Tool-use and tool-result fields and `isSidechain`
remain fixture-certified, because this run used no tools or subagents.

TFIX-4 mapping v2 adds the live-observed metadata paths `effort`,
`message.usage.output_tokens_details.thinking_tokens`,
`message.usage.cache_creation.ephemeral_5m_input_tokens` and
`message.usage.cache_creation.ephemeral_1h_input_tokens` to `LIVE_FIELDS`.
These metadata keys are now mapped rather than listed as unmapped; this does
not certify thinking content or add a new live run. See the collection contract
for the migration/rebuild of v1 sessions.

## Newly mapped turn keys (synthetic certification)

Claude turn metadata adds exactly these leaves: non-tool-result user lines
`turnOrigin`, `promptSource` (enum tags), `turnPosition.promptIndex`,
`turnPosition.turnIndex` (numbers), `promptId` (id); assistant lines
`requestId` (id), `message.stop_reason` (enum tag); system lines `subtype`
(enum tag), `durationMs` (number); tool results
`toolUseResult.backgroundTaskId` (id only); queue-operation lines `operation`
(enum tag only). Enum values must match `^[a-z][a-z0-9_-]{0,31}$`;
all other strings become `other`. These are metadata, never message content.
No other leaf inside `toolUseResult` is read, stored or hashed.

These leaves are fixture-certified; no additional owner transcript or live agent probe was used.
