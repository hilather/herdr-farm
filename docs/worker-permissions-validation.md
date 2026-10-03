# PERM-1 worker permission validation

Implemented on `factory/worker-permissions` for the 2026-10-03 owner policy.

## Design and files

`src/worker_permissions.rs` owns classification, requests, owner decisions,
launch-time script checks and restart obligations. `src/project.rs` defaults
unset policies to coordinator and validates owner extras. CLI, launch jobs,
ticker, thread list, context and doctor consume the same managed grants.
Automatic grants match an exact build/test prefix, an owner extra, or a regular
tracked file resolved by `git ls-tree` on the integration/default branch.
Worker-only files and interpreter code options cannot establish authority.
Script provenance records the target blob; merged updates remain granted and
removal suspends the rule. Worker script paths cannot resolve outside checkout.

Authority and restart history are a versioned project-local JSON stream:
`.state/worker-permissions.json`, migration
`migrations/worker-permissions/0001.json`, and the stream's MIGRATIONS list.
Canonical SQLite schema is unchanged and no new tables or crates are added.
Retention classification and telemetry backup exclusions are updated in
`src/telemetry/maintenance/`; the stream needs lifetime retention and
whole-project backup. Owner configuration is never rewritten.

Restarts use the existing branch/worktree/brief path under the ticker's root
execution lease. Idle/done/blocked agents are gracefully finished; working
agents wait. Pending grants batch together and lifecycle generations prevent
repeating a completed restart. Adopted/resolved threads are excluded.
Uncommitted work stays in the existing checkout. Conversation context is lost.

Updated operations.md, getting-started.md, profiles.md, telemetry/contracts.md
and skill/COORDINATOR.md. New coverage is exclusively CLI E2E, in projects.rs
and ticker_jobs.rs; it reads no src files and uses temporary projects and local
Herdr fixtures. The coordinator allow-list includes grant and read-only requests,
with approve/reject/revoke reserved for the owner's terminal.

## Results

- `cargo check --locked --offline -j 3 --features state-store`: passed.
- `cargo check --locked --offline -j 3`: passed on the final implementation.
- `cargo clippy --locked --offline -j 3 --features state-store --all-targets`:
  passed; pre-existing warnings remain, none on changed lines.
- `cargo test --locked --offline -j 3 --features state-store --test projects --test profiles --test threads --test ticker_jobs --test delivery --no-fail-fast`:
  12 passed (`projects`: 6, `profiles`: 6); 23 socket-only failures below.
- `git diff --check`: passed.

The passing permission E2E covers committed scripts, a worker-only script,
interpreter narrowing, network and shell escalation, owner policy and extras,
owner terminal approval/rejection/revocation, attribution, merged script changes,
target deletion, interpreter option injection, explicit Claude launch arguments,
invalid policy validation and unchanged owner configuration.

## Socket-only failures

Every test below failed in fixture setup at UnixListener::bind with OS error 1,
`Operation not permitted`. No behavior assertion ran. The steward must run
these outside this sandbox; no workaround was attempted.

### tests/delivery.rs

- `a_lost_start_is_reported_once_and_only_a_restart_starts_the_agent_again`

### tests/threads.rs

- `reprime_updates_only_the_priming_fields_of_the_coordinator_record`
- `resolved_done_thread_cleans_its_pane_and_worktree_but_keeps_branch_and_report`
- `integrated_resolution_enforces_policy_readiness_and_git_ancestry`
- `prompt_refuses_a_bare_shell_a_blocked_or_unknown_agent_and_sends_otherwise`
- `restart_follows_what_the_record_reached`
- `report_review_ack_and_resolve_copy_home`
- `start_restart_and_adopt_write_briefs_branches_and_launch_line`
- `thread_list_groups_every_record_and_live_state`
- `thread_start_uses_agent_arguments_only_for_the_kind_they_are_bound_to`
- `ticker_crash_after_pane_close_resumes_without_repeating_it`
- `unsafe_resolved_threads_are_kept_and_cleanup_keep_opts_out`
- `used_quiesced_project_migrates_after_final_copy_and_memory_record`

### tests/ticker_jobs.rs

- `a_primed_coordinator_is_primed_again_only_after_an_explicit_reprime`
- `linked_worktree_start_acknowledges_the_recorded_default_arguments`
- `a_failed_coordinator_start_backs_off_while_another_project_works`
- `open_after_a_coordinator_start_keeps_its_claim_and_never_starts_again`
- `a_due_legacy_routine_is_claimed_once_and_delivered_after_its_command_ends`
- `a_thread_start_is_confirmed_on_acknowledgement_without_waiting_for_the_agent`
- `remote_machines_poll_on_their_own_deadlines_and_only_long_outages_are_reported`
- `token_refreshes_follow_each_panes_current_group_and_write_no_execution_state`
- `token_refreshes_cool_down_while_another_coordinator_starts_and_primes`
- `worker_permission_grants_restart_preserving_work_and_revoke_next_start`

## Limits and remaining verification

Automatic committed-script verification currently supports owner-configured
local repositories. Remote-only script requests conservatively escalate.
Unsupported permission-pattern syntax is recorded as a request but must be
narrowed before it can be approved and rendered safely as a worker rule.
The socket E2E for restarts, dirty-worktree preservation, batching, owner approval
and next-start revocation compiled but could not run in this sandbox.
No owner server/ticker or agent data was consulted; no agent CLI was run.
No push was performed.
