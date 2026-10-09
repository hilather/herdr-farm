# LAUNCH-BUDGET-1 validation

All Cargo commands used `TMPDIR=$PWD/target/tmp` and
`nice -n 19 ionice -c 3 cargo ... --locked --offline -j 3`, with
`--features state-store`. No canonical schema change was needed.

The design gives draft and reserve independent 60-second deadlines inside the
600-second default launch admission window (`--budget-seconds` can bound it).
Each retries budget exhaustion at most three times. Launch Git observations
reserve their five-second command cap plus five seconds cleanup before spawn.
Only draft (no reservation writes) and atomic reserve are retried. The CLI
reports per-step elapsed milliseconds, preflight time, and the budget decision.
See [the deadline trace](canonical-worker-launch.md). Signing and filesystem
calls retain their existing behavior; the admission deadline does not forcibly
interrupt those calls.

Validation commands and results:

- `cargo test ... --lib launch`: 15 passed, 24 socket-only failures, 2 ignored.
- `cargo test ... --bin herdr-farm launch`: 15 passed, 5 socket-only failures.
- `cargo test ... --lib worktree`: 11 passed, 10 socket-only failures; an additional nested helper invocation passed.
- `cargo test ... --lib supervision`: 18 passed.
- `cargo clippy ... --all-targets`: completed; existing warnings, none on changed lines.
- `cargo test ... --test operator_launch`: initial run 29 passed / 7 socket-only failures; final run 29 passed / 7 socket-only failures / 1 existing timing-bound failure.
- Both new CLI regressions passed in the final full suite and in isolated reruns. The slow-draft fixture paused the CLI for 61 seconds after deadline creation, observed internal retry 2/3, and verified exactly one persisted attempt.
- The short-budget fixture verified a named refusal with timing/budget detail and no attempt, then reserved exactly once with sufficient budget.
- The existing `launch_run_busy_execution_lock_exhaustion_keeps_the_resumable_step_report` passed initially, failed its existing five-second timing assertion in the later parallel run, and passed an isolated rerun without changes.

Socket-only failures below were not worked around. Direct fixture failures report
`Operation not permitted`; CLI native-probe fixtures report `native probe server
exited` when their Python server cannot bind its Unix socket. The steward must
run these outside this sandbox. Worktree failures overlap library launch failures.

The three documented `copy_jobs` tests that assume a tree without Git were not
part of these filtered suites and were not changed. No owner roots, servers,
tickers, agent data, or canonical store schema were touched. No tests were
changed to bypass socket restrictions.

## Library launch socket-only failures

- `canonical_worker::tests::launch_reconciliation_ignores_unrelated_history_without_releasing_a_gate`
- `canonical_worker::tests::launch_advancement_refuses_unusable_environment_before_creation`
- `canonical_worker::tests::launch_advancement_recovers_each_boundary_then_delivers_brief_and_stops`
- `canonical_worker::tests::direct_workers_require_positive_visible_readiness_not_managed_launch_flag`
- `canonical_worker::tests::launch_advancement_selection_is_bounded_and_operation_scoped`
- `canonical_worker::tests::stock_herdr_launch_execs_the_launcher_in_place_of_the_shell`
- `canonical_worker::tests::prepared_launch_selection_rotates_stages_and_excludes_cancelled_or_expired_effects`
- `canonical_worker::tests::stock_herdr_unconfirmed_launch_closes_its_workspace_and_counts_nothing`
- `canonical_worker::tests::target_inventory_rejects_rehashed_inputs_and_broken_launch_operation_links`
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

## Binary launch socket-only failures

- `brief_jobs::launch::tests::changed_configuration_and_cancellation_after_claim_leave_uncertainty_without_start`
- `brief_jobs::launch::tests::lost_foreign_and_malformed_acknowledgements_never_repeat_launch`
- `brief_jobs::launch::tests::blocked_launch_retains_exclusion_and_cancels_its_subprocess`
- `brief_jobs::launch::tests::ambiguous_busy_missing_capability_and_missing_terminal_do_not_claim`
- `brief_jobs::launch::tests::restored_config_path_cannot_authorize_arguments_parsed_from_other_bytes`

## Worktree socket-only failures

- `profile_preparation::tests::launch_ingress::lost_worktree_receipt_commit_recovers_after_approval_revocation_without_git_add`
- `profile_preparation::tests::launch_ingress::canonical_worktree_paths_and_aliases_block_before_approval_consumption`
- `profile_preparation::tests::launch_ingress::started_worktree_proof_allows_output_but_rejects_reassociation`
- `profile_preparation::tests::launch_ingress::worktree_creation_supports_multiple_repositories_binary_files_and_symlinks`
- `profile_preparation::tests::launch_ingress::signed_worktree_creation_retains_exact_checkout_and_recovers_without_replay`
- `profile_preparation::tests::launch_ingress::worktree_inventory_retains_uncertain_paths_and_refuses_corrupt_provenance`
- `profile_preparation::tests::launch_ingress::worktree_only_stop_fences_late_launch_and_preserves_uncertain_resources`
- `profile_preparation::tests::launch_ingress::worktree_preparation_uses_selected_history_through_receipt_commit`
- `profile_preparation::tests::launch_ingress::worktree_refusals_precede_approval_consumption_and_effects`
- `profile_preparation::tests::launch_ingress::worktree_verification_and_stop_select_only_their_launch_provenance`

## Operator launch socket-only failures

- `canonical_worker_viewers_create_reopen_focus_and_close_only_the_recorded_tab`
- `code_launch_passes_only_selected_toolchain_environment_to_the_worker`
- `launch_run_retries_after_termination_but_refuses_an_unobserved_live_worker`
- `launch_run_with_a_dedicated_server_after_verify_interaction_reserves_both_kinds`
- `stale_profile_evidence_is_refreshed_by_launch_run`
- `verify_interaction_produces_launchable_evidence_for_codex_and_claude_from_the_cli`
- `verify_interaction_tolerates_agents_writing_into_their_execution_home`

