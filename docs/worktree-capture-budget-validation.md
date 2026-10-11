# Worktree capture budget validation

WORKTREE-CAPTURE-BUDGET-1, 2026-10-11. No schema changes or new crates.

Tracked byte omissions preserve raw Git blob identity, size and mode in manifest v5. Untracked input retains the default 50 MiB fail-hard capture policy. Only typed capture limits after proven native termination release capacity with a deduplicated coordinator notice; originals remain on disk. Scoped submission reads avoid the whole-project snapshot.

Commands used `TMPDIR=$PWD/target/tmp`, `nice -n 19 ionice -c 3`, `--locked --offline -j 3`, `--features state-store`, and three test threads. No owner services or live agent CLIs were used.

The existing staged-stop over-limit test deliberately now expects native termination with `capture_incomplete`; its explicit capture refusal and unsafe/forged evidence assertions remain. New coverage uses CLI workflows for >50 MiB tracked edits/submission/natural exit, oversized untracked source refusal, and automatic paused-attempt recovery.

## Required suites

- `--lib worktree_preservation`: 6 passed.
- `--lib canonical_worker`: 2 passed, 67 socket-only failures, 10 ignored.
- `--test canonical_worker`: 12 passed, 54 socket-only failures, 6 setup failures listed below.

## Canonical worker socket-only failures

Socket fixtures cannot bind AF_UNIX in this hard sandbox (`Operation not permitted`). E2E fixture startup consequently times out at `Lab::serve`. No workaround was applied. These tests need an outside-sandbox run.

### Focused library

- `canonical_worker::tests::a_starting_agent_whose_own_display_state_moves_is_named_once_but_identity_drift_is_refused`
- `canonical_worker::tests::brief_preparation_and_delivery_do_not_decode_unrelated_approval_history`
- `canonical_worker::tests::barrier_revocation_during_prompt_preserves_stale_delivery_and_stop_recovery`
- `canonical_worker::tests::brief_preparation_expiry_and_failed_commit_leave_no_delivery_obligation`
- `canonical_worker::tests::brief_rendering_accounts_selected_inputs_before_decode_or_claim`
- `canonical_worker::tests::brief_rendering_shares_deadline_and_does_not_scan_retained_history`
- `canonical_worker::tests::busy_foreign_and_changed_executable_workers_are_not_claimed`
- `canonical_worker::tests::conflicting_legacy_socket_alias_prevents_brief_claim`
- `canonical_worker::tests::controller_recovers_missing_initial_brief_without_sending_or_duplicating_it`
- `canonical_worker::tests::corrupt_target_payload_cannot_hide_a_retained_resource_using_a_terminated_peer`
- `canonical_worker::tests::creation_claim_and_recovery_intent_roll_back_together_before_native_effects`
- `canonical_worker::tests::creation_and_recovery_refuse_terminal_replacement_during_process_observation`
- `canonical_worker::tests::creation_retains_observed_supervisor_after_authority_changes`
- `canonical_worker::tests::direct_workers_require_positive_visible_readiness_not_managed_launch_flag`
- `canonical_worker::tests::exit_before_any_identity_observation_keeps_uncertain_creation_reserved`
- `canonical_worker::tests::exited_worker_reconciles_after_handles_are_lost_without_claiming_task_success`
- `canonical_worker::tests::gate_release_boundary_is_one_use_and_does_not_confirm_a_start`
- `canonical_worker::tests::gate_release_refuses_changed_target_authority_and_failed_commit_is_atomic`
- `canonical_worker::tests::gate_selection_claim_and_render_share_bounded_reads`
- `canonical_worker::tests::last_moment_brief_preflight_blocks_changed_authority_without_submission`
- `canonical_worker::tests::launch_advancement_recovers_each_boundary_then_delivers_brief_and_stops`
- `canonical_worker::tests::launch_advancement_refuses_unusable_environment_before_creation`
- `canonical_worker::tests::launch_reconciliation_ignores_unrelated_history_without_releasing_a_gate`
- `canonical_worker::tests::launch_advancement_selection_is_bounded_and_operation_scoped`
- `canonical_worker::tests::live_gate_proof_refuses_the_same_child_after_exec`
- `canonical_worker::tests::lost_creation_recovers_exact_resource_without_config_or_another_claim`
- `canonical_worker::tests::lost_or_foreign_brief_acknowledgments_retain_claim_and_never_repeat`
- `canonical_worker::tests::lost_workspace_reply_recovers_exact_live_marker_even_after_revocation_and_expiry`
- `canonical_worker::tests::malformed_neighbor_identity_never_panics_claims_or_sends_a_brief`
- `canonical_worker::tests::native_brief_original_deadline_bounds_a_stalled_submission`
- `canonical_worker::tests::native_gate_submission_is_once_even_when_the_reply_is_lost`
- `canonical_worker::tests::native_gate_refusal_does_not_consume_the_release_opportunity_or_send_input`
- `canonical_worker::tests::native_start_confirmation_requires_real_exec_and_exact_agent_then_replays_read_only`
- `canonical_worker::tests::observed_resource_exit_before_target_commit_remains_recoverable`
- `canonical_worker::tests::new_workspace_creation_and_lost_layout_reply_are_one_use`
- `canonical_worker::tests::pane_conflicts_remain_effect_fences_with_ten_thousand_retired_neighbors`
- `canonical_worker::tests::prepared_launch_selection_rotates_stages_and_excludes_cancelled_or_expired_effects`
- `canonical_worker::tests::readiness_lost_after_claim_prevents_prompt_and_retains_one_use_claim`
- `canonical_worker::tests::recovery_target_commit_failure_retains_original_claim_and_can_be_reobserved`
- `canonical_worker::tests::resource_creation_ignores_cold_approval_history_without_replaying`
- `canonical_worker::tests::resource_creation_selection_and_render_are_history_bounded`
- `canonical_worker::tests::resource_preparation_rejects_changed_config_before_consuming_approval_or_creating`
- `canonical_worker::tests::resource_recovery_selects_its_evidence_without_scanning_history`
- `canonical_worker::tests::retained_pane_projection_excludes_reused_history_and_tracks_recovery_boundaries`
- `canonical_worker::tests::retained_pane_projection_tracks_source_mutations_and_preserves_workspaces`
- `canonical_worker::tests::revoked_barrier_routes_a_real_supervised_stop_and_recovers_after_commit_failure`
- `canonical_worker::tests::staged_pane_selection_ignores_other_panes_and_preserves_provenance_fences`
- `canonical_worker::tests::staged_pane_selection_reads_the_real_schema42_without_new_indexes`
- `canonical_worker::tests::staged_repository_stop_requires_and_records_preserved_partial_files`
- `canonical_worker::tests::staged_stop_commit_failure_retains_capacity_and_recovers_without_creation`
- `canonical_worker::tests::staged_stop_requires_output_evidence_and_records_an_empty_directory_as_absent`
- `canonical_worker::tests::start_selection_is_bounded_and_cancellable_with_retained_history`
- `canonical_worker::tests::stock_herdr_launch_execs_the_launcher_in_place_of_the_shell`
- `canonical_worker::tests::stock_herdr_unconfirmed_launch_closes_its_workspace_and_counts_nothing`
- `canonical_worker::tests::stop_before_brief_atomically_retires_send_and_commit_failure_keeps_capacity`
- `canonical_worker::tests::supervised_root_creation_and_commit_losses_recover_without_bootstrap_or_replay`
- `canonical_worker::tests::supervised_root_recovery_needs_exact_process_and_preserves_expired_authority`
- `canonical_worker::tests::target_inventory_rejects_rehashed_inputs_and_broken_launch_operation_links`
- `canonical_worker::tests::target_retention_uses_selected_history_and_original_control`
- `canonical_worker::tests::termination_does_not_decode_unrelated_approval_history`
- `canonical_worker::tests::termination_selection_is_bounded_and_cancellable_with_retained_history`
- `canonical_worker::tests::uncertain_creation_refuses_foreign_socket_duplicate_panes_and_changed_command`
- `canonical_worker::tests::uncertain_initial_brief_never_becomes_a_fresh_preparation_hint`
- `canonical_worker::tests::workers_sharing_one_herdr_server_are_named_independently_and_recover_unapplied_names`
- `canonical_worker::tests::workspace_acknowledgment_loss_retains_uncertainty_without_layout_or_recreation`
- `canonical_worker::tests::workspace_layout_commit_failure_resumes_without_recreating_workspace`
- `canonical_worker::tests::workspace_receipt_commit_failure_never_submits_layout_or_recreates`

### E2E

- `a_hidden_path_covering_the_execution_home_refuses_the_launch_before_creation`
- `a_brief_the_agent_never_accepts_is_left_ambiguous_not_confirmed`
- `a_launch_outlasts_an_operator_holding_the_root_after_its_workspace_creation`
- `a_legacy_thread_holding_the_planned_worktree_blocks_its_creation`
- `a_launch_reaches_running_while_another_holder_takes_the_shared_root_intermittently`
- `a_proven_worker_end_keeps_the_project_admitted_but_an_unexplained_pane_loss_pauses_it`
- `a_subdirectory_binding_runs_in_the_same_subdirectory_of_the_new_worktree`
- `a_worker_that_dies_without_submitting_is_noticed_and_preserved_while_another_attempts_check_runs`
- `a_worker_that_exits_after_submitting_is_recorded_terminated_while_another_attempts_check_runs`
- `a_worker_that_exits_during_its_own_verification_ends_after_the_verdict_without_contention`
- `a_sandboxed_reviewer_uses_its_worker_channel_through_the_spool`
- `a_worker_branch_reaching_a_corrupt_quarantined_object_is_refused`
- `accepted_editing_worker_completes_automatically_after_integration`
- `accepted_verify_only_editing_worker_completes_without_integration_automation`
- `ambiguous_creation_with_removed_dedicated_server_releases_capacity_once`
- `an_automatic_pause_names_its_blockers_and_lifts_itself_once_they_clear`
- `an_isolated_codex_worker_commits_through_codex_workspace_write_sandbox`
- `an_isolated_worker_cannot_read_owner_secrets_or_lift_the_hiding_but_still_commits_and_submits`
- `an_untracked_working_directory_is_refused_before_the_approval_is_used`
- `an_operator_finishes_a_worker_that_never_submitted_and_the_result_lands_automatically`
- `attention_mid_run_failure_remains_incomplete_at_termination`
- `canonical_attempt_sidebar_clears_after_termination_in_an_active_project`
- `canonical_attempt_sidebar_refreshes_and_clears_on_pause_and_termination`
- `canonical_attempt_sidebar_restart_offers_no_historical_cleanup_or_native_request`
- `canonical_attempt_sidebar_uses_collected_usage_and_observed_waiting`
- `dedicated_worker_refuses_remote_manifest_before_its_brief`
- `concurrent_attempt_tokens_publish_and_missing_retired_server_stays_quiet`
- `editing_worker_requires_operator_completion_when_automation_is_off`
- `idle_worker_notice_restarts_stretch_and_deduplicates_within_a_ticker`
- `launch_run_reviews_use_the_claude_spool_and_record_skeptical_yield`
- `launch_run_reviews_use_the_codex_spool_and_record_skeptical_yield`
- `launch_sets_the_intended_permission_mode_over_a_stale_one_in_the_home`
- `persistent_launch_conflict_closes_attempt_and_notifies_once`
- `proven_launch_server_loss_retires_owned_runtime_and_next_brief_succeeds`
- `rejected_editing_worker_stays_running_and_can_resubmit`
- `repeated_nontransient_recovery_stops_and_escalates_with_capacity_retained`
- `review_assignment_launches_with_blind_brief_and_records_session`
- `slow_creation_outlives_short_lease_without_ambiguous_delivery`
- `stale_sealed_brief_authority_fails_once_without_prompt_or_retry`
- `submit_captured_retains_remember_from_the_attempt_report_and_replays_once`
- `three_transient_missing_socket_observations_retain_capacity`
- `ticker_does_not_dispatch_a_launch_cancelled_before_creation`
- `ticker_ends_an_expired_unreleased_launch_and_allows_a_new_attempt`
- `ticker_launches_nothing_on_a_server_without_the_launch_contract_or_while_paused`
- `ticker_launches_and_briefs_once_then_stops_a_cancelled_worker_while_paused_and_revoked`
- `ticker_recovers_a_lost_creation_reply_without_creating_again`
- `ticker_stops_the_dedicated_herdr_server_of_a_finished_task`
- `ticker_retires_a_cancelled_gated_worker_without_starting_it`
- `verified_worker_results_wake_idle_coordinator_once_and_defer_while_busy`
- `wall_budget_termination_retries_contention_and_notifies_once`
- `wall_budget_termination_with_changed_frozen_definition_keeps_process_exit`
- `worker_uid_owner_and_verification_stay_frozen_with_mounts_enforced`
- `worker_uid_default_root_and_verification_stay_frozen`
- `wall_budget_termination_without_contention`

## Other canonical E2E setup failures

These failed before socket setup; they are not classified as socket-only. The heavily loaded host produced store deadlines or stale matching observations. The capture behavior was not reached.

- `a_brief_swallowed_by_the_agent_is_redelivered_and_confirmed_only_once_accepted`: store Deadline.
- `an_isolated_worker_submits_only_through_its_own_spool`: store Deadline.
- `barrier_stop_gets_a_fair_turn_and_maintenance_keeps_advancing`: fresh matching observation required.
- `canonical_attempt_sidebar_does_not_publish_to_a_replaced_terminal`: store Deadline.
- `launch_revision_conflict_reselects_without_duplicate_resources_or_brief`: store Deadline.
- `worktree_capture_budgets_preserve_tracked_identity_and_release_untracked_refusals`: fresh matching observation required.

## Full library suite

`--lib`: 525 passed, 88 failed, 13 ignored. Of the failures, 84 are socket-only: the same 67 canonical-worker tests listed above, plus these 17 tests.

- `profile_preparation::tests::launch_ingress::canonical_worktree_paths_and_aliases_block_before_approval_consumption`
- `profile_preparation::tests::launch_ingress::draft_signature_and_reservation_preserve_the_exact_brief_and_one_use_boundary`
- `profile_preparation::tests::launch_ingress::lost_worktree_receipt_commit_recovers_after_approval_revocation_without_git_add`
- `profile_preparation::tests::launch_ingress::preparation_capture_releases_sqlite_and_rechecks_selected_state`
- `profile_preparation::tests::launch_ingress::preparation_stop_preserves_created_repositories_and_records_uncreated_plans_in_order`
- `profile_preparation::tests::launch_ingress::repository_observation_ignores_replacement_refs_and_refuses_lazy_fetch`
- `profile_preparation::tests::launch_ingress::signed_worktree_creation_retains_exact_checkout_and_recovers_without_replay`
- `profile_preparation::tests::launch_ingress::started_worktree_proof_allows_output_but_rejects_reassociation`
- `profile_preparation::tests::launch_ingress::unsigned_or_changed_launch_inputs_never_create_a_reservation`
- `profile_preparation::tests::launch_ingress::worktree_creation_supports_multiple_repositories_binary_files_and_symlinks`
- `profile_preparation::tests::launch_ingress::worktree_inventory_retains_uncertain_paths_and_refuses_corrupt_provenance`
- `profile_preparation::tests::launch_ingress::worktree_only_stop_fences_late_launch_and_preserves_uncertain_resources`
- `profile_preparation::tests::launch_ingress::worktree_preparation_uses_selected_history_through_receipt_commit`
- `profile_preparation::tests::launch_ingress::worktree_refusals_precede_approval_consumption_and_effects`
- `profile_preparation::tests::launch_ingress::worktree_verification_and_stop_select_only_their_launch_provenance`
- `runner::socket::tests::reply_is_bounded_and_requires_a_complete_utf8_line`
- `runner::socket::tests::trickle_reply_cannot_restart_deadline`

Four additional failures are not classified as socket-only. They failed timing, expiry, or deadline assertions under host load; no assertions were relaxed:

- `admission::tests::admission_resumes_after_a_full_page_of_unsigned_candidates`
- `runner::socket::tests::blocked_submission_obeys_same_deadline`
- `runtime::controlled_tests::expiry_sql_is_interrupted_without_undoing_published_observations`
- `store::barriers::tests::ancestor_memory_expiry_during_later_release_rolls_back_publication`

The three named `copy_jobs::tests` cases are binary tests, outside the requested library/canonical suites, and were not run. Their checkout-contained TMPDIR assumption remains unchanged.

## Final verification

After the final failure-boundary and lint fixes, `cargo clippy --locked --offline -j 3 --features state-store --all-targets` completed successfully. Existing diagnostics remain; none intersects changed lines. A fresh `--lib worktree_preservation` run passed all six tests.

The final one-thread `worktree_capture_budgets_preserve_tracked_identity_and_release_untracked_refusals` run compiled and failed only at `Lab::serve`: AF_UNIX `bind` returned `Operation not permitted`. Its initial full-suite stale-observation failure above was not reproduced. Runtime assertions for both branches require the outside-sandbox steward run.

The final one-thread `an_automatic_pause_names_its_blockers_and_lifts_itself_once_they_clear` run likewise compiled and failed only at AF_UNIX fixture binding. No running owner project, ticker, server, or owner agent data was accessed.
