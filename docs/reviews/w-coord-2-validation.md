# W-COORD-2 validation

Canonical `open` uses a version 1 effect journal, native readiness and acceptance
checks, canonical runtime routing, fresh observation and ownership. Repeated open
reuses the accepted coordinator; swallowed primes are retried at most three times.
Interrupted sends retain pending intent. Recreating an owned, closed pane requires
fresh absence evidence from the same session; withdrawal and rebinding are atomic.
No SQLite schema, table or crate was added. The journal is retained as canonical
state and must accompany project backups.

Thread mapping: every `thread` command clearly refuses and names task/launch,
result verification/integration and finalization replacements. This preserves
sealed contracts and attempts rather than silently editing executing briefs.
Coordinator instructions retain propose/auto approval and keep/auto cleanup
semantics. Signing uses `--sign-with` or `[coordinator].signing_key`.

Files: `src/canonical_coordinator.rs`, coordinator/CLI wiring, native validation
helpers, runtime/store route and result-review APIs, signing ingress, maintenance
classification; E2E tests in `canonical_coordinator`, `operator_launch` and `cli`;
coordinator skill, canonical-worker-launch, operator-runbook, getting-started,
migration-workflow and telemetry contracts documentation.

## Commands and results

```sh
cargo check --locked --offline -j 3
cargo check --locked --offline -j 3
cargo clippy --locked --offline -j 3 --all-targets
cargo test --locked --offline -j 3 --test canonical_coordinator --test operator_launch --test canonical_worker --test cli --no-fail-fast
cargo test --locked --offline -j 3 --test canonical_coordinator
```

Both build configurations compile. Clippy exits successfully; existing repository
warnings remain, with none in changed lines. `git diff --check` passes. Only the
new Rust files were formatted; no repository-wide formatting was run.

| Suite | Passed | Socket-blocked | Ignored |
| --- | ---: | ---: | ---: |
| canonical_coordinator | 1 | 1 | 0 |
| canonical_worker | 5 | 31 | 0 |
| cli | 60 | 24 | 1 |
| operator_launch | 7 | 4 | 0 |
| Total | 73 | 60 | 1 |

The context/thread workflow and configured-signing reservation/replay workflow
pass. The full coordinator lifecycle (including swallowed priming, closed-pane
recreation and lost-acknowledgement recovery) stops at the real Unix socket fixture.
The extended task-add/launch-to-Running workflow likewise needs the steward's
socket-capable environment. No sandbox workaround was attempted. The socket
failures below are `Operation not permitted`, or fixture startup timeouts/native
probe exits caused by their Python Herdr server failing to bind. Later workflow
assertions were not reached, so these counts do not establish their success.

An earlier CLI run hit the existing migration-plan race in
`effect_commands_read_only_their_rows_with_ten_thousand_retired_neighbors`
(`source or mapping changed since plan`); it passed in the final full run.
There are no remaining non-socket failures in that run.

## Socket-only failures for the steward

Run these outside the hard sandbox with the steward's nextest configuration.
All lab CLI/ticker processes use `tests/support/time-scale.txt`.

### canonical_coordinator

- `socket_open_primes_owned_coordinator_retries_swallowed_prompt_and_recreates_closed_pane`

### canonical_worker

- `a_brief_swallowed_by_the_agent_is_redelivered_and_confirmed_only_once_accepted`
- `a_brief_the_agent_never_accepts_is_left_ambiguous_not_confirmed`
- `a_hidden_path_covering_the_execution_home_refuses_the_launch_before_creation`
- `a_launch_reaches_running_while_another_holder_takes_the_shared_root_intermittently`
- `a_legacy_thread_holding_the_planned_worktree_blocks_its_creation`
- `a_proven_worker_end_keeps_the_project_admitted_but_an_unexplained_pane_loss_pauses_it`
- `a_sandboxed_reviewer_uses_its_worker_channel_through_the_spool`
- `a_subdirectory_binding_runs_in_the_same_subdirectory_of_the_new_worktree`
- `a_worker_branch_reaching_a_corrupt_quarantined_object_is_refused`
- `accepted_editing_worker_completes_automatically_after_integration`
- `accepted_verify_only_editing_worker_completes_without_integration_automation`
- `an_isolated_codex_worker_commits_through_codex_workspace_write_sandbox`
- `an_isolated_worker_cannot_read_owner_secrets_or_lift_the_hiding_but_still_commits_and_submits`
- `an_isolated_worker_submits_only_through_its_own_spool`
- `an_operator_finishes_a_worker_that_never_submitted_and_the_result_lands_automatically`
- `an_untracked_working_directory_is_refused_before_the_approval_is_used`
- `canonical_attempt_sidebar_clears_after_termination_in_an_active_project`
- `canonical_attempt_sidebar_does_not_publish_to_a_replaced_terminal`
- `canonical_attempt_sidebar_refreshes_and_clears_on_pause_and_termination`
- `canonical_attempt_sidebar_restart_offers_no_historical_cleanup_or_native_request`
- `canonical_attempt_sidebar_uses_collected_usage_and_observed_waiting`
- `editing_worker_requires_operator_completion_when_automation_is_off`
- `launch_sets_the_intended_permission_mode_over_a_stale_one_in_the_home`
- `rejected_editing_worker_stays_running_and_can_resubmit`
- `review_assignment_launches_with_blind_brief_and_records_session`
- `ticker_does_not_dispatch_a_launch_cancelled_before_creation`
- `ticker_launches_and_briefs_once_then_stops_a_cancelled_worker_while_paused_and_revoked`
- `ticker_launches_nothing_on_a_server_without_the_launch_contract_or_while_paused`
- `ticker_recovers_a_lost_creation_reply_without_creating_again`
- `ticker_retires_a_cancelled_gated_worker_without_starting_it`
- `ticker_stops_the_dedicated_herdr_server_of_a_finished_task`

### cli

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

### operator_launch

- `launch_run_retries_after_termination_but_refuses_an_unobserved_live_worker`
- `launch_run_with_a_dedicated_server_after_verify_interaction_reserves_both_kinds`
- `verify_interaction_produces_launchable_evidence_for_codex_and_claude_from_the_cli`
- `verify_interaction_tolerates_agents_writing_into_their_execution_home`

The `state-store` feature is enabled by default; build recipes above use that default.
