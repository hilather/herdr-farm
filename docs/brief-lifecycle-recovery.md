# BRIEF-LIFECYCLE-26b-3 recovery lock audit

The old recovery path dropped its root guard while the local `Native` value
still retained transferred descriptors. A transferred `LockFile` closes without
explicitly unlocking, so those descriptors kept the same open file description
locked even after all readiness helpers exited. Delivery then opened a new file
description and conflicted with its own process. This does not require a lingering
helper child. The second acquisition was bounded by the staged two-second wait,
not the operator wait; its error therefore lacked `OperatorLockContention`.

## Static acquisition trace

Line references describe this fix revision.

- `src/canonical_worker.rs:892` enters `RootGuard::operator_by`.
  `src/execution_guard.rs:289-290` acquires `.execution-turn.lock`, then
  `.execution.lock`; the turn is released after acquiring the root.
- `src/migration/mod.rs:633` and `:644` validate publication and open SQLite.
  Neither takes an execution guard or a runtime-mutation guard.
  `src/store/worker_brief.rs:173` and `:191` observe submission and authorize
  retry using SQLite transactions, without acquiring the root gate.
- `src/canonical_worker/inventory.rs:49` checks pane aliases using publication
  and read-only SQLite inventory (`src/migration/identity_inventory.rs:82`,
  `:95`). No execution lock is acquired. Executable validation at
  `src/canonical_worker.rs:84` reads and hashes a file without taking a gate.
- `src/canonical_worker.rs:905` inherits the held root descriptor.
  `src/execution_guard.rs:254` calls `LockFile::transfer` (`:86`), which clones
  the existing description and marks the guard transferred. It does not flock
  a new description. Native readiness (`src/canonical_worker.rs:265`) calls
  the supervisor (`src/supervision.rs:33`); the runner duplicates inherited
  descriptors (`src/runner.rs:276`) and spawns through `GatedSpawn` (`:300`).
  The spawn gate is a separate in-process synchronization gate. Native fencing
  (`src/canonical_worker.rs:152`) reconnects pidfds and namespace identity
  (`src/worker_supervision/observation.rs:407`) without acquiring root locks.
- Previously, delivery reacquired the root via `exclusive_by`; that method
  takes the writer turn and root at `src/execution_guard.rs:280-281`.
  `exclusive_file_until` opens a new description at `:117`, tries to flock at
  `:119`, and emits the exact `another operation owns lock ...; retry` message
  at `:122`. This is the conflicting acquisition, not a migration lock.
- Now `src/canonical_worker.rs:909` passes the held guard into delivery.
  `deliver_brief_once` (`:389`) inherits it instead of acquiring another gate.
  Revision-conflict retries and failure recording also retain the same guard.
  Ordinary ticker delivery still uses its existing staged root acquisition.

## Coverage limits

All existing recovery assertions remain. The socket-free CLI contention E2E
exercises missing-operation recovery, timeout diagnostics, unchanged persisted
state, and the same request reaching the store after release. A full recovery
retry cannot be socket-free: native fencing requires an actual Unix socket's
retained inode/device/birth identity as well as a live supervisor incarnation.
The sandbox forbids socket bind. No bypass, unit test, source-text assertion,
or new public testing API was added; the two existing full recovery E2Es must
also run outside the sandbox.

The fix changes no schema, dependencies, process spawn sites, or retention rules.

## Sandbox validation

All cargo commands used `TMPDIR=$PWD/target/tmp`, `nice -n 19 ionice -c 3`,
`--locked --offline -j 3`, and `--features state-store`.

- `cargo test --lib`: 529 passed, 84 socket-only failures, 13 ignored.
- `cargo test --test canonical_worker brief_recovery`: the socket-free
  `brief_recovery_lock_budget_exhaustion_is_actionable_and_preserves_state` passed;
  `ambiguous_brief_recovery_requires_exact_idle_worker_and_sends_only_once`
  failed before recovery when fixture socket bind was denied.
- `cargo test --test canonical_worker ambiguous_idle_brief_can_have_only_one_explicit_recovery_send`:
  failed before recovery when fixture socket bind was denied. Both targeted
  full recovery failures show Python `PermissionError: [Errno 1] Operation not
  permitted`, followed by the fixture startup deadline assertion.
- `cargo clippy --all-targets`: completed successfully with existing warnings;
  no warnings in changed lines. `git diff --check` passed.

No new regression was added because exercising the descriptor transfer and
subsequent recovery send through public entry points requires the socket
incarnation described above. The existing full E2Es retain every assertion.
The `copy_jobs::tests` environmental cases were not run by the requested lib
suite; their code was not changed.

Socket-only lib failures (each reported `Operation not permitted` at socket
fixture setup):

- `canonical_worker::tests::a_starting_agent_whose_own_display_state_moves_is_named_once_but_identity_drift_is_refused`
- `canonical_worker::tests::barrier_revocation_during_prompt_preserves_stale_delivery_and_stop_recovery`
- `canonical_worker::tests::brief_preparation_and_delivery_do_not_decode_unrelated_approval_history`
- `canonical_worker::tests::brief_preparation_expiry_and_failed_commit_leave_no_delivery_obligation`
- `canonical_worker::tests::brief_rendering_accounts_selected_inputs_before_decode_or_claim`
- `canonical_worker::tests::brief_rendering_shares_deadline_and_does_not_scan_retained_history`
- `canonical_worker::tests::busy_foreign_and_changed_executable_workers_are_not_claimed`
- `canonical_worker::tests::conflicting_legacy_socket_alias_prevents_brief_claim`
- `canonical_worker::tests::controller_recovers_missing_initial_brief_without_sending_or_duplicating_it`
- `canonical_worker::tests::corrupt_target_payload_cannot_hide_a_retained_resource_using_a_terminated_peer`
- `canonical_worker::tests::creation_and_recovery_refuse_terminal_replacement_during_process_observation`
- `canonical_worker::tests::creation_claim_and_recovery_intent_roll_back_together_before_native_effects`
- `canonical_worker::tests::creation_retains_observed_supervisor_after_authority_changes`
- `canonical_worker::tests::direct_workers_require_positive_visible_readiness_not_managed_launch_flag`
- `canonical_worker::tests::exit_before_any_identity_observation_keeps_uncertain_creation_reserved`
- `canonical_worker::tests::gate_release_boundary_is_one_use_and_does_not_confirm_a_start`
- `canonical_worker::tests::exited_worker_reconciles_after_handles_are_lost_without_claiming_task_success`
- `canonical_worker::tests::gate_release_refuses_changed_target_authority_and_failed_commit_is_atomic`
- `canonical_worker::tests::gate_selection_claim_and_render_share_bounded_reads`
- `canonical_worker::tests::last_moment_brief_preflight_blocks_changed_authority_without_submission`
- `canonical_worker::tests::launch_advancement_recovers_each_boundary_then_delivers_brief_and_stops`
- `canonical_worker::tests::launch_advancement_selection_is_bounded_and_operation_scoped`
- `canonical_worker::tests::launch_advancement_refuses_unusable_environment_before_creation`
- `canonical_worker::tests::launch_reconciliation_ignores_unrelated_history_without_releasing_a_gate`
- `canonical_worker::tests::live_gate_proof_refuses_the_same_child_after_exec`
- `canonical_worker::tests::lost_creation_recovers_exact_resource_without_config_or_another_claim`
- `canonical_worker::tests::lost_or_foreign_brief_acknowledgments_retain_claim_and_never_repeat`
- `canonical_worker::tests::lost_workspace_reply_recovers_exact_live_marker_even_after_revocation_and_expiry`
- `canonical_worker::tests::malformed_neighbor_identity_never_panics_claims_or_sends_a_brief`
- `canonical_worker::tests::native_brief_original_deadline_bounds_a_stalled_submission`
- `canonical_worker::tests::native_gate_refusal_does_not_consume_the_release_opportunity_or_send_input`
- `canonical_worker::tests::native_start_confirmation_requires_real_exec_and_exact_agent_then_replays_read_only`
- `canonical_worker::tests::native_gate_submission_is_once_even_when_the_reply_is_lost`
- `canonical_worker::tests::new_workspace_creation_and_lost_layout_reply_are_one_use`
- `canonical_worker::tests::observed_resource_exit_before_target_commit_remains_recoverable`
- `canonical_worker::tests::prepared_launch_selection_rotates_stages_and_excludes_cancelled_or_expired_effects`
- `canonical_worker::tests::pane_conflicts_remain_effect_fences_with_ten_thousand_retired_neighbors`
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
- `canonical_worker::tests::supervised_root_recovery_needs_exact_process_and_preserves_expired_authority`
- `canonical_worker::tests::supervised_root_creation_and_commit_losses_recover_without_bootstrap_or_replay`
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
- `profile_preparation::tests::launch_ingress::canonical_worktree_paths_and_aliases_block_before_approval_consumption`
- `profile_preparation::tests::launch_ingress::draft_signature_and_reservation_preserve_the_exact_brief_and_one_use_boundary`
- `profile_preparation::tests::launch_ingress::lost_worktree_receipt_commit_recovers_after_approval_revocation_without_git_add`
- `profile_preparation::tests::launch_ingress::preparation_capture_releases_sqlite_and_rechecks_selected_state`
- `profile_preparation::tests::launch_ingress::preparation_stop_preserves_created_repositories_and_records_uncreated_plans_in_order`
- `profile_preparation::tests::launch_ingress::repository_observation_ignores_replacement_refs_and_refuses_lazy_fetch`
- `profile_preparation::tests::launch_ingress::signed_worktree_creation_retains_exact_checkout_and_recovers_without_replay`
- `profile_preparation::tests::launch_ingress::started_worktree_proof_allows_output_but_rejects_reassociation`
- `profile_preparation::tests::launch_ingress::worktree_creation_supports_multiple_repositories_binary_files_and_symlinks`
- `profile_preparation::tests::launch_ingress::unsigned_or_changed_launch_inputs_never_create_a_reservation`
- `profile_preparation::tests::launch_ingress::worktree_inventory_retains_uncertain_paths_and_refuses_corrupt_provenance`
- `profile_preparation::tests::launch_ingress::worktree_only_stop_fences_late_launch_and_preserves_uncertain_resources`
- `profile_preparation::tests::launch_ingress::worktree_preparation_uses_selected_history_through_receipt_commit`
- `profile_preparation::tests::launch_ingress::worktree_refusals_precede_approval_consumption_and_effects`
- `profile_preparation::tests::launch_ingress::worktree_verification_and_stop_select_only_their_launch_provenance`
- `runner::socket::tests::reply_is_bounded_and_requires_a_complete_utf8_line`
- `runner::socket::tests::trickle_reply_cannot_restart_deadline`
