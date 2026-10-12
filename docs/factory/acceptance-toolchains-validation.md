# ACCEPT-CMD-1 validation

Owner toolchains are resolved from the external migration-pinned config. Launch
adds a signed toolchain digest; manual contracts use `result PROJECT
toolchain-policy`. The verifier pins file hashes and metadata, exposes read-only
tools and dependencies, and runs checkout-relative commands with a cleared
environment. Network namespaces, a read-only root, dropped capabilities and
`no_new_privs` protect the host. Repository-local ignored tools are also exposed
in the disposable checkout. Ignored output remains allowed by the existing
Git clean-tree check. Toolchain timeouts cover verification and integration;
claim bounds and job budgets cover their maximum durations. Existing signed
policy text and verification metadata hold the evidence, so no storage schema
or retention/backup classification changes were needed.

Changed code: `src/verification/{toolchains,setup,supervise,repetitions,mod}.rs`,
`src/domain/verification_policy.rs`, `src/worker_supervision.rs`,
`src/{cli,launch_run,canonical_verification_jobs,canonical_integration_jobs}.rs`,
`src/store/{results,delivery,integration}.rs`, and `src/integration/mod.rs`.
Coverage: `tests/factory_harness.rs` and `tests/operator_launch.rs`.
Operator docs: `docs/operator-runbook.md`, `docs/factory/verified-results.md`,
and `skill/COORDINATOR.md`.

All cargo commands used `TMPDIR=$PWD/target/tmp`, `--locked --offline -j 3`.
The complete run used:

```sh
TMPDIR=$PWD/target/tmp cargo nextest run --locked --offline -j 3 \
  --features state-store --test canonical_worker --test factory_harness \
  --test operator_launch --lib \
  -E 'binary(canonical_worker) | binary(factory_harness) | binary(operator_launch) | test(/^verification::tests::/)'
```

| Suite | Passed | Socket restriction failures |
|---|---:|---:|
| canonical_worker | 6 | 32 |
| factory_harness | 20 | 0 |
| operator_launch | 14 | 6 |
| Existing verification tests | 12 | 0 |
| Total | 52 | 38 |

The 613 unrelated library tests were excluded by the filter. The new acceptance
workflow passes through signed contract install, public result submission, real
namespace verification and integration. It covers shell exit 0/1, ignored caches,
tracked/unignored changes, owner secret and root/tool writes, dropped mount
privileges, separate network namespace/no routes, missing toolchains, toolchain
changes before install and after signing, timeout enforcement, identity evidence,
and scratch/checkouts under `/tmp`. The extended CLI workflow checks repeated
`--accept`, quoted arguments and the manual policy preparation command. No new
unit tests, source-text assertions or crates were added.

After the final Clippy cleanup, the two acceptance/launch workflows and all
12 verification tests were rerun on the final source: **14 passed**. Command:

```sh
TMPDIR=$PWD/target/tmp cargo nextest run --locked --offline -j 3 \
  --features state-store --test factory_harness --test operator_launch --lib \
  -E 'test(owner_toolchain_acceptance) | test(code_launch_reserves_under_concurrent) | test(/^verification::tests::/)'
```

Clippy command:

```sh
TMPDIR=$PWD/target/tmp cargo clippy --locked --offline -j 3 \
  --features state-store --all-targets
```

Clippy completes successfully. Existing warnings remain outside changed lines;
added and modified lines have no diagnostics. `git diff --check` passes.

The following tests could not complete in this hard sandbox. Unix socket binds
return `Operation not permitted`. Four operator interaction tests wrap the fixture
server's bind failure as `native probe server exited` (marked below). No socket
restriction was bypassed. These workflows need the steward's outside-sandbox run.

## canonical_worker

- `a_brief_swallowed_by_the_agent_is_redelivered_and_confirmed_only_once_accepted`.
- `a_brief_the_agent_never_accepts_is_left_ambiguous_not_confirmed`.
- `a_hidden_path_covering_the_execution_home_refuses_the_launch_before_creation`.
- `a_launch_reaches_running_while_another_holder_takes_the_shared_root_intermittently`.
- `a_legacy_thread_holding_the_planned_worktree_blocks_its_creation`.
- `a_proven_worker_end_keeps_the_project_admitted_but_an_unexplained_pane_loss_pauses_it`.
- `a_sandboxed_reviewer_uses_its_worker_channel_through_the_spool`.
- `a_subdirectory_binding_runs_in_the_same_subdirectory_of_the_new_worktree`.
- `a_worker_branch_reaching_a_corrupt_quarantined_object_is_refused`.
- `accepted_editing_worker_completes_automatically_after_integration`.
- `accepted_verify_only_editing_worker_completes_without_integration_automation`.
- `an_isolated_codex_worker_commits_through_codex_workspace_write_sandbox`.
- `an_isolated_worker_cannot_read_owner_secrets_or_lift_the_hiding_but_still_commits_and_submits`.
- `an_isolated_worker_submits_only_through_its_own_spool`.
- `an_operator_finishes_a_worker_that_never_submitted_and_the_result_lands_automatically`.
- `an_untracked_working_directory_is_refused_before_the_approval_is_used`.
- `canonical_attempt_sidebar_clears_after_termination_in_an_active_project`.
- `canonical_attempt_sidebar_does_not_publish_to_a_replaced_terminal`.
- `canonical_attempt_sidebar_refreshes_and_clears_on_pause_and_termination`.
- `canonical_attempt_sidebar_restart_offers_no_historical_cleanup_or_native_request`.
- `canonical_attempt_sidebar_uses_collected_usage_and_observed_waiting`.
- `dedicated_worker_refuses_remote_manifest_before_its_brief`.
- `editing_worker_requires_operator_completion_when_automation_is_off`.
- `launch_sets_the_intended_permission_mode_over_a_stale_one_in_the_home`.
- `rejected_editing_worker_stays_running_and_can_resubmit`.
- `review_assignment_launches_with_blind_brief_and_records_session`.
- `ticker_does_not_dispatch_a_launch_cancelled_before_creation`.
- `ticker_launches_and_briefs_once_then_stops_a_cancelled_worker_while_paused_and_revoked`.
- `ticker_launches_nothing_on_a_server_without_the_launch_contract_or_while_paused`.
- `ticker_recovers_a_lost_creation_reply_without_creating_again`.
- `ticker_retires_a_cancelled_gated_worker_without_starting_it`.
- `ticker_stops_the_dedicated_herdr_server_of_a_finished_task`.

## operator_launch

- `canonical_worker_viewers_create_reopen_focus_and_close_only_the_recorded_tab`.
- `launch_run_retries_after_termination_but_refuses_an_unobserved_live_worker`.
- `launch_run_with_a_dedicated_server_after_verify_interaction_reserves_both_kinds` — native probe server exited.
- `stale_profile_evidence_is_refreshed_by_launch_run` — native probe server exited.
- `verify_interaction_produces_launchable_evidence_for_codex_and_claude_from_the_cli` — native probe server exited.
- `verify_interaction_tolerates_agents_writing_into_their_execution_home` — native probe server exited.
