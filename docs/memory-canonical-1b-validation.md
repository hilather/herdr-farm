# MEMORY-CANONICAL-1b validation

Branch: `factory/memory-canonical-b`. Sandbox run: 2026-10-04.

## Design and files

- Optional `## Remember` evidence comes from the submission's `report` JSON
  field; `submit-captured` reads the retained attempt output `report.md`.
- Schema 71 (`migrations/0071_result_memory.sql`) retains capture obligations,
  first-submission provenance and delegated decisions. `src/store/result_memory.rs`
  recovers capture through the existing proposal path in `src/memory/result_memory.rs`;
  proposal persistence commits one coordinator notice. Identity is attempt plus
  extracted content digest. Text remains data, including instruction-like text.
- Dedicated `memory list/show/approve/reject` verbs and generated coordinator
  permissions expose the narrow 2026-10-04 owner delegation. Approval creates an
  active informational observation, never mandatory memory. Review audit, owner
  inbox item and the existing Herdr notification operation commit together.
  Handling the inbox item or changing the producer task revision cannot suppress
  this independent notification. Signed owner review/promote remains unchanged.
- Worker framing uses estimator v3 for the new intake instructions. The change
  to `launch run` is the estimator identity only; snapshot selection scope is
  unchanged. Canonical evidence has lifetime retention/whole-store backup
  classification in `src/telemetry/maintenance/mod.rs`.
- Flow/delegation docs: `docs/memory-store.md`, `docs/canonical-worker-launch.md`,
  `docs/operations.md`; migration index: `docs/telemetry/contracts.md`.

## Checks

Every cargo invocation used `TMPDIR=$PWD/target/tmp`, `--locked --offline -j 3`
and `--features state-store`. No repo-wide formatting, new crates or source-text
assertions were introduced. New coverage uses the CLI and public store API over
isolated temporary projects and local external-command fixtures.

| Suite/check | Result |
| --- | --- |
| `cargo check` | passed |
| `memory_barriers` | 6 passed |
| `memory_control` | 13 passed |
| `memory_read_sets` | 4 passed |
| `memory_regressions` | 14 passed |
| `plans` | final full run: 10 passed |
| final focused `canonical_remember_is_captured_once_and_decisions_notify_the_owner` | passed after final indexed notification/evidence changes |
| `canonical_coordinator` | 9 passed; 5 socket-only failures |
| `canonical_worker` | full run: 5 passed, 35 socket-only failures and one budget-fixture failure; updated budget fixture's focused rerun passed |
| `cli::launch_worker_snapshot_cli_retains_instructions_and_refuses_missing_source` | passed |
| `cargo clippy --all-targets` | completed; existing warnings only, none in changed lines |
| `git diff --check` | passed |

The initial plans run had a transient `isolation_setup_failed` in
`ticker_requests_replans_only_while_enabled_and_active`; its final full-suite
rerun passed. Implementation/test failures found during development were fixed
and the final capture/decision workflow and budget fixture reruns passed.

## Socket-only failures requiring the steward's unsandboxed run

The coordinator failures directly report `Operation not permitted` at
`UnixListener::bind`. Each worker failure below is the fixture's socket startup
wait at `Lab::serve`, after Python reports `PermissionError: [Errno 1] Operation
not permitted` from `server.bind(path)`. No socket workaround was attempted.
Consequently the generated-settings assertion and captured-report workflows
could not reach their assertions in this sandbox.

`tests/canonical_coordinator.rs`:

- `migrated_doctor_reports_canonical_binding_despite_stale_legacy_record`
- `open_waits_for_brief_project_and_root_contention`
- `socket_open_primes_owned_coordinator_retries_swallowed_prompt_and_recreates_closed_pane`
- `socket_coordinator_remote_manifest_and_explicit_local_override_policy`
- `unchanged_ticker_observations_preserve_head_and_refresh_coordinator_admission`

`tests/canonical_worker.rs`:

- `a_brief_swallowed_by_the_agent_is_redelivered_and_confirmed_only_once_accepted`
- `a_hidden_path_covering_the_execution_home_refuses_the_launch_before_creation`
- `a_brief_the_agent_never_accepts_is_left_ambiguous_not_confirmed`
- `a_legacy_thread_holding_the_planned_worktree_blocks_its_creation`
- `a_launch_reaches_running_while_another_holder_takes_the_shared_root_intermittently`
- `a_proven_worker_end_keeps_the_project_admitted_but_an_unexplained_pane_loss_pauses_it`
- `a_subdirectory_binding_runs_in_the_same_subdirectory_of_the_new_worktree`
- `a_worker_branch_reaching_a_corrupt_quarantined_object_is_refused`
- `a_sandboxed_reviewer_uses_its_worker_channel_through_the_spool`
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
- `dedicated_worker_refuses_remote_manifest_before_its_brief`
- `launch_run_reviews_use_the_claude_spool_and_record_skeptical_yield`
- `editing_worker_requires_operator_completion_when_automation_is_off`
- `launch_run_reviews_use_the_codex_spool_and_record_skeptical_yield`
- `launch_sets_the_intended_permission_mode_over_a_stale_one_in_the_home`
- `rejected_editing_worker_stays_running_and_can_resubmit`
- `review_assignment_launches_with_blind_brief_and_records_session`
- `submit_captured_retains_remember_from_the_attempt_report_and_replays_once`
- `ticker_does_not_dispatch_a_launch_cancelled_before_creation`
- `ticker_launches_and_briefs_once_then_stops_a_cancelled_worker_while_paused_and_revoked`
- `ticker_launches_nothing_on_a_server_without_the_launch_contract_or_while_paused`
- `ticker_recovers_a_lost_creation_reply_without_creating_again`
- `ticker_retires_a_cancelled_gated_worker_without_starting_it`
- `ticker_stops_the_dedicated_herdr_server_of_a_finished_task`

## Limits

The socket-dependent worker, capture and generated-settings scenarios still
need the steward's run outside the sandbox. The task's three known `copy_jobs`
tests were not part of these targeted suites; they were neither changed nor
reclassified as code failures. Existing owner/server/agent data and running
services were not used. No push was performed.
