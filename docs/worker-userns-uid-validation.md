# WORKER-USERNS-UID-1 validation

Branch: `factory/worker-userns-uid`, based on `c6bf0e2`.

## Design and scope

The external owner-only per-project safety setting `worker_uid` accepts `root`
(default) or `owner`. A new optional uid/gid pair is sealed in the existing
approved launch payload. Absent fields remain absent on serialization, retaining
historical/default launch identities without a schema migration. Worker gate
creation/release and verification/integration checks reuse that pair. Native
profile probes use the owner's selected mapping. The outer root namespace still
performs mounts; only the inner agent/check exec changes. All added process
launches use the existing gated spawn paths. Transfers and routine execution
were not changed; no crates were added.

PROJECT.md front matter refuses the setting, including nested tables. Explicit
policy sources must be external to the project, owner-owned and not writable by
group/other. Invalid values refuse. HOME, Git quarantine and shared login file
ownership remain the same host ownership; inherited token descriptors need no
chown. The verifier maps in its gated child pre-exec step using a private
setup-proc descriptor, closed before exec, because its visible `/proc` is
read-only. Supervisor dumpability is disabled to block parent-fd reopening.
Codex's permission grants are unchanged. See the config reference in
[operations](operations.md), [worker isolation](profiles.md#filesystem-isolation),
the [isolation review update](reviews/2026-09-29-worker-isolation.md), and
[verification](factory/verified-results.md).

## Commands and results

Every cargo command used `TMPDIR=$PWD/target/tmp` and
`nice -n 19 ionice -c 3 cargo ... --locked --offline -j 3`. No repo-wide formatting,
agent CLI, live project/server/ticker, deployment, or owner config was run or
changed. Fixture agents and external services use isolated temporary labs.

- `cargo check --features state-store`: passed; existing dead-code warnings.
- `cargo clippy --features state-store --all-targets`: passed, zero warning
  locations on changed lines. Existing unrelated warnings remain.
- `cargo test --features state-store --test projects --test thread_sandbox
  --no-fail-fast -- --test-threads=3`: 9 passed. This includes strict values,
  defaults, per-project selection, PROJECT.md rejection, project-local source
  rejection and group/other-writable source rejection. A temporary error-context
  regression was corrected and the whole pair of suites rerun successfully.
- `cargo test --features state-store --test canonical_worker uid_
  -- --test-threads=3`: 1 passed, 2 socket-only failures. The passing public
  CLI/store workflow verifies both mappings are retained in approved launch
  inputs after config edits, including omission of the default root field.
  Initial new fixture setup failures were corrected by acknowledging owner
  config changes through project control before draft/contract installation.
- `cargo test --features state-store --lib verification::
  -- --test-threads=3`: 15 passed. Namespace execution is restricted here;
  these existing tests do not establish that the opted-in execution ran.
- `cargo test --features state-store --lib admission::
  -- --test-threads=3`: 16 passed, 2 store-deadline failures. Individual retries
  passed `contract_routes_by_profile_kind_and_required_capabilities` but
  `admission_resumes_after_a_full_page_of_unsigned_candidates` still failed with
  `state store: Deadline` at `src/admission.rs:1054`. This is an unresolved
  existing test failure under load, not a socket-only failure; steward recheck
  is required.

Logs remain under `target/worker-userns-*.log` (ignored build artifacts).
The three documented `copy_jobs::tests` affected by a TMPDIR within a Git tree
were not run or changed.

## Socket-only uid workflow failures

The fixture server's Unix socket bind returns `PermissionError: [Errno 1]
Operation not permitted`; the lab then times out waiting for its socket. No
workaround was attempted. The steward must run these outside this sandbox:

- `worker_uid_default_root_and_verification_stay_frozen`
- `worker_uid_owner_and_verification_stay_frozen_with_mounts_enforced`

Both workflows run a fixture agent and an owner-declared verification toolchain
through public entry points. They assert `id -u` for both processes, preserved
hidden/read-only paths, private HOME writes, unchanged host files, accepted
persisted verification state, and the original mapping after a mid-flight config
edit. The root case leaves the setting absent. The owner case requires the
lab's non-zero host uid. The real agent and UnrealEditor-Cmd deployment checks
remain for the owner/steward; this sandbox cannot establish their outcome.

## Broad canonical-worker suite

The broad `--test canonical_worker` run finished with 9 passed and 50 failed.
42 failures were the Unix fixture socket-bind refusal listed below. Three more
were TCP fixture bind refusals, also `Operation not permitted`. Three tests
hit store deadlines (`an_automatic_pause_names_its_blockers_and_lifts_itself_once_they_clear`,
`launch_sets_the_intended_permission_mode_over_a_stale_one_in_the_home`, and
`owner_uid_policy_is_retained_in_approved_launch_inputs_without_a_server`).
Two new uid workflows failed initial config acknowledgment; those fixtures were
corrected. The focused uid rerun then passed retention and reached socket-only
failures. The broad run used the earlier fixtures; the later focused run covers
the corrected fixtures. The two existing deadline failures were retried individually.

Socket-only failures in that broad run:

- `a_brief_the_agent_never_accepts_is_left_ambiguous_not_confirmed`
- `a_brief_swallowed_by_the_agent_is_redelivered_and_confirmed_only_once_accepted`
- `a_hidden_path_covering_the_execution_home_refuses_the_launch_before_creation`
- `a_launch_outlasts_an_operator_holding_the_root_after_its_workspace_creation`
- `a_launch_reaches_running_while_another_holder_takes_the_shared_root_intermittently`
- `a_legacy_thread_holding_the_planned_worktree_blocks_its_creation`
- `a_subdirectory_binding_runs_in_the_same_subdirectory_of_the_new_worktree`
- `a_proven_worker_end_keeps_the_project_admitted_but_an_unexplained_pane_loss_pauses_it`
- `a_sandboxed_reviewer_uses_its_worker_channel_through_the_spool`
- `a_worker_branch_reaching_a_corrupt_quarantined_object_is_refused`
- `accepted_editing_worker_completes_automatically_after_integration`
- `accepted_verify_only_editing_worker_completes_without_integration_automation`
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
- `dedicated_worker_refuses_remote_manifest_before_its_brief`
- `concurrent_attempt_tokens_publish_and_missing_retired_server_stays_quiet`
- `launch_run_reviews_use_the_claude_spool_and_record_skeptical_yield`
- `idle_worker_notice_restarts_stretch_and_deduplicates_within_a_ticker`
- `editing_worker_requires_operator_completion_when_automation_is_off`
- `launch_run_reviews_use_the_codex_spool_and_record_skeptical_yield`
- `rejected_editing_worker_stays_running_and_can_resubmit`
- `review_assignment_launches_with_blind_brief_and_records_session`
- `submit_captured_retains_remember_from_the_attempt_report_and_replays_once`
- `ticker_does_not_dispatch_a_launch_cancelled_before_creation`
- `ticker_launches_and_briefs_once_then_stops_a_cancelled_worker_while_paused_and_revoked`
- `ticker_launches_nothing_on_a_server_without_the_launch_contract_or_while_paused`
- `ticker_recovers_a_lost_creation_reply_without_creating_again`
- `ticker_retires_a_cancelled_gated_worker_without_starting_it`
- `ticker_stops_the_dedicated_herdr_server_of_a_finished_task`
- `wall_budget_termination_retries_contention_and_notifies_once`
- `wall_budget_termination_with_changed_frozen_definition_keeps_process_exit`
- `wall_budget_termination_without_contention`

TCP socket-only failures (local deterministic held-check fixture):

- `a_worker_that_dies_without_submitting_is_noticed_and_preserved_while_another_attempts_check_runs`
- `a_worker_that_exits_after_submitting_is_recorded_terminated_while_another_attempts_check_runs`
- `a_worker_that_exits_during_its_own_verification_ends_after_the_verdict_without_contention`

The two existing canonical deadline failures were retried after the builds:
both reached the Unix fixture bind refusal instead. Additional socket-only
failures from those retries:

- `an_automatic_pause_names_its_blockers_and_lifts_itself_once_they_clear`
- `launch_sets_the_intended_permission_mode_over_a_stale_one_in_the_home`

After the verifier's private-proc pre-exec mapping change, the focused
verification suite was rerun (15 passed), the uid workflows were rerun (retention
passed; both execution cases socket-only failures), and all-targets clippy was
rerun (passed, no warnings on changed lines). The owner check fixture also asserts
that it cannot inspect `/proc/1/fd`, fencing the setup-only mapping descriptor.
No socket or namespace restriction was bypassed. Actual namespace execution and
the unresolved admission paging deadline still require steward validation.
