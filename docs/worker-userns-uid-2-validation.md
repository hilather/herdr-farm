# WORKER-USERNS-UID-2 validation

2026-10-08, branch `factory/worker-userns-uid-2`.

## Design and deployment

Choose (b): product-owned `--no-daemon` for every Codex interactive worker and
native probe. Herdr observes the TUI through native readiness/prompt APIs and
collects rollout files; it does not use managed daemon control files or require
its cross-session lifecycle. Each TUI starts its own app server, avoiding
uid-keyed control-link collisions in a shared execution home and private
`/tmp`. Existing control links are untouched, including links belonging to
concurrent sessions.

The effective argument digest includes this flag. Preparation, retained-evidence
derivation, frozen-definition checks, native probing and canonical launch all
derive it consistently. Refresh Codex profile preparation, native interaction
verification and launch approval before deployment. Re-enabling the live
project's owner mapping remains the owner's deployment decision.

Native probing checks the retained namespace-init pidfd before initial agent observation
and during prompt readiness, even while the host wrapper is still draining. Proven early termination produces an explicit
early-exit message and a bounded terminal tail (including stderr and a terminal
exit status when available). It does not turn procfs access failure into evidence
of death. Raw terminal content is not retained in telemetry.

New coverage goes through the compiled CLI with local fake agent/server fixtures:
root → owner → root on one home with a legacy socket link; and immediate agent
exit with `File exists (os error 17)`. Started records assert the effective uid
and launch counts. Existing process-title fixtures preserve the new fixed flag
across their setup-wrapper exec.

Changed production files: `src/profile_config.rs`,
`src/profile_preparation.rs`, `src/profile_preparation/native.rs`,
`src/profile_preparation/revalidation.rs`,
`src/canonical_worker/resources.rs` and `src/worker_supervision/observation.rs`. Coverage is in `tests/profiles.rs` and
`tests/fixtures/probe_readiness_{agent.c,server.py}`. Operator documentation is
updated in `docs/profiles.md` and `docs/operations.md`.
No schema changes, migrations, dependencies, or new process spawns.

## Checks

All Cargo commands used `TMPDIR=$PWD/target/tmp` and
`nice -n 19 ionice -c 3 cargo ... --locked --offline -j 3 --features state-store`.
CLI labs use the shared test time scale. No live server, ticker or agent CLI was
run; no owner agent-data directories were consulted.

| Suite | Passed | Failed | Ignored |
| --- | ---: | ---: | ---: |
| canonical_worker library | 2 | 67 | 10 |
| canonical_worker CLI | 10 | 49 | 0 |
| profiles CLI | 7 | 9 | 0 |
| profile library | 23 | 15 | 3 |
| worker_supervision library | 10 | 0 | 0 |

The combined CLI command stopped after canonical_worker failed; profiles was
run explicitly afterward. The profiles CLI suite was rerun after improving the
new fixtures' socket-failure diagnostics and after tightening exit detection to
the init pidfd. Worker supervision was rerun after that change; the table gives
the final runs.

All 140 failures stopped at sandbox-denied socket setup, before the behavior
under test: 137 Unix socket failures and three TCP listener failures. The
canonical CLI's Unix socket failures surface as waiting for the missing fixture
socket; its stderr contains the Python bind `Operation not permitted` traceback.
The three TCP failures are at the existing `HeldCheck` fixture's
`TcpListener::bind`. No workaround or coverage deletion was attempted.

The new root/owner/root and early-exit E2Es compiled but could not execute their
launch assertions here. They require the steward's run outside this sandbox.
The C fixture compiled and the Python fixture syntax checked successfully.
`git diff --check` passed. The final
`cargo clippy --locked --offline -j 3 --features state-store --all-targets`
completed successfully. Existing warnings remain in untouched code; matching
diagnostic locations against the changed-line ranges found no warnings in
changed lines. One new collapsible-if warning was fixed before the final run.

The copy_jobs suite was not requested or run. Its three known checkout-backed
TMPDIR environmental cases were left unchanged:
`artifact_transfer_scopes_nested_thread_dirs_to_one_common_git_dir`,
`final_worker_recovers_without_sender_and_finishes_copy_after_eligibility_loss`,
and `retained_recovery_never_refetches_and_config_withdrawal_preserves_intent`.

## Socket-only failure list

### canonical_worker library

- `canonical_worker::tests::a_starting_agent_whose_own_display_state_moves_is_named_once_but_identity_drift_is_refused` (Unix)
- `canonical_worker::tests::barrier_revocation_during_prompt_preserves_stale_delivery_and_stop_recovery` (Unix)
- `canonical_worker::tests::brief_preparation_and_delivery_do_not_decode_unrelated_approval_history` (Unix)
- `canonical_worker::tests::brief_preparation_expiry_and_failed_commit_leave_no_delivery_obligation` (Unix)
- `canonical_worker::tests::brief_rendering_accounts_selected_inputs_before_decode_or_claim` (Unix)
- `canonical_worker::tests::brief_rendering_shares_deadline_and_does_not_scan_retained_history` (Unix)
- `canonical_worker::tests::busy_foreign_and_changed_executable_workers_are_not_claimed` (Unix)
- `canonical_worker::tests::conflicting_legacy_socket_alias_prevents_brief_claim` (Unix)
- `canonical_worker::tests::controller_recovers_missing_initial_brief_without_sending_or_duplicating_it` (Unix)
- `canonical_worker::tests::corrupt_target_payload_cannot_hide_a_retained_resource_using_a_terminated_peer` (Unix)
- `canonical_worker::tests::creation_and_recovery_refuse_terminal_replacement_during_process_observation` (Unix)
- `canonical_worker::tests::creation_claim_and_recovery_intent_roll_back_together_before_native_effects` (Unix)
- `canonical_worker::tests::creation_retains_observed_supervisor_after_authority_changes` (Unix)
- `canonical_worker::tests::direct_workers_require_positive_visible_readiness_not_managed_launch_flag` (Unix)
- `canonical_worker::tests::exit_before_any_identity_observation_keeps_uncertain_creation_reserved` (Unix)
- `canonical_worker::tests::exited_worker_reconciles_after_handles_are_lost_without_claiming_task_success` (Unix)
- `canonical_worker::tests::gate_release_boundary_is_one_use_and_does_not_confirm_a_start` (Unix)
- `canonical_worker::tests::gate_release_refuses_changed_target_authority_and_failed_commit_is_atomic` (Unix)
- `canonical_worker::tests::gate_selection_claim_and_render_share_bounded_reads` (Unix)
- `canonical_worker::tests::last_moment_brief_preflight_blocks_changed_authority_without_submission` (Unix)
- `canonical_worker::tests::launch_advancement_recovers_each_boundary_then_delivers_brief_and_stops` (Unix)
- `canonical_worker::tests::launch_advancement_refuses_unusable_environment_before_creation` (Unix)
- `canonical_worker::tests::launch_advancement_selection_is_bounded_and_operation_scoped` (Unix)
- `canonical_worker::tests::launch_reconciliation_ignores_unrelated_history_without_releasing_a_gate` (Unix)
- `canonical_worker::tests::live_gate_proof_refuses_the_same_child_after_exec` (Unix)
- `canonical_worker::tests::lost_creation_recovers_exact_resource_without_config_or_another_claim` (Unix)
- `canonical_worker::tests::lost_or_foreign_brief_acknowledgments_retain_claim_and_never_repeat` (Unix)
- `canonical_worker::tests::lost_workspace_reply_recovers_exact_live_marker_even_after_revocation_and_expiry` (Unix)
- `canonical_worker::tests::malformed_neighbor_identity_never_panics_claims_or_sends_a_brief` (Unix)
- `canonical_worker::tests::native_brief_original_deadline_bounds_a_stalled_submission` (Unix)
- `canonical_worker::tests::native_gate_refusal_does_not_consume_the_release_opportunity_or_send_input` (Unix)
- `canonical_worker::tests::native_gate_submission_is_once_even_when_the_reply_is_lost` (Unix)
- `canonical_worker::tests::native_start_confirmation_requires_real_exec_and_exact_agent_then_replays_read_only` (Unix)
- `canonical_worker::tests::new_workspace_creation_and_lost_layout_reply_are_one_use` (Unix)
- `canonical_worker::tests::observed_resource_exit_before_target_commit_remains_recoverable` (Unix)
- `canonical_worker::tests::pane_conflicts_remain_effect_fences_with_ten_thousand_retired_neighbors` (Unix)
- `canonical_worker::tests::prepared_launch_selection_rotates_stages_and_excludes_cancelled_or_expired_effects` (Unix)
- `canonical_worker::tests::readiness_lost_after_claim_prevents_prompt_and_retains_one_use_claim` (Unix)
- `canonical_worker::tests::recovery_target_commit_failure_retains_original_claim_and_can_be_reobserved` (Unix)
- `canonical_worker::tests::resource_creation_ignores_cold_approval_history_without_replaying` (Unix)
- `canonical_worker::tests::resource_creation_selection_and_render_are_history_bounded` (Unix)
- `canonical_worker::tests::resource_preparation_rejects_changed_config_before_consuming_approval_or_creating` (Unix)
- `canonical_worker::tests::resource_recovery_selects_its_evidence_without_scanning_history` (Unix)
- `canonical_worker::tests::retained_pane_projection_excludes_reused_history_and_tracks_recovery_boundaries` (Unix)
- `canonical_worker::tests::retained_pane_projection_tracks_source_mutations_and_preserves_workspaces` (Unix)
- `canonical_worker::tests::revoked_barrier_routes_a_real_supervised_stop_and_recovers_after_commit_failure` (Unix)
- `canonical_worker::tests::staged_pane_selection_ignores_other_panes_and_preserves_provenance_fences` (Unix)
- `canonical_worker::tests::staged_pane_selection_reads_the_real_schema42_without_new_indexes` (Unix)
- `canonical_worker::tests::staged_repository_stop_requires_and_records_preserved_partial_files` (Unix)
- `canonical_worker::tests::staged_stop_commit_failure_retains_capacity_and_recovers_without_creation` (Unix)
- `canonical_worker::tests::staged_stop_requires_output_evidence_and_records_an_empty_directory_as_absent` (Unix)
- `canonical_worker::tests::start_selection_is_bounded_and_cancellable_with_retained_history` (Unix)
- `canonical_worker::tests::stock_herdr_launch_execs_the_launcher_in_place_of_the_shell` (Unix)
- `canonical_worker::tests::stock_herdr_unconfirmed_launch_closes_its_workspace_and_counts_nothing` (Unix)
- `canonical_worker::tests::stop_before_brief_atomically_retires_send_and_commit_failure_keeps_capacity` (Unix)
- `canonical_worker::tests::supervised_root_creation_and_commit_losses_recover_without_bootstrap_or_replay` (Unix)
- `canonical_worker::tests::supervised_root_recovery_needs_exact_process_and_preserves_expired_authority` (Unix)
- `canonical_worker::tests::target_inventory_rejects_rehashed_inputs_and_broken_launch_operation_links` (Unix)
- `canonical_worker::tests::target_retention_uses_selected_history_and_original_control` (Unix)
- `canonical_worker::tests::termination_does_not_decode_unrelated_approval_history` (Unix)
- `canonical_worker::tests::termination_selection_is_bounded_and_cancellable_with_retained_history` (Unix)
- `canonical_worker::tests::uncertain_creation_refuses_foreign_socket_duplicate_panes_and_changed_command` (Unix)
- `canonical_worker::tests::uncertain_initial_brief_never_becomes_a_fresh_preparation_hint` (Unix)
- `canonical_worker::tests::workers_sharing_one_herdr_server_are_named_independently_and_recover_unapplied_names` (Unix)
- `canonical_worker::tests::workspace_acknowledgment_loss_retains_uncertainty_without_layout_or_recreation` (Unix)
- `canonical_worker::tests::workspace_layout_commit_failure_resumes_without_recreating_workspace` (Unix)
- `canonical_worker::tests::workspace_receipt_commit_failure_never_submits_layout_or_recreates` (Unix)

### canonical_worker CLI

- `a_brief_swallowed_by_the_agent_is_redelivered_and_confirmed_only_once_accepted` (Unix)
- `a_brief_the_agent_never_accepts_is_left_ambiguous_not_confirmed` (Unix)
- `a_hidden_path_covering_the_execution_home_refuses_the_launch_before_creation` (Unix)
- `a_launch_outlasts_an_operator_holding_the_root_after_its_workspace_creation` (Unix)
- `a_launch_reaches_running_while_another_holder_takes_the_shared_root_intermittently` (Unix)
- `a_legacy_thread_holding_the_planned_worktree_blocks_its_creation` (Unix)
- `a_proven_worker_end_keeps_the_project_admitted_but_an_unexplained_pane_loss_pauses_it` (Unix)
- `a_sandboxed_reviewer_uses_its_worker_channel_through_the_spool` (Unix)
- `a_subdirectory_binding_runs_in_the_same_subdirectory_of_the_new_worktree` (Unix)
- `a_worker_branch_reaching_a_corrupt_quarantined_object_is_refused` (Unix)
- `a_worker_that_dies_without_submitting_is_noticed_and_preserved_while_another_attempts_check_runs` (TCP)
- `a_worker_that_exits_after_submitting_is_recorded_terminated_while_another_attempts_check_runs` (TCP)
- `a_worker_that_exits_during_its_own_verification_ends_after_the_verdict_without_contention` (TCP)
- `accepted_editing_worker_completes_automatically_after_integration` (Unix)
- `accepted_verify_only_editing_worker_completes_without_integration_automation` (Unix)
- `an_automatic_pause_names_its_blockers_and_lifts_itself_once_they_clear` (Unix)
- `an_isolated_codex_worker_commits_through_codex_workspace_write_sandbox` (Unix)
- `an_isolated_worker_cannot_read_owner_secrets_or_lift_the_hiding_but_still_commits_and_submits` (Unix)
- `an_isolated_worker_submits_only_through_its_own_spool` (Unix)
- `an_operator_finishes_a_worker_that_never_submitted_and_the_result_lands_automatically` (Unix)
- `an_untracked_working_directory_is_refused_before_the_approval_is_used` (Unix)
- `attention_mid_run_failure_remains_incomplete_at_termination` (Unix)
- `barrier_stop_gets_a_fair_turn_and_maintenance_keeps_advancing` (Unix)
- `canonical_attempt_sidebar_clears_after_termination_in_an_active_project` (Unix)
- `canonical_attempt_sidebar_does_not_publish_to_a_replaced_terminal` (Unix)
- `canonical_attempt_sidebar_refreshes_and_clears_on_pause_and_termination` (Unix)
- `canonical_attempt_sidebar_restart_offers_no_historical_cleanup_or_native_request` (Unix)
- `canonical_attempt_sidebar_uses_collected_usage_and_observed_waiting` (Unix)
- `concurrent_attempt_tokens_publish_and_missing_retired_server_stays_quiet` (Unix)
- `dedicated_worker_refuses_remote_manifest_before_its_brief` (Unix)
- `editing_worker_requires_operator_completion_when_automation_is_off` (Unix)
- `idle_worker_notice_restarts_stretch_and_deduplicates_within_a_ticker` (Unix)
- `launch_run_reviews_use_the_claude_spool_and_record_skeptical_yield` (Unix)
- `launch_run_reviews_use_the_codex_spool_and_record_skeptical_yield` (Unix)
- `launch_sets_the_intended_permission_mode_over_a_stale_one_in_the_home` (Unix)
- `rejected_editing_worker_stays_running_and_can_resubmit` (Unix)
- `review_assignment_launches_with_blind_brief_and_records_session` (Unix)
- `submit_captured_retains_remember_from_the_attempt_report_and_replays_once` (Unix)
- `ticker_does_not_dispatch_a_launch_cancelled_before_creation` (Unix)
- `ticker_launches_and_briefs_once_then_stops_a_cancelled_worker_while_paused_and_revoked` (Unix)
- `ticker_launches_nothing_on_a_server_without_the_launch_contract_or_while_paused` (Unix)
- `ticker_recovers_a_lost_creation_reply_without_creating_again` (Unix)
- `ticker_retires_a_cancelled_gated_worker_without_starting_it` (Unix)
- `ticker_stops_the_dedicated_herdr_server_of_a_finished_task` (Unix)
- `wall_budget_termination_retries_contention_and_notifies_once` (Unix)
- `wall_budget_termination_with_changed_frozen_definition_keeps_process_exit` (Unix)
- `wall_budget_termination_without_contention` (Unix)
- `worker_uid_default_root_and_verification_stay_frozen` (Unix)
- `worker_uid_owner_and_verification_stay_frozen_with_mounts_enforced` (Unix)

### profiles CLI

- `codex_shared_home_survives_root_owner_root_with_a_stale_daemon_link` (Unix)
- `native_probe_reports_early_agent_exit_and_terminal_reason` (Unix)
- `native_readiness_accepts_cold_start_beyond_thirty_seconds` (Unix)
- `native_readiness_rejects_changed_arguments` (Unix)
- `native_readiness_rejects_changed_executable` (Unix)
- `native_readiness_retries_empty_process_title` (Unix)
- `native_readiness_retries_non_terminated_process_title` (Unix)
- `native_readiness_timeout_retains_categories_visible_in_cli` (Unix)
- `native_readiness_waits_for_setup_wrapper_exec` (Unix)

### profile library

- `profile_preparation::tests::launch_ingress::canonical_worktree_paths_and_aliases_block_before_approval_consumption` (Unix)
- `profile_preparation::tests::launch_ingress::draft_signature_and_reservation_preserve_the_exact_brief_and_one_use_boundary` (Unix)
- `profile_preparation::tests::launch_ingress::lost_worktree_receipt_commit_recovers_after_approval_revocation_without_git_add` (Unix)
- `profile_preparation::tests::launch_ingress::preparation_capture_releases_sqlite_and_rechecks_selected_state` (Unix)
- `profile_preparation::tests::launch_ingress::preparation_stop_preserves_created_repositories_and_records_uncreated_plans_in_order` (Unix)
- `profile_preparation::tests::launch_ingress::repository_observation_ignores_replacement_refs_and_refuses_lazy_fetch` (Unix)
- `profile_preparation::tests::launch_ingress::signed_worktree_creation_retains_exact_checkout_and_recovers_without_replay` (Unix)
- `profile_preparation::tests::launch_ingress::started_worktree_proof_allows_output_but_rejects_reassociation` (Unix)
- `profile_preparation::tests::launch_ingress::unsigned_or_changed_launch_inputs_never_create_a_reservation` (Unix)
- `profile_preparation::tests::launch_ingress::worktree_creation_supports_multiple_repositories_binary_files_and_symlinks` (Unix)
- `profile_preparation::tests::launch_ingress::worktree_inventory_retains_uncertain_paths_and_refuses_corrupt_provenance` (Unix)
- `profile_preparation::tests::launch_ingress::worktree_only_stop_fences_late_launch_and_preserves_uncertain_resources` (Unix)
- `profile_preparation::tests::launch_ingress::worktree_preparation_uses_selected_history_through_receipt_commit` (Unix)
- `profile_preparation::tests::launch_ingress::worktree_refusals_precede_approval_consumption_and_effects` (Unix)
- `profile_preparation::tests::launch_ingress::worktree_verification_and_stop_select_only_their_launch_provenance` (Unix)
