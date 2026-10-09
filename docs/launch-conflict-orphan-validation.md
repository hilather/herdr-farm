# LAUNCH-CONFLICT-ORPHAN-1 validation

Branch: `factory/launch-conflict-orphan`, based on LAUNCH-BUDGET-1 `19636ad`.
No canonical or sidecar schema change, migrations, new crates or deployment.

Design and files:

- `canonical_worker` launch/start/brief ingress reselects only the relevant
  delivery or attempt on conflicts, with three bounded passes through existing
  one-use effect boundaries. `store/controlled.rs` supplies scoped retry reads.
- `store/launch_failure.rs` records typed terminal diagnostics, retained effect
  event references and a deduplicated coordinator notice. Worktree-only failures
  release worker capacity; uncertain terminal resources retain capacity and the
  task pointer for cancellation. `store/worker_termination.rs` accepts that
  failed-but-retained staged attempt during proven termination.
- Dedicated-server loss requires an owner-controlled server record matching the
  recorded creation route and project/task, an absent socket directory and no
  server process naming it. Inaccessible identity evidence fails closed. Recovery,
  cancellation and forced server retirement preserve worktrees and can release
  capacity using this proof. Retirement records it before deleting `server.json`.
- Three identical non-transient recovery errors stop the queue and escalate;
  transient errors continue to retry. `store/controller_hint.rs` suppresses
  stopped recovery, and `store/inbox.rs` prevents a second generic end notice.
- `telemetry/health/rules.rs` adds `launch_stalled` (health-rules.v5): 5-minute warn,
  15-minute critical, or immediate critical for stopped recovery retaining capacity.
  Scoped store reads use progress timestamps appended by `store/launch.rs` and
  `store/worktrees.rs` in existing events, excluding errors.
- E2E additions in `tests/canonical_worker.rs` cover revision movement after
  creation, persistent conflicts before and after worktree readiness, ambiguous
  creation followed by dedicated-server removal, a transient missing socket,
  a live server process preventing proof, cancellation, forced retirement and
  bounded escalation. `tests/telemetry_health.rs` updates the public rule contract.
- Updated `docs/canonical-worker-launch.md` and `docs/telemetry/contracts-health.md`.

LAUNCH-BUDGET-1 already covers `insufficient transfer cleanup budget` via typed
`BudgetExhausted` and fresh bounded draft/reserve budgets. That design is retained.

Every Cargo invocation used `TMPDIR=$PWD/target/tmp` and
`nice -n 19 ionice -c 3 cargo ... --locked --offline -j 3 --features state-store`.
No repo-wide formatting, owner roots/data, live server/ticker, agent CLI, stash,
push or socket-sandbox workaround was used.

## Results

| Suite | Result |
| --- | --- |
| Library `canonical_worker` | 2 passed, 67 socket-only failures, 10 ignored |
| Library `launch` | 15 passed, 24 socket-only failures, 2 ignored |
| Latest combined library `canonical_worker` + `launch` | 17 passed, 82 socket-only failures, 10 ignored |
| Binary `launch` | 15 passed, 5 socket-only failures |
| CLI `canonical_worker` | 10 passed, 53 failed; four setup deadlines reached socket denial on isolated reruns; remaining failures were socket-only |
| Operator `operator_launch` | 28 passed, 9 failed; 7 socket-only; both other failures passed isolated reruns |
| Health E2E | 15 passed, 1 socket-only failure |
| Final four new launch regressions | 4 socket-only fixture failures before workflow execution |
| Clippy `--all-targets` | Passed; existing warnings, none on changed lines |

The first worker CLI run also hit a setup deadline in
`a_sandboxed_reviewer_uses_its_worker_channel_through_the_spool`; its isolated
rerun reached the socket denial. The four setup-deadline cases in the later full
run were `an_isolated_worker_cannot_read_owner_secrets_or_lift_the_hiding_but_still_commits_and_submits`,
`an_isolated_worker_submits_only_through_its_own_spool`,
`attention_mid_run_failure_remains_incomplete_at_termination` and
`canonical_attempt_sidebar_does_not_publish_to_a_replaced_terminal`.
Their isolated reruns likewise reached the socket denial.

The initial operator failures outside the socket boundary were
`code_launch_reserves_under_concurrent_writes_and_submits_all_scoped_changes`
(lock contention) and
`launch_run_records_reviews_and_skeptical_yield_and_refuses_without_writes`
(binary replacement while another build overlapped the test). Both passed on
isolated reruns after edits were frozen. Later validation runs were serialized.
The health rule-list expectation was updated to include the new rule and passed
on rerun.

The new regressions compile but cannot exercise their complete socket workflows
in this hard sandbox. The steward must run them outside. Socket fixture failures
are either direct `Operation not permitted`, `native probe server exited`, or
`Lab::serve` waiting for a socket after its Python server logs the bind denial.

The three documented `copy_jobs` cases that assume a tree without Git were not
included in these filtered suites and were not modified:
`artifact_transfer_scopes_nested_thread_dirs_to_one_common_git_dir`,
`final_worker_recovers_without_sender_and_finishes_copy_after_eligibility_loss`,
and `retained_recovery_never_refetches_and_config_withdrawal_preserves_intent`.
Their checkout-local TMPDIR limitation remains environmental.

## Socket-only failures

The CLI list includes the four cases whose initial setup deadline was resolved
by reaching the socket-denied fixture on isolated rerun.

### Library: `--lib -- canonical_worker launch`

- `canonical_worker::tests::brief_preparation_expiry_and_failed_commit_leave_no_delivery_obligation`
- `canonical_worker::tests::a_starting_agent_whose_own_display_state_moves_is_named_once_but_identity_drift_is_refused`
- `canonical_worker::tests::brief_preparation_and_delivery_do_not_decode_unrelated_approval_history`
- `canonical_worker::tests::brief_rendering_accounts_selected_inputs_before_decode_or_claim`
- `canonical_worker::tests::barrier_revocation_during_prompt_preserves_stale_delivery_and_stop_recovery`
- `canonical_worker::tests::brief_rendering_shares_deadline_and_does_not_scan_retained_history`
- `canonical_worker::tests::busy_foreign_and_changed_executable_workers_are_not_claimed`
- `canonical_worker::tests::controller_recovers_missing_initial_brief_without_sending_or_duplicating_it`
- `canonical_worker::tests::conflicting_legacy_socket_alias_prevents_brief_claim`
- `canonical_worker::tests::corrupt_target_payload_cannot_hide_a_retained_resource_using_a_terminated_peer`
- `canonical_worker::tests::creation_and_recovery_refuse_terminal_replacement_during_process_observation`
- `canonical_worker::tests::creation_claim_and_recovery_intent_roll_back_together_before_native_effects`
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
- `canonical_worker::tests::launch_advancement_selection_is_bounded_and_operation_scoped`
- `canonical_worker::tests::launch_reconciliation_ignores_unrelated_history_without_releasing_a_gate`
- `canonical_worker::tests::live_gate_proof_refuses_the_same_child_after_exec`
- `canonical_worker::tests::lost_or_foreign_brief_acknowledgments_retain_claim_and_never_repeat`
- `canonical_worker::tests::lost_creation_recovers_exact_resource_without_config_or_another_claim`
- `canonical_worker::tests::lost_workspace_reply_recovers_exact_live_marker_even_after_revocation_and_expiry`
- `canonical_worker::tests::malformed_neighbor_identity_never_panics_claims_or_sends_a_brief`
- `canonical_worker::tests::native_brief_original_deadline_bounds_a_stalled_submission`
- `canonical_worker::tests::native_gate_refusal_does_not_consume_the_release_opportunity_or_send_input`
- `canonical_worker::tests::native_gate_submission_is_once_even_when_the_reply_is_lost`
- `canonical_worker::tests::native_start_confirmation_requires_real_exec_and_exact_agent_then_replays_read_only`
- `canonical_worker::tests::new_workspace_creation_and_lost_layout_reply_are_one_use`
- `canonical_worker::tests::observed_resource_exit_before_target_commit_remains_recoverable`
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

### Binary: `--bin herdr-farm launch`

- `brief_jobs::launch::tests::ambiguous_busy_missing_capability_and_missing_terminal_do_not_claim`
- `brief_jobs::launch::tests::changed_configuration_and_cancellation_after_claim_leave_uncertainty_without_start`
- `brief_jobs::launch::tests::lost_foreign_and_malformed_acknowledgements_never_repeat_launch`
- `brief_jobs::launch::tests::blocked_launch_retains_exclusion_and_cancels_its_subprocess`
- `brief_jobs::launch::tests::restored_config_path_cannot_authorize_arguments_parsed_from_other_bytes`

### CLI: `--test canonical_worker`

- `a_brief_swallowed_by_the_agent_is_redelivered_and_confirmed_only_once_accepted`
- `a_brief_the_agent_never_accepts_is_left_ambiguous_not_confirmed`
- `a_launch_outlasts_an_operator_holding_the_root_after_its_workspace_creation`
- `a_hidden_path_covering_the_execution_home_refuses_the_launch_before_creation`
- `a_launch_reaches_running_while_another_holder_takes_the_shared_root_intermittently`
- `a_worker_that_dies_without_submitting_is_noticed_and_preserved_while_another_attempts_check_runs`
- `a_worker_that_exits_after_submitting_is_recorded_terminated_while_another_attempts_check_runs`
- `a_worker_that_exits_during_its_own_verification_ends_after_the_verdict_without_contention`
- `a_legacy_thread_holding_the_planned_worktree_blocks_its_creation`
- `a_proven_worker_end_keeps_the_project_admitted_but_an_unexplained_pane_loss_pauses_it`
- `a_subdirectory_binding_runs_in_the_same_subdirectory_of_the_new_worktree`
- `a_sandboxed_reviewer_uses_its_worker_channel_through_the_spool`
- `a_worker_branch_reaching_a_corrupt_quarantined_object_is_refused`
- `accepted_editing_worker_completes_automatically_after_integration`
- `an_isolated_worker_cannot_read_owner_secrets_or_lift_the_hiding_but_still_commits_and_submits`
- `an_isolated_worker_submits_only_through_its_own_spool`
- `ambiguous_creation_with_removed_dedicated_server_releases_capacity_once`
- `an_automatic_pause_names_its_blockers_and_lifts_itself_once_they_clear`
- `accepted_verify_only_editing_worker_completes_without_integration_automation`
- `an_isolated_codex_worker_commits_through_codex_workspace_write_sandbox`
- `an_operator_finishes_a_worker_that_never_submitted_and_the_result_lands_automatically`
- `an_untracked_working_directory_is_refused_before_the_approval_is_used`
- `attention_mid_run_failure_remains_incomplete_at_termination`
- `canonical_attempt_sidebar_does_not_publish_to_a_replaced_terminal`
- `canonical_attempt_sidebar_clears_after_termination_in_an_active_project`
- `canonical_attempt_sidebar_restart_offers_no_historical_cleanup_or_native_request`
- `canonical_attempt_sidebar_uses_collected_usage_and_observed_waiting`
- `canonical_attempt_sidebar_refreshes_and_clears_on_pause_and_termination`
- `barrier_stop_gets_a_fair_turn_and_maintenance_keeps_advancing`
- `concurrent_attempt_tokens_publish_and_missing_retired_server_stays_quiet`
- `dedicated_worker_refuses_remote_manifest_before_its_brief`
- `launch_run_reviews_use_the_claude_spool_and_record_skeptical_yield`
- `idle_worker_notice_restarts_stretch_and_deduplicates_within_a_ticker`
- `editing_worker_requires_operator_completion_when_automation_is_off`
- `launch_run_reviews_use_the_codex_spool_and_record_skeptical_yield`
- `launch_revision_conflict_reselects_without_duplicate_resources_or_brief`
- `launch_sets_the_intended_permission_mode_over_a_stale_one_in_the_home`
- `persistent_launch_conflict_closes_attempt_and_notifies_once`
- `rejected_editing_worker_stays_running_and_can_resubmit`
- `repeated_nontransient_recovery_stops_and_escalates_with_capacity_retained`
- `review_assignment_launches_with_blind_brief_and_records_session`
- `submit_captured_retains_remember_from_the_attempt_report_and_replays_once`
- `ticker_launches_and_briefs_once_then_stops_a_cancelled_worker_while_paused_and_revoked`
- `ticker_does_not_dispatch_a_launch_cancelled_before_creation`
- `ticker_launches_nothing_on_a_server_without_the_launch_contract_or_while_paused`
- `ticker_recovers_a_lost_creation_reply_without_creating_again`
- `ticker_retires_a_cancelled_gated_worker_without_starting_it`
- `ticker_stops_the_dedicated_herdr_server_of_a_finished_task`
- `wall_budget_termination_with_changed_frozen_definition_keeps_process_exit`
- `wall_budget_termination_retries_contention_and_notifies_once`
- `wall_budget_termination_without_contention`
- `worker_uid_default_root_and_verification_stay_frozen`
- `worker_uid_owner_and_verification_stay_frozen_with_mounts_enforced`

### Operator: `--test operator_launch`

- `canonical_worker_viewers_create_reopen_focus_and_close_only_the_recorded_tab`
- `code_launch_passes_only_selected_toolchain_environment_to_the_worker`
- `launch_run_retries_after_termination_but_refuses_an_unobserved_live_worker`
- `launch_run_with_a_dedicated_server_after_verify_interaction_reserves_both_kinds`
- `stale_profile_evidence_is_refreshed_by_launch_run`
- `verify_interaction_produces_launchable_evidence_for_codex_and_claude_from_the_cli`
- `verify_interaction_tolerates_agents_writing_into_their_execution_home`

### Health: `--test telemetry_health`

- `recommendations_and_notices_change_no_canonical_state_and_no_dispatch`


## LAUNCH-CONFLICT-ORPHAN-1b

The missing-record proof now accepts an owner-owned run directory (or an absent
run directory) when `server.json` is missing. The socket still comes exclusively
from this operation's identity-checked `runtime.launch_creation` route, its parent
must be absent, and `/proc` must contain no readable environment naming that
socket. Unreadable evidence for this UID fails closed. The scan covers all
processes, without relying on their command-line name. Present records retain
ownership, permissions, project/task/socket/PID and operator-management checks;
unreadable or invalid records cannot establish loss.

Recovery uses the existing `Lost` / `termination_observed=true` path, retaining
worktrees and releasing capacity and write claims with one notice. Cancellation
already shared this proof in `61c60a6`; no separate cancellation change or schema
migration is needed.

The existing public-entry-point E2E
`ambiguous_creation_with_removed_dedicated_server_releases_capacity_once` now
covers recovery and cancellation after record removal, signed inputs retained
in the run directory, absent socket parent, ambiguous creation with no target or
start, a live route holder whose command does not end in `server`, operator and
invalid records, released persisted write claims, and replay notice deduplication.
The fixture continues to cover a present dedicated record and forced stop.


### 1b validation results

All commands used checkout-local `TMPDIR=$PWD/target/tmp`, offline locked Cargo,
`nice -n 19 ionice -c 3`, and `-j 3`. No socket workaround was attempted.

| Suite | Result |
| --- | --- |
| Library `canonical_worker` | 2 passed, 67 socket-only failures, 10 ignored |
| Library `launch` | 15 passed, 24 socket-only failures, 2 ignored |
| Binary `launch` | 15 passed, 5 socket-only failures |
| CLI `canonical_worker launch` filter | 1 passed, 12 socket-only failures |
| Full CLI `canonical_worker` (final test edits) | 10 passed, 53 socket-only failures |
| Operator `operator_launch` | 30 passed, 7 socket-only failures |
| Health `telemetry_health` | 14 passed, 1 socket-only failure, 1 transient ticker timeout |
| Isolated health ticker rerun | 1 passed |
| Clippy `--all-targets` after final code/test edits | Passed; existing warnings, none in changed lines |

The socket-only failure names are listed in the existing suite inventories above:
all 67 library worker names, all 24 library launch names, all 5 binary launch
names, all 53 CLI worker names, all 7 operator socket names, and the one health
socket name (`recommendations_and_notices_change_no_canonical_state_and_no_dispatch`).
The requested worker/launch suites had no additional failure names or non-socket
failures. The additional health suite's
`ticker_health_evaluates_by_default_obeys_interval_and_never_notifies` timed out
waiting for ticker accounting in the full run, then passed its isolated rerun. CLI fixture timeouts occurred
at `Lab::serve` after Python logged `PermissionError: Operation not permitted`.
The expanded orphan regression failed at that same socket fixture boundary;
its recovery/cancellation assertions must be exercised by the steward outside
the sandbox. The three checkout-TMPDIR `copy_jobs` environmental cases remain
unchanged and were not selected by these suites.


## LAUNCH-CONFLICT-ORPHAN-1c

The six steward findings are addressed as follows:

1. `reconcile_launch` validates the supplied revision before any disappearance
   observation can write. It no longer adopts a fresh revision on a caller's
   conflict. Selection and commit conflicts propagate without escalation.
2. Recovery escalation requires a typed socket-observation failure from the API
   using the validated creation intent. Native start confirmation, store errors,
   JSON errors and unrelated filesystem failures propagate; they are not evidence
   that the launch's resources failed. Three identical permanent socket failures
   still stop recovery with capacity retained, and proven disappearance still
   records `Lost` immediately.
3. SQLite `SQLITE_CONSTRAINT_TRIGGER` maps to a failed-commit `Io` diagnostic,
   rather than the generic constraint `Conflict`. A trigger's explicit abort is
   not a revision race. Brief preparation/delivery check cancellation and expiry
   before exhausted-conflict notification. Expiry and failed commits add no
   failure events or brief obligations. The new E2E
   `launch_store_commit_abort_and_stale_claim_leave_state_unchanged` creates a
   signed launch through the CLI, exercises the public store claim API, and
   compares persisted snapshots including approval consumption and inbox state.
4. A terminated attempt correctly loses access to live `memory attempt-input`.
   The persistent-conflict test reads immutable retained launch inputs through
   the public snapshot instead, requires nonempty worktree plans, and asserts
   every planned checkout remains. No production knowledge binding changed.
5. `dedicated_server_gone` rejects `server.json` when `mode & 0o022 != 0`.
   The old fixture wrote it with default permissions, so a group-writable host
   umask could reject the intended owner-controlled record. The fixture now
   explicitly uses mode 0600 and also tests mode 0660 rejection before restoring
   0600. Production dedicated-record creation and viewer rewrites now explicitly
   use 0600 too: their former `fs::write` calls had the same umask dependency.
   Existing operator E2E coverage launches with child-only umask 0002, asserts the
   persisted record is 0600, and verifies a viewer rewrite repairs a 0660 record.
   Run-directory ownership, exact recorded socket, socket-parent absence,
   matching record and the all-process `/proc` scan remain unchanged. The
   steward's actual host umask/process evidence cannot be observed in this
   sandbox; full orphan recovery remains subject to outside validation.
6. The flaky test stopped the ticker once the server logged `agent.prompt`,
   before the brief transaction necessarily committed. It now waits for both
   persisted `Running` and a confirmed brief delivery. Its exact one-create,
   one-prompt and running-attempt assertions and original deadline remain.

Files: `src/canonical_worker.rs`, `src/canonical_worker/resources.rs`,
`src/canonical_worker/start.rs`, `src/store/mod.rs`,
`src/launch_run.rs`, `tests/canonical_worker.rs`, `tests/operator_launch.rs`,
`docs/canonical-worker-launch.md`, and this report.
No new crates, canonical or sidecar schema changes, migrations, source-text tests,
unit tests, ignored tests, relaxed assertions or widened deadlines.


### 1c validation

Every Cargo invocation used checkout-local `TMPDIR=$PWD/target/tmp`,
`nice -n 19 ionice -c 3`, `--locked --offline -j 3` and `--features state-store`.
Suites ran serially with `--test-threads=3`; no socket workaround was used.

| Suite | Result |
| --- | --- |
| Focused public-store E2E | 1 passed |
| Combined library `canonical_worker` + `launch` | 17 passed, 82 socket-only failures, 10 pre-existing ignored |
| Binary `launch` | 15 passed, 5 socket-only failures |
| Full CLI `canonical_worker` | 11 passed, 53 socket-only failures |
| Operator `operator_launch` | 30 passed, 7 socket-only failures |
| Health `telemetry_health` | 15 passed, 1 socket-only failure |
| Clippy `--all-targets` | Passed; existing warnings, none on changed lines |

The CLI failures were 50 `Lab::serve` startup waits after Python's bind denial
and three direct verification-lab bind failures. The library and binary failures
reported `Operation not permitted` or the resulting native probe exit. The seven
operator failures likewise reported socket denial/probe exit; health's sole
failure was `recommendations_and_notices_change_no_canonical_state_and_no_dispatch`
at its socket bind. Clippy completed successfully and its diagnostic spans were
checked against the added/changed lines. All exact failure names are already listed in the suite inventories above; no name
has been omitted or added. The six steward regressions compile, but their socket
workflows cannot reach the hotfix assertions here.

The three checkout-TMPDIR `copy_jobs` environmental cases were not selected by
these suites and remain unchanged. They are environmental, not product regressions:
`artifact_transfer_scopes_nested_thread_dirs_to_one_common_git_dir`,
`final_worker_recovers_without_sender_and_finishes_copy_after_eligibility_loss`,
and `retained_recovery_never_refetches_and_config_withdrawal_preserves_intent`.

The binary launch suite was repeated after the final dedicated-record writer edit:
15 passed and the same five socket-only failures. Across the five full suites,
88 tests passed, 148 failed only at denied socket fixtures, and 10 existing
library tests remained ignored. The standalone new public-store E2E also passed.
No additional non-socket failures remained. Socket workflows, including the
orphan loss proof and umask/viewer assertions, require the steward's outside run.
