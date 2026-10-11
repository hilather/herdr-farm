# Coordinator wake on submit

Unfinished worker-result inbox items queue a canonical coordinator wake. With
`nudge = true`, only owned native coordinators reporting idle or done receive a
prompt. Busy coordinators persist the batch hash and a retry one normal tick
later without adding a wake attempt. Context reads mark SEEN and do not consume
this DONE-based wake policy.

Wake delivery holds the project effect guard and a shared root barrier, following
the supervised notification path. Foreground wakes use the same supervised calls.
Transferred locks remain held until descendants exit; the wake does not acquire
exclusive root ownership or suppress all controller services.

Detached ticker start waits for its child's published lock ownership, child exit,
or a five-second deadline. An immediate stop can therefore see the started ticker.
Bounded test runs refuse an existing ticker with a nonzero exit instead of
reporting success with zero passes. After the requested passes, they wait up to
60 seconds for pending copy/effect jobs, settling and logging completions before
executor shutdown. A stop file still cancels promptly. Continuous production
runs retain their existing shutdown behavior.

Replaying a worker submission means submitting the same retained submission file.
Recapturing creates a new result ID and legitimately permits another wake.
Confirmed batches are deduplicated across restart; ambiguous native replies fence
overlapping items until DONE. Claims persisted before a native call still become
uncertain on restart: the current retained claim does not distinguish a pre-call
stop from a lost reply. Removing that ambiguity needs a durable call-start marker
and end-to-end interruption coverage; this fix preserves the conservative fence.

No canonical schema or dependency changes are required. Wake claims and retry
state remain in `.state/canonical-wake.json`, retained with project backups.

## Fix-round validation

All cargo commands used `TMPDIR=$PWD/target/tmp`, `nice -n 19 ionice -c 3`,
`--locked --offline -j 3`. `cargo test --no-run` compiled all tests. Clippy with
`--features state-store --all-targets` passed; existing warnings do not point to
changed lines. No repository-wide formatting was run.

The requested suites completed with these results:

| Suite | Passed | Socket-only failures | Ignored |
| --- | ---: | ---: | ---: |
| canonical_coordinator | 12 | 7 | 0 |
| canonical_worker | 11 | 57 | 0 |
| cli | 69 | 24 | 1 |
| telemetry_launch | 3 | 0 | 0 |
| ticker_timing | 5 | 0 | 0 |

A preliminary direct timing run during compilation exceeded two deadlines;
after compilation, all five timing tests passed both on direct recheck and in
the requested suite run, without changing deadlines. The new lifecycle test
also passed an isolated recheck.

Socket binds fail with `Operation not permitted` in this sandbox. Worker
fixtures that bind in Python subsequently fail their existing ten-second socket
startup wait (`tests/canonical_worker.rs:275`); the CLI `outcome_success_path`
has the same failure mechanism (`tests/cli.rs:3193`). Other failures below are
at direct socket binds. The steward must verify these workflows outside the
sandbox, including the three reported wake regressions. No socket workaround
was used. The separately identified `copy_jobs::tests` affected by temporary
directories inside a git checkout were not run in these integration suites.

The exact socket-only failures follow.

<details>
<summary>canonical_coordinator (7)</summary>

- `canonical_toast_backoff_and_live_nudge_policy_survive_context_reads`
- `canonical_wake_lost_reply_fences_replay_until_done_and_releases_new_results`
- `migrated_doctor_reports_canonical_binding_despite_stale_legacy_record`
- `open_waits_for_brief_project_and_root_contention`
- `socket_coordinator_remote_manifest_and_explicit_local_override_policy`
- `socket_open_primes_owned_coordinator_retries_swallowed_prompt_and_recreates_closed_pane`
- `unchanged_ticker_observations_preserve_head_and_refresh_coordinator_admission`

</details>

<details>
<summary>canonical_worker (57)</summary>

- `a_brief_swallowed_by_the_agent_is_redelivered_and_confirmed_only_once_accepted`
- `a_brief_the_agent_never_accepts_is_left_ambiguous_not_confirmed`
- `a_hidden_path_covering_the_execution_home_refuses_the_launch_before_creation`
- `a_launch_outlasts_an_operator_holding_the_root_after_its_workspace_creation`
- `a_launch_reaches_running_while_another_holder_takes_the_shared_root_intermittently`
- `a_legacy_thread_holding_the_planned_worktree_blocks_its_creation`
- `a_proven_worker_end_keeps_the_project_admitted_but_an_unexplained_pane_loss_pauses_it`
- `a_sandboxed_reviewer_uses_its_worker_channel_through_the_spool`
- `a_subdirectory_binding_runs_in_the_same_subdirectory_of_the_new_worktree`
- `a_worker_branch_reaching_a_corrupt_quarantined_object_is_refused`
- `a_worker_that_dies_without_submitting_is_noticed_and_preserved_while_another_attempts_check_runs`
- `a_worker_that_exits_after_submitting_is_recorded_terminated_while_another_attempts_check_runs`
- `a_worker_that_exits_during_its_own_verification_ends_after_the_verdict_without_contention`
- `accepted_editing_worker_completes_automatically_after_integration`
- `accepted_verify_only_editing_worker_completes_without_integration_automation`
- `ambiguous_creation_with_removed_dedicated_server_releases_capacity_once`
- `an_automatic_pause_names_its_blockers_and_lifts_itself_once_they_clear`
- `an_isolated_codex_worker_commits_through_codex_workspace_write_sandbox`
- `an_isolated_worker_cannot_read_owner_secrets_or_lift_the_hiding_but_still_commits_and_submits`
- `an_isolated_worker_submits_only_through_its_own_spool`
- `an_operator_finishes_a_worker_that_never_submitted_and_the_result_lands_automatically`
- `an_untracked_working_directory_is_refused_before_the_approval_is_used`
- `attention_mid_run_failure_remains_incomplete_at_termination`
- `barrier_stop_gets_a_fair_turn_and_maintenance_keeps_advancing`
- `canonical_attempt_sidebar_clears_after_termination_in_an_active_project`
- `canonical_attempt_sidebar_does_not_publish_to_a_replaced_terminal`
- `canonical_attempt_sidebar_refreshes_and_clears_on_pause_and_termination`
- `canonical_attempt_sidebar_restart_offers_no_historical_cleanup_or_native_request`
- `canonical_attempt_sidebar_uses_collected_usage_and_observed_waiting`
- `concurrent_attempt_tokens_publish_and_missing_retired_server_stays_quiet`
- `dedicated_worker_refuses_remote_manifest_before_its_brief`
- `editing_worker_requires_operator_completion_when_automation_is_off`
- `idle_worker_notice_restarts_stretch_and_deduplicates_within_a_ticker`
- `launch_revision_conflict_reselects_without_duplicate_resources_or_brief`
- `launch_run_reviews_use_the_claude_spool_and_record_skeptical_yield`
- `launch_run_reviews_use_the_codex_spool_and_record_skeptical_yield`
- `launch_sets_the_intended_permission_mode_over_a_stale_one_in_the_home`
- `persistent_launch_conflict_closes_attempt_and_notifies_once`
- `proven_launch_server_loss_retires_owned_runtime_and_next_brief_succeeds`
- `rejected_editing_worker_stays_running_and_can_resubmit`
- `repeated_nontransient_recovery_stops_and_escalates_with_capacity_retained`
- `review_assignment_launches_with_blind_brief_and_records_session`
- `stale_sealed_brief_authority_fails_once_without_prompt_or_retry`
- `submit_captured_retains_remember_from_the_attempt_report_and_replays_once`
- `three_transient_missing_socket_observations_retain_capacity`
- `ticker_does_not_dispatch_a_launch_cancelled_before_creation`
- `ticker_launches_and_briefs_once_then_stops_a_cancelled_worker_while_paused_and_revoked`
- `ticker_launches_nothing_on_a_server_without_the_launch_contract_or_while_paused`
- `ticker_recovers_a_lost_creation_reply_without_creating_again`
- `ticker_retires_a_cancelled_gated_worker_without_starting_it`
- `ticker_stops_the_dedicated_herdr_server_of_a_finished_task`
- `verified_worker_results_wake_idle_coordinator_once_and_defer_while_busy`
- `wall_budget_termination_retries_contention_and_notifies_once`
- `wall_budget_termination_with_changed_frozen_definition_keeps_process_exit`
- `wall_budget_termination_without_contention`
- `worker_uid_default_root_and_verification_stay_frozen`
- `worker_uid_owner_and_verification_stay_frozen_with_mounts_enforced`

</details>

<details>
<summary>cli (24)</summary>

- `canonical_ownership_cli_adopts_recorded_coordinator_without_prompting`
- `controller_captures_uncommitted_worker_edits_for_submission_and_verification`
- `hot_paths_skip_the_whole_store_check_and_the_ticker_checks_off_its_pass_then_pauses_admission_and_effects_on_corruption`
- `integration_releases_project_ownership_during_the_candidate_check`
- `launch_reserve_records_operator_reason`
- `native_ticker_claims_legacy_routine_and_restart_delivers_without_rerun`
- `operator_verify_releases_project_ownership_during_the_check`
- `outcome_success_path`
- `rejected_reservation_writes_no_decision`
- `ticker_auto_chain_releases_verified_integrated_and_fan_in_dependents`
- `ticker_auto_integrates_two_results_serially_and_recovers_stale_and_crash`
- `ticker_auto_verification_releases_project_ownership_during_the_check`
- `ticker_auto_verifies_once_and_recovers_after_kill`
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

</details>
