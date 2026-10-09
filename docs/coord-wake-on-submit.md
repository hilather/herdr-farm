# COORD-WAKE-ON-SUBMIT-1

Implemented on the current branch `factory/coord-wake` (the task label named
`factory/coord-wake-submit`; the checkout's current branch was kept).

Canonical controller passes now offer a supervised result-wake job through the
existing executor and root effect barrier. It reads only coordinator authority
and unfinished result rows, checks the owned, accepted coordinator journal,
current policy, session/pane incarnation and native idle/done readiness, and
rechecks readiness immediately before prompting. Working coordinators defer.
The healthy target is a 15-second controller pass plus a 45-second bounded job:
within 60 seconds after idle becomes observable. Contention or unavailable native
readiness can exceed that target and remain visible in health evidence.

`PROJECT.md` keeps its explicit `nudge = false` default for new projects: false
shows native toasts; true prompts. Existing projects were not edited. Changing
from a confirmed toast to prompt policy rearms unfinished results. Canonical
worker-result wakes and `inbox wait` use DONE, so context's existing SEEN writes
cannot consume them; ordinary wait notices retain unseen/unfinished semantics.

The retained `.state/canonical-wake.json` reuses typed notification claims and
retry records. Confirmed IDs dedupe shrinkage, replay and restart. Known toast
rejection and proven preflight failures back off. Lost/contradictory replies and
crashed pending claims fence overlapping IDs; inspect and handle those items,
then mark them DONE before allowing another wake. Each attempted delivery is
persisted before send and receives an outcome; the latest 1024 attempts are
available in `telemetry <slug> health --json`. Health also reports oldest
unfinished-result age, warning at one minute and critical at ten minutes.
Retention/backup classification was added for this retained payload. There are
no new crates, canonical migrations, SCHEMA bump, or sidecar schema changes.

## Diagnosis verification

Code inspection confirmed the canonical ticker skips the legacy pass and its
nudge step; automatic result notices previously inserted inbox rows without
queuing coordinator prompts; manual runtime notifications are native toasts;
false nudge policy selects toasts; and wait previously read only unseen rows
while context marks rows SEEN. The coordinator skill and prime text previously
required a background wait. The task's incident timestamps, live PROJECT.md
setting, missing wait invocations and zero notification operations were provided
incident evidence, not independently re-read from owner data or live services.

## Files

- Controller/executor: `src/canonical_controller.rs`, `src/canonical_coordinator.rs`,
  new `src/canonical_coordinator_wake.rs`, `src/copy_jobs/queue.rs`, `src/ticker.rs`,
  and session API calls through the configured runner in `src/herdr.rs`.
- Public reads/wait: `src/store/inbox.rs`, `src/store/effect_rows.rs`,
  `src/store/mod.rs`, `src/runtime.rs`, `src/inbox.rs`.
- Health/retention: `src/telemetry/health/{mod,rules}.rs`,
  `src/telemetry/maintenance/mod.rs`.
- E2E: `tests/canonical_coordinator.rs`, `tests/canonical_worker.rs`,
  `tests/telemetry_health.rs`, shared `tests/support/coordinator-herdr.py`.
- Documentation: `docs/operations.md`, `docs/test-tick-timing.md`,
  `docs/telemetry/contracts-health.md`, `skill/COORDINATOR.md`, and this report.

## Validation

Every cargo invocation used `TMPDIR=$PWD/target/tmp` and
`nice -n 19 ionice -c 3 cargo ... --locked --offline -j 3`.

| Suite | Full run | Follow-up |
| --- | --- | --- |
| canonical_coordinator | 12 passed, 7 socket failures | Socket-free lost-attempt/context/wait/health workflow passed |
| canonical_worker | 9 passed, 47 socket failures, 4 setup store deadlines | Sequential retries: 1 passed; the other 3 reached socket failures, including the new verified-worker wake E2E |
| telemetry_health | First run: 15 passed, 1 socket failure; later run: 14 passed, 1 socket failure, 1 ticker timeout | Final isolated ticker test passed (6.58 s); an earlier retry had writer contention |
| clippy --features state-store --all-targets | Completed successfully | No diagnostics on changed lines; existing warnings remain |

Across the final per-test outcomes: 37 tests passed, 58 remained blocked only by
socket restrictions. Socket server startup failures are the fixture's timeout
following Python `socket.bind` raising `Operation not permitted`; direct native
fixtures report the same OS error. No socket workaround was introduced.
`git diff --check` passed. New coverage uses CLI or public store lifecycle APIs;
no unit tests or source-text assertions were added. Native delivery, busy/idle
behavior, lost replies, toast retry and the time bound still require the
steward's run outside this sandbox. The supplied TMPDIR-specific copy_jobs tests
were not run or changed.

## Socket-only failures

### canonical_coordinator

- `canonical_toast_backoff_and_live_nudge_policy_survive_context_reads`
- `canonical_wake_lost_reply_fences_replay_until_done_and_releases_new_results`
- `migrated_doctor_reports_canonical_binding_despite_stale_legacy_record`
- `open_waits_for_brief_project_and_root_contention`
- `socket_coordinator_remote_manifest_and_explicit_local_override_policy`
- `socket_open_primes_owned_coordinator_retries_swallowed_prompt_and_recreates_closed_pane`
- `unchanged_ticker_observations_preserve_head_and_refresh_coordinator_admission`

### canonical_worker

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
- `launch_run_reviews_use_the_claude_spool_and_record_skeptical_yield`
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
- `verified_worker_results_wake_idle_coordinator_once_and_defer_while_busy`
- `wall_budget_termination_retries_contention_and_notifies_once`
- `wall_budget_termination_with_changed_frozen_definition_keeps_process_exit`
- `wall_budget_termination_without_contention`
- `worker_uid_default_root_and_verification_stay_frozen`
- `worker_uid_owner_and_verification_stay_frozen_with_mounts_enforced`

### telemetry_health

- `recommendations_and_notices_change_no_canonical_state_and_no_dispatch`

## Steward handoff

Rerun the named suites outside the hard sandbox, using isolated labs, the shared
test time scale, the prescribed TMPDIR and low-priority offline cargo commands.
Consider `-- --test-threads=3` to keep fixture compilation and CLI setup load
bounded as well as build load. No owner server, ticker or agent data was touched.
No push was performed.

After deployment, ask Matt/shepherd whether to set `nudge = true` in unreal-t4c
and tactics-dev PROJECT.md. That remains a project configuration decision.
