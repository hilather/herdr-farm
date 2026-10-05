# COORD-SESSIONS-1 validation

Implemented on factory/coord-sessions; no push.

## Design and files

- Ingest migration 0015 and the ingest MIGRATIONS list add session-retained
  claude_turn_lines, keyed by session, source digest and byte offset.
  Maintenance retention and backup inventories classify the new table with
  normalized sessions. Existing Claude offsets replay; envelopes use normalization v3.
- claude.rs and sanitize.rs collect only the approved leaves, with strict
  EnumTag validation and no traversal of toolUseResult siblings.
- accounting/claude_turns.rs, accounting/fleet.rs and accounting/mod.rs expose
  accounting coordinator (JSON, including cross-session summaries) and Claude
  turn detail in M34. Codex-only M34 output remains compatible; stale detail
  wording is corrected. outcome.rs adds worker turns and stop_reasons.
- All four requested telemetry documents are updated. operations-runbook.md
  includes the new backup inventory row.
- CLI E2E coverage uses tests/fixtures/telemetry/claude-code/turns.jsonl and
  telemetry_claude.rs. Existing conformance migration expectations advance to
  version 15. No new unit tests, source-text assertions, crates or process
  spawns in src were added; no repository-wide formatting was run.

Trigger classification is a documented metadata heuristic. A rewrite means
positive cache creation with zero reported cache reads. No-tool turns are
candidates for no-op wake-ups, not a judgment about their output. Missing cache
tier sums remain null; unpriced requests remain explicit. Newly mapped leaves
have synthetic fixture certification, not additional live certification.

## Checks

Every cargo command used TMPDIR=$PWD/target/tmp and
nice -n 19 ionice -c 3 cargo with --locked --offline -j 3.

All 23 telemetry/CLI integration suites ran with --no-fail-fast.
Results below combine the full runs with final reruns after corrections.
Final Claude: 18 passed; conformance: 22 passed; accounting: 34 passed and one
socket-only failure. The backup runbook replay passed. The operating-clock
timing test and two CLI delivery timing assertions failed under concurrent
load and passed isolated reruns; the scale ticker collection test passed the
repeat full suite. Migration expectation and Codex M34 compatibility failures
were corrected and their suites rerun.

Combined verified results: **335 passed, 32 socket-limited failures, 19 ignored**.

| Suite | Passed | Socket-limited | Ignored |
|---|---:|---:|---:|
| cli.rs | 63 | 24 | 1 |
| telemetry.rs | 23 | 1 | 0 |
| telemetry_accounting.rs | 34 | 1 | 0 |
| telemetry_certification.rs | 13 | 0 | 0 |
| telemetry_claude.rs | 18 | 0 | 0 |
| telemetry_collect.rs | 6 | 0 | 0 |
| telemetry_compare.rs | 6 | 0 | 0 |
| telemetry_conformance.rs | 22 | 0 | 0 |
| telemetry_export.rs | 7 | 0 | 0 |
| telemetry_gemini.rs | 8 | 0 | 0 |
| telemetry_health.rs | 14 | 1 | 0 |
| telemetry_live.rs | 0 | 0 | 5 |
| telemetry_muse.rs | 10 | 0 | 0 |
| telemetry_opencode.rs | 5 | 0 | 0 |
| telemetry_operations.rs | 14 | 0 | 0 |
| telemetry_otlp.rs | 22 | 4 | 0 |
| telemetry_quality.rs | 8 | 0 | 0 |
| telemetry_query.rs | 16 | 0 | 0 |
| telemetry_review.rs | 22 | 0 | 0 |
| telemetry_routines.rs | 2 | 0 | 0 |
| telemetry_scale.rs | 4 | 0 | 13 |
| telemetry_views.rs | 9 | 0 | 0 |
| telemetry_workspace.rs | 9 | 1 | 0 |


cargo clippy --locked --offline -j 3 --features state-store --all-targets
completed successfully, with no warnings in changed lines. Existing unrelated
warnings remain. git diff --check passed.

## Socket-only failures

These fixtures cannot bind sockets in this hard sandbox (Operation not
permitted). No workaround was used. outcome_success_path waits for its two
synthetic Unix-socket servers after their Python bind calls receive
PermissionError; its resulting timeout is the same socket restriction.
The OTLP HTTP fixtures are also socket-limited.

### cli.rs

- canonical_ownership_cli_adopts_recorded_coordinator_without_prompting
- controller_captures_uncommitted_worker_edits_for_submission_and_verification
- hot_paths_skip_the_whole_store_check_and_the_ticker_checks_off_its_pass_then_pauses_admission_and_effects_on_corruption
- integration_releases_project_ownership_during_the_candidate_check
- launch_reserve_records_operator_reason
- native_ticker_claims_legacy_routine_and_restart_delivers_without_rerun
- operator_verify_releases_project_ownership_during_the_check
- outcome_success_path
- rejected_reservation_writes_no_decision
- ticker_auto_chain_releases_verified_integrated_and_fan_in_dependents
- ticker_auto_integrates_two_results_serially_and_recovers_stale_and_crash
- ticker_auto_verification_releases_project_ownership_during_the_check
- ticker_auto_verifies_once_and_recovers_after_kill
- ticker_canonical_notification_confirms_or_retains_ambiguity_after_owner_death
- ticker_canonical_observations_commit_cancel_and_restart_in_the_shared_pool
- ticker_coordinator_prime_confirms_or_recovers_once_across_restart
- ticker_coordinator_start_then_prime_recover_without_replaying_start
- ticker_local_and_remote_launches_acknowledge_once_and_recover_lost_replies
- ticker_native_briefs_confirm_or_recover_uncertainty_without_replay
- ticker_native_copy_publishes_announces_and_does_not_recopy_after_restart
- ticker_native_merged_finalization_resolves_and_replays_notice_after_restart
- ticker_notifications_recover_across_restart_and_reconcile_through_cli
- ticker_remote_briefs_confirm_or_recover_uncertainty_without_replay
- ticker_tokens_use_supervised_local_remote_and_coordinator_refreshes_after_restart

### telemetry.rs

- attempts_show_attention_summary

### telemetry_accounting.rs

- attention_intervals_union_and_censor

### telemetry_health.rs

- recommendations_and_notices_change_no_canonical_state_and_no_dispatch

### telemetry_otlp.rs

- devin_two_processes_with_same_session_and_sequence_count_both_requests_over_http
- http_auth_limits_malformed_and_replay
- http_protobuf_attempt_token_binding_auth_and_project_token_unchanged
- http_request_rate_is_bounded

### telemetry_workspace.rs

- thread_start_records_the_dispatch_reason_and_the_sidebar_suffix

## Limits

The steward must rerun socket fixtures outside the sandbox. Existing ignored
live/resource suites stayed ignored; no real transcripts, live agent probes,
owner agent directories, live server or ticker were accessed. No live or paid
external services were used. The three named copy_jobs unit tests were not run
and were not changed; their documented TMPDIR/git-tree limitation still applies.
