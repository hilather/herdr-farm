# MET-NOW-B validation

Implemented M60–M64 as metadata-only report metrics in registry v8. The operations sidecar stream has a versioned v1 migration. Launch load is best-effort and does not change canonical receipts; storage sampling is at most hourly, bounded to 90 days / 2,160 rows, and participates in retention, backup, restore, and tombstones. Idle gaps reuse fleet lifecycle and operating intervals, preserving unknown history.

Implementation: `src/telemetry/operations.rs`, fleet accounting, registry/report rendering, ticker lane registration, launch creation sampling, maintenance classification, and `migrations/telemetry/operations/0001_samples.sql`. Definitions and absence reasons are documented in the three telemetry contracts; the operations runbook snapshot is updated. New coverage uses public CLI/store workflows and temporary projects.

## Verification

Every command used `TMPDIR=$PWD/target/tmp nice -n 19 ionice -c 3 cargo`, with `--locked --offline -j 3` and `--features state-store`. No repo-wide formatter, live services, paid agents, or owner data directories were used.

All 22 telemetry targets, `cli`, and `canonical_worker` were run with `--no-fail-fast`. Initial results and final isolated corrections:

| Suite | Initial passed | Initial failed | Ignored | Final correction |
| --- | ---: | ---: | ---: | --- |
| `canonical_worker` | 6 | 37 | 0 |  |
| `cli` | 64 | 26 | 1 | 2 non-socket failures passed isolated reruns; 24 socket-blocked |
| `telemetry` | 23 | 2 | 0 | Legacy sidecar upgrade passed after idempotent migration correction; 1 socket-blocked |
| `telemetry_accounting` | 34 | 1 | 0 |  |
| `telemetry_certification` | 13 | 0 | 0 |  |
| `telemetry_claude` | 17 | 1 | 0 | Native-source precedence passed isolated rerun |
| `telemetry_collect` | 6 | 0 | 0 |  |
| `telemetry_compare` | 6 | 0 | 0 |  |
| `telemetry_conformance` | 22 | 0 | 0 |  |
| `telemetry_export` | 7 | 0 | 0 |  |
| `telemetry_gemini` | 8 | 0 | 0 |  |
| `telemetry_health` | 14 | 1 | 0 |  |
| `telemetry_live` | 0 | 0 | 5 |  |
| `telemetry_muse` | 10 | 0 | 0 |  |
| `telemetry_opencode` | 5 | 0 | 0 |  |
| `telemetry_operations` | 15 | 0 | 0 |  |
| `telemetry_otlp` | 22 | 4 | 0 |  |
| `telemetry_quality` | 8 | 0 | 0 |  |
| `telemetry_query` | 18 | 0 | 0 | Final full rerun: 19 passed, including new overlap/window fixture |
| `telemetry_review` | 22 | 0 | 0 |  |
| `telemetry_routines` | 2 | 0 | 0 |  |
| `telemetry_scale` | 3 | 1 | 13 | Ticker readiness passed isolated rerun; 13 opt-in tests remain ignored |
| `telemetry_views` | 9 | 0 | 0 |  |
| `telemetry_workspace` | 9 | 1 | 0 |  |

Final focused reruns: `telemetry_query` **19 passed**, `telemetry_operations` **15 passed**. The CLI fleet text fixture and lock-wait fixture, legacy sidecar upgrade, Claude native-source precedence fixture, and scale ticker fixture all passed isolated reruns. Initial non-socket failures are resolved; 69 socket-blocked failures remain. Aggregate with successful reruns and the added overlap fixture: **349 passed, 69 sandbox-blocked, 19 ignored**.

`cargo clippy --locked --offline -j 3 --features state-store --all-targets` completed successfully. Existing warnings remain, with none on changed lines. `git diff --check` passed.

Paid/live tests (5), opt-in scale tests (13), and one pre-existing CLI ignored test were not enabled. Canonical launch integration and socket-dependent workflows cannot be validated successfully in this sandbox; the steward must rerun the following outside it. No socket workaround was attempted.

## Socket-only failures

Unix socket binds fail with `Operation not permitted`; canonical-worker fixtures subsequently time out waiting for their fixture server. The four OTLP tests also require TCP loopback binds, which this sandbox denies.

### `canonical_worker` (37)

- `a_launch_reaches_running_while_another_holder_takes_the_shared_root_intermittently`
- `a_legacy_thread_holding_the_planned_worktree_blocks_its_creation`
- `a_brief_the_agent_never_accepts_is_left_ambiguous_not_confirmed`
- `a_brief_swallowed_by_the_agent_is_redelivered_and_confirmed_only_once_accepted`
- `a_hidden_path_covering_the_execution_home_refuses_the_launch_before_creation`
- `a_proven_worker_end_keeps_the_project_admitted_but_an_unexplained_pane_loss_pauses_it`
- `a_subdirectory_binding_runs_in_the_same_subdirectory_of_the_new_worktree`
- `a_worker_branch_reaching_a_corrupt_quarantined_object_is_refused`
- `a_sandboxed_reviewer_uses_its_worker_channel_through_the_spool`
- `accepted_editing_worker_completes_automatically_after_integration`
- `accepted_verify_only_editing_worker_completes_without_integration_automation`
- `an_isolated_codex_worker_commits_through_codex_workspace_write_sandbox`
- `an_isolated_worker_cannot_read_owner_secrets_or_lift_the_hiding_but_still_commits_and_submits`
- `an_isolated_worker_submits_only_through_its_own_spool`
- `an_untracked_working_directory_is_refused_before_the_approval_is_used`
- `an_operator_finishes_a_worker_that_never_submitted_and_the_result_lands_automatically`
- `attention_mid_run_failure_remains_incomplete_at_termination`
- `canonical_attempt_sidebar_clears_after_termination_in_an_active_project`
- `canonical_attempt_sidebar_does_not_publish_to_a_replaced_terminal`
- `canonical_attempt_sidebar_refreshes_and_clears_on_pause_and_termination`
- `canonical_attempt_sidebar_uses_collected_usage_and_observed_waiting`
- `canonical_attempt_sidebar_restart_offers_no_historical_cleanup_or_native_request`
- `dedicated_worker_refuses_remote_manifest_before_its_brief`
- `launch_run_reviews_use_the_codex_spool_and_record_skeptical_yield`
- `launch_run_reviews_use_the_claude_spool_and_record_skeptical_yield`
- `concurrent_attempt_tokens_publish_and_missing_retired_server_stays_quiet`
- `editing_worker_requires_operator_completion_when_automation_is_off`
- `launch_sets_the_intended_permission_mode_over_a_stale_one_in_the_home`
- `rejected_editing_worker_stays_running_and_can_resubmit`
- `submit_captured_retains_remember_from_the_attempt_report_and_replays_once`
- `ticker_does_not_dispatch_a_launch_cancelled_before_creation`
- `review_assignment_launches_with_blind_brief_and_records_session`
- `ticker_launches_and_briefs_once_then_stops_a_cancelled_worker_while_paused_and_revoked`
- `ticker_launches_nothing_on_a_server_without_the_launch_contract_or_while_paused`
- `ticker_recovers_a_lost_creation_reply_without_creating_again`
- `ticker_stops_the_dedicated_herdr_server_of_a_finished_task`
- `ticker_retires_a_cancelled_gated_worker_without_starting_it`

### `cli` (24)

- `canonical_ownership_cli_adopts_recorded_coordinator_without_prompting`
- `controller_captures_uncommitted_worker_edits_for_submission_and_verification`
- `integration_releases_project_ownership_during_the_candidate_check`
- `hot_paths_skip_the_whole_store_check_and_the_ticker_checks_off_its_pass_then_pauses_admission_and_effects_on_corruption`
- `launch_reserve_records_operator_reason`
- `native_ticker_claims_legacy_routine_and_restart_delivers_without_rerun`
- `operator_verify_releases_project_ownership_during_the_check`
- `outcome_success_path`
- `rejected_reservation_writes_no_decision`
- `ticker_auto_chain_releases_verified_integrated_and_fan_in_dependents`
- `ticker_auto_integrates_two_results_serially_and_recovers_stale_and_crash`
- `ticker_auto_verification_releases_project_ownership_during_the_check`
- `ticker_auto_verifies_once_and_recovers_after_kill`
- `ticker_canonical_observations_commit_cancel_and_restart_in_the_shared_pool`
- `ticker_canonical_notification_confirms_or_retains_ambiguity_after_owner_death`
- `ticker_coordinator_start_then_prime_recover_without_replaying_start`
- `ticker_coordinator_prime_confirms_or_recovers_once_across_restart`
- `ticker_local_and_remote_launches_acknowledge_once_and_recover_lost_replies`
- `ticker_native_briefs_confirm_or_recover_uncertainty_without_replay`
- `ticker_native_copy_publishes_announces_and_does_not_recopy_after_restart`
- `ticker_native_merged_finalization_resolves_and_replays_notice_after_restart`
- `ticker_notifications_recover_across_restart_and_reconcile_through_cli`
- `ticker_remote_briefs_confirm_or_recover_uncertainty_without_replay`
- `ticker_tokens_use_supervised_local_remote_and_coordinator_refreshes_after_restart`

### `telemetry` (1)

- `attempts_show_attention_summary`

### `telemetry_accounting` (1)

- `attention_intervals_union_and_censor`

### `telemetry_health` (1)

- `recommendations_and_notices_change_no_canonical_state_and_no_dispatch`

### `telemetry_otlp` (4)

- `devin_two_processes_with_same_session_and_sequence_count_both_requests_over_http`
- `http_auth_limits_malformed_and_replay`
- `http_request_rate_is_bounded`
- `http_protobuf_attempt_token_binding_auth_and_project_token_unchanged`

### `telemetry_workspace` (1)

- `thread_start_records_the_dispatch_reason_and_the_sidebar_suffix`

