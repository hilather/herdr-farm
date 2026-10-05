# MET-LAUNCH-1 validation

2026-10-05; branch `factory/met-launch`; sandboxed offline run.

M90–M95 are registry v11 fixture-certified metadata. M96–M99 remain reserved. No canonical schema change or new dependency. Operations stream v2 owns typed CLI associations, bounded root-wide ticker class counts, and aggregate submission diffs; verification also retains counts in existing metadata. Missing observations stay unavailable/partial.

New coverage uses CLI/public store workflows only. Memory proposal promotion/rejection, retry/intervention counts, unavailable reasons, disabled telemetry, and real verifier whole-range counts passed. Positive running-time and acknowledged-brief assertions extend `cli::outcome_success_path`, which cannot reach those assertions here because its local socket fixture cannot bind. Steward must rerun it outside this sandbox.

## Requested suites

Every command used `TMPDIR=$PWD/target/tmp`, `nice -n 19 ionice -c 3`, `--locked --offline -j 3`, and `--features state-store`. Integration tests used `--no-fail-fast -- --test-threads=3`. No ignored live/paid tests were enabled.

| Suite | Passed | Failed | Ignored |
|---|---:|---:|---:|
| cli | 66 | 24 | 1 |
| memory_barriers | 6 | 0 | 0 |
| memory_control | 13 | 0 | 0 |
| memory_read_sets | 4 | 0 | 0 |
| memory_regressions | 15 | 0 | 0 |
| telemetry | 25 | 1 | 0 |
| telemetry_accounting | 35 | 1 | 0 |
| telemetry_certification | 13 | 0 | 0 |
| telemetry_claude | 18 | 0 | 0 |
| telemetry_collect | 6 | 0 | 0 |
| telemetry_compare | 6 | 0 | 0 |
| telemetry_conformance | 22 | 0 | 0 |
| telemetry_export | 7 | 0 | 0 |
| telemetry_gemini | 8 | 0 | 0 |
| telemetry_health | 14 | 1 | 0 |
| telemetry_launch | 2 | 0 | 0 |
| telemetry_live | 0 | 0 | 5 |
| telemetry_muse | 10 | 0 | 0 |
| telemetry_opencode | 5 | 0 | 0 |
| telemetry_operations | 15 | 0 | 0 |
| telemetry_otlp | 22 | 4 | 0 |
| telemetry_quality | 8 | 0 | 0 |
| telemetry_query | 19 | 0 | 0 |
| telemetry_review | 22 | 0 | 0 |
| telemetry_rework | 2 | 0 | 0 |
| telemetry_routines | 2 | 0 | 0 |
| telemetry_scale | 4 | 0 | 13 |
| telemetry_stored_metrics | 5 | 0 | 0 |
| telemetry_views | 9 | 0 | 0 |
| telemetry_workspace | 9 | 1 | 0 |

**Total: 392 passed, 32 socket-only failures, 19 ignored across 30 suites.**

## Socket-only failures

All direct bind failures report `Operation not permitted` (os error 1). `outcome_success_path` reports its Python stand-in socket bind PermissionError followed by a workflow wait timeout. No socket workaround was attempted.

### cli (Unix sockets)

- `canonical_ownership_cli_adopts_recorded_coordinator_without_prompting`
- `controller_captures_uncommitted_worker_edits_for_submission_and_verification`
- `hot_paths_skip_the_whole_store_check_and_the_ticker_checks_off_its_pass_then_pauses_admission_and_effects_on_corruption`
- `integration_releases_project_ownership_during_the_candidate_check`
- `launch_reserve_records_operator_reason`
- `native_ticker_claims_legacy_routine_and_restart_delivers_without_rerun`
- `operator_verify_releases_project_ownership_during_the_check`
- `rejected_reservation_writes_no_decision`
- `ticker_auto_chain_releases_verified_integrated_and_fan_in_dependents`
- `ticker_auto_integrates_two_results_serially_and_recovers_stale_and_crash`
- `ticker_auto_verification_releases_project_ownership_during_the_check`
- `ticker_auto_verifies_once_and_recovers_after_kill`
- `outcome_success_path`
- `ticker_canonical_notification_confirms_or_retains_ambiguity_after_owner_death`
- `ticker_canonical_observations_commit_cancel_and_restart_in_the_shared_pool`
- `ticker_coordinator_prime_confirms_or_recovers_once_across_restart`
- `ticker_coordinator_start_then_prime_recover_without_replaying_start`
- `ticker_local_and_remote_launches_acknowledge_once_and_recover_lost_replies`
- `ticker_native_briefs_confirm_or_recover_uncertainty_without_replay`
- `ticker_native_copy_publishes_announces_and_does_not_recopy_after_restart`
- `ticker_native_merged_finalization_resolves_and_replays_notice_after_restart`
- `ticker_notifications_recover_across_restart_and_reconcile_through_cli`
- `ticker_remote_briefs_confirm_or_recover_uncertainty_without_replay`
- `ticker_tokens_use_supervised_local_remote_and_coordinator_refreshes_after_restart`

### telemetry (Unix sockets)

- `attempts_show_attention_summary`

### telemetry_accounting (Unix sockets)

- `attention_intervals_union_and_censor`

### telemetry_health (Unix sockets)

- `recommendations_and_notices_change_no_canonical_state_and_no_dispatch`

### telemetry_otlp (TCP loopback)

- `devin_two_processes_with_same_session_and_sequence_count_both_requests_over_http`
- `http_auth_limits_malformed_and_replay`
- `http_protobuf_attempt_token_binding_auth_and_project_token_unchanged`
- `http_request_rate_is_bounded`

### telemetry_workspace (Unix sockets)

- `thread_start_records_the_dispatch_reason_and_the_sidebar_suffix`

## Limits

The three documented `copy_jobs::tests` TMPDIR/git-tree failures were not run; no changes were made to them. The integration suites above do not run binary unit tests. No owner data directories, live server/ticker, or real agent CLI were used. No push.

`cargo clippy --locked --offline -j 3 --features state-store --all-targets` completed successfully. Existing unrelated warnings remain; no warnings point to changed lines. `git diff --check` passed. The final `telemetry_launch` rerun after behavior-preserving clippy cleanup passed both workflows (2 passed).
