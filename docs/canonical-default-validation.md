# CANONICAL-DEFAULT-1 validation (2026-10-03)

Default builds include `state-store`. CLI `new` and the plugin new-project action
share canonical creation at the final path. The skeleton is paused and marked
`.creating` before PROJECT.md is published. Creation uses project ownership and
a shared root barrier, never the ticker singleton lock or exclusive root
maintenance. Only the four-file empty skeleton is accepted by this path; existing
legacy migration retains its original global locking protocol.

The existing migration journal/advance implementation publishes the schema,
format, active journal, config pin and paused control. Creation then sets up
owner-signed memory when the automatic signer is available. The creation marker is
excluded from provenance and removed only after publication. Listing and doctor
report creating projects; ticker scans and open leave them inert. Interrupted
creation is explicitly removed and recreated after confirming no creator remains.

New canonical projects install the same owner-signed `sqlite-v1` memory policy
as empty-project adoption before publication, without a ticker quiet window.
`MEMORY.md` remains the owner’s index and `memory PROJECT record` works immediately.
If the automatic owner signer is unavailable, `new` retains `legacy-markdown`
and prints the later `memory PROJECT adopt` command. Existing projects and
`new --legacy` retain their current behavior; adoption of existing projects still
requires its normal maintenance barrier.

`new --legacy` retains legacy creation.

First-run owner setup generates Ed25519 approval keys through gated ssh-keygen,
sets private/public modes to 0600/0644, and appends authority and available starter
profiles without rewriting existing config bytes. Config files created by setup
use 0600. Starter profiles omit model and effort, which profile validation accepts.
Existing authority tables and existing profiles are preserved.

Changed implementation: Cargo.toml; src/project.rs, migration/mod.rs,
owner_setup.rs, lib.rs, cli.rs, actions.rs and doctor.rs. Coverage: CLI workflow
in tests/cli.rs; historical CLI/migration lab helpers in cli,
canonical_coordinator, operator_launch and reconcile explicitly request legacy
creation. Documentation: README, getting-started, operator-runbook section 0,
migration-workflow, renaming, and all affected state-store build recipes.
No schema changes or dependencies were added. CI already explicitly enables the
feature and needs no adjustment.

## Checks

- `cargo nextest run --locked --offline -j 3 --no-fail-fast --test cli --test canonical_coordinator --test operator_launch --test reconcile`: **69 passed, 33 failed, 1 skipped**. The canonical creation workflow passed, including a running ticker, canonical context/runtime, authority policy, key modes, byte-preserved config, legacy behavior and interrupted creation.
- `cargo clippy --locked --offline -j 3 --features state-store --all-targets`: passed; existing warnings remain, with no diagnostics on changed lines.
- `cargo build --locked --offline -j 3 --no-default-features`: passed.
- `cargo check --locked --offline -j 3 --no-default-features --tests`: passed (feature-gated test compilation).
- `git diff --check`: passed.

The initial suite run overlapped a no-default-feature binary build, and a queued
focused rebuild overlapped the next run. Their binary replacement failures are
superseded by the final sequential run above. The final run had no non-socket
failures. No socket restrictions were bypassed. These workflows still need the
steward's outside-sandbox execution.

## Explicit socket-bind failures

These tests reported `Operation not permitted` while binding their isolated Unix
socket fixtures:

- `canonical_coordinator::socket_open_primes_owned_coordinator_retries_swallowed_prompt_and_recreates_closed_pane`
- `canonical_coordinator::socket_coordinator_remote_manifest_and_explicit_local_override_policy`
- `cli::canonical_ownership_cli_adopts_recorded_coordinator_without_prompting`
- `cli::controller_captures_uncommitted_worker_edits_for_submission_and_verification`
- `cli::hot_paths_skip_the_whole_store_check_and_the_ticker_checks_off_its_pass_then_pauses_admission_and_effects_on_corruption`
- `cli::integration_releases_project_ownership_during_the_candidate_check`
- `cli::launch_reserve_records_operator_reason`
- `cli::native_ticker_claims_legacy_routine_and_restart_delivers_without_rerun`
- `cli::operator_verify_releases_project_ownership_during_the_check`
- `cli::outcome_success_path`
- `cli::rejected_reservation_writes_no_decision`
- `cli::ticker_auto_chain_releases_verified_integrated_and_fan_in_dependents`
- `cli::ticker_auto_integrates_two_results_serially_and_recovers_stale_and_crash`
- `cli::ticker_auto_verification_releases_project_ownership_during_the_check`
- `cli::ticker_auto_verifies_once_and_recovers_after_kill`
- `cli::ticker_canonical_notification_confirms_or_retains_ambiguity_after_owner_death`
- `cli::ticker_canonical_observations_commit_cancel_and_restart_in_the_shared_pool`
- `cli::ticker_coordinator_prime_confirms_or_recovers_once_across_restart`
- `cli::ticker_coordinator_start_then_prime_recover_without_replaying_start`
- `cli::ticker_local_and_remote_launches_acknowledge_once_and_recover_lost_replies`
- `cli::ticker_native_briefs_confirm_or_recover_uncertainty_without_replay`
- `cli::ticker_native_copy_publishes_announces_and_does_not_recopy_after_restart`
- `cli::ticker_native_merged_finalization_resolves_and_replays_notice_after_restart`
- `cli::ticker_notifications_recover_across_restart_and_reconcile_through_cli`
- `cli::ticker_remote_briefs_confirm_or_recover_uncertainty_without_replay`
- `cli::ticker_tokens_use_supervised_local_remote_and_coordinator_refreshes_after_restart`
- `reconcile::reconcile_records_pane_evidence_from_what_each_session_lists`
- `reconcile::reconcile_records_worktree_evidence_only_from_complete_git_listings`
- `reconcile::recovery_plans_advise_without_retrying_ambiguous_effects_or_releasing_lost_capacity`

## Operator fixture server exits

These tests reported `native probe server exited` at verification. Running the
same fixture's server entry point in a disposable temporary directory confirmed
its `socket.bind` raises `PermissionError: [Errno 1] Operation not permitted` in
this sandbox:

- `operator_launch::launch_run_with_a_dedicated_server_after_verify_interaction_reserves_both_kinds`
- `operator_launch::launch_run_retries_after_termination_but_refuses_an_unobserved_live_worker`
- `operator_launch::verify_interaction_produces_launchable_evidence_for_codex_and_claude_from_the_cli`
- `operator_launch::verify_interaction_tolerates_agents_writing_into_their_execution_home`

## CANONICAL-DEFAULT-1b fixture compatibility

Legacy runtime and migration fixtures explicitly create projects with
`new --legacy`, including labs that subsequently pause and migrate their own
projects. Paired creation/lifecycle setup now passes the flag only to creation.
The existing CLI, coordinator, operator-launch and reconcile helpers already
select legacy creation. Library and binary test fixtures using `project::create`
retain its legacy semantics and require no change.

The projects CLI workflow preserves all legacy skeleton, settings, slug,
duplicate and escape assertions, and also verifies that default `new` publishes
canonical storage, paused context and an inspectable runtime while the explicit
legacy skeleton has no canonical database. The existing comprehensive default
creation workflow in `tests/cli.rs` continues to use unmodified CLI arguments.
No runtime code, schemas, dependencies or assertions were removed.

Validation uses `TMPDIR=$PWD/target/tmp` for every Cargo command:

- `cargo nextest run --locked --offline -j 3 --no-run`: passed, all binaries built.
- `cargo clippy --locked --offline -j 3 --features state-store --all-targets`: passed; pre-existing warnings, no diagnostic locations on changed lines.
- Focused final run (`projects`, `migration`, `memory_regressions`, `satisfaction`, `plans`, `scheduling`): **48 passed, 7 socket-only failures**. All seven scheduling tests fail at their fixture's Unix socket bind.
- Separate final `projects` run: **6 passed**.
- The two binary tests named by the steward: **2 passed**.
- Replay/telemetry support consumers (`replay_suite`, `telemetry_operations`, `telemetry_routines`): **22 passed, 2 socket-only failures**, both replay worker fixture servers denied `AF_UNIX` binding.

The first focused run exposed a new canonical-context assertion using the legacy
output format. It was corrected to assert SQLite context and persisted paused
control through `runtime PROJECT inspect`; both final runs above pass that test.

- Broad affected-suite run: **235 passed, 150 socket-only failures, 9 skipped**
  across 47 binaries (385 executed tests). This includes all changed integration
  binaries plus the existing CLI, coordinator, operator-launch and reconcile
  suites. Live-service tests remain ignored.

All **152 distinct remaining failures** across these runs are listed as
`binary::test` in [canonical-default-1b-socket-failures.txt](canonical-default-1b-socket-failures.txt).
Direct socket binds report `Operation not permitted`; Python fixture-server
failures show the same error before downstream readiness timeouts. Four
operator-launch failures report only `native probe server exited` because
production probing discards child stderr. Executing that exact local Herdr
stand-in's server branch in a disposable lab confirmed its `AF_UNIX` bind exits
with `PermissionError: [Errno 1] Operation not permitted`. No sandbox workaround
was attempted. These workflows require the steward's unrestricted rerun.
There are no remaining real failures in the final runs.

## NEW-MEMORY-DEFAULT-1 validation (2026-10-07)

Canonical creation now signs the empty memory adoption plan and cutover policy
with the automatic owner signer before removing `.creating`. It uses the existing
project barrier, memory policy, backup and recovery receipts; no schema changes
or root ticker lock are needed. Signer discovery failure preserves Markdown
memory and prints the later adoption command. Existing adoption retains its
maintenance barrier.

The CLI workflow in `tests/projects.rs` verifies SQLite authority and its policy,
doctor output, immediate owner recording, retained index bytes, creation under a
held ticker lock, missing-signer fallback, later adoption and unchanged legacy
creation. All seven project tests passed. Memory barriers (6), read sets (4) and
regressions (15) passed. Memory control passed 12 initially; the remaining test,
`proposal_review_requires_signed_exact_scope_and_promotion_rechecks_authority`,
expired its fixture's 60-second authorization under load and passed its isolated
rerun. The full operator suite passed 25 of 35, including both existing-memory
adoption tests and canonical owner recording.

All cargo commands used `TMPDIR=$PWD/target/tmp`, `nice -n 19 ionice -c 3`,
`--locked --offline -j 3`. State-store clippy with `--all-targets` completed
successfully with existing warnings and no warnings on changed lines. No
repo-wide formatting, live services, owner agent data or canonical schema were
changed.

Socket-limited operator tests requiring a run outside this hard sandbox:

- `canonical_worker_viewers_create_reopen_focus_and_close_only_the_recorded_tab`
- `code_launch_passes_only_selected_toolchain_environment_to_the_worker`
- `launch_run_retries_after_termination_but_refuses_an_unobserved_live_worker`
- `launch_run_with_a_dedicated_server_after_verify_interaction_reserves_both_kinds`
- `stale_profile_evidence_is_refreshed_by_launch_run`
- `verify_interaction_produces_launchable_evidence_for_codex_and_claude_from_the_cli`
- `verify_interaction_tolerates_agents_writing_into_their_execution_home`

The viewer and termination tests directly report `Operation not permitted` at
Unix-listener binding. The five profile-probe tests report `native probe server
exited`; their local Herdr fixture binds a Unix listener before serving probes.
These were not worked around.

Three additional operator failures in the full concurrent run passed on isolated
reruns, with no code or fixture changes:

- `contracted_launch_starts_at_an_unchecked_out_base_and_captures_only_worker_changes`:
  worktree Git execution failure/output-budget refusal.
- `code_launch_reserves_under_concurrent_writes_and_submits_all_scoped_changes`:
  transient root execution-lock contention at profile evidence.
- `launch_run_reacknowledges_an_owner_configuration_edited_since_control_was_activated`:
  a matching observation had become stale.

Thus all non-socket checks passed, including isolated reruns of the load-sensitive
failures. The seven socket-limited workflows remain for the steward outside the
sandbox; the new memory defaults and existing-memory adoption paths are verified.
