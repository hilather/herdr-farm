# RESULT-IMPORT-1 validation (2026-10-07)

The checkout CLI reuses verification's materializer and digest-checked retained
objects in `.state/factory-objects/staging/<sha256(submission key)>/<byte digest>`.
Worker quarantine objects are imported into the shared repository before staging;
checkout never reads quarantine configuration or refs. The trusted base is fetched
locally as in verification. The destination is a fresh detached repository, and
source refs and checkout are untouched. No canonical schema change or new crate.

Changed implementation: `src/cli.rs`, `src/store/verification.rs`,
`src/verification/{mod.rs,checkout.rs}`. Guidance: `src/canonical_coordinator.rs`,
`skill/COORDINATOR.md`, `docs/factory/verified-results.md`. Coverage: `tests/cli.rs`.

The new CLI workflow passed: exact candidate OID and contents, repeatable submission
and attempt selection, absent and empty destinations, nonempty destination refusal,
unknown submission refusal, worker-context refusal, source hook and smudge filter
suppression, UTF-8 report printing, oversized report refusal and symlink refusal.
Review document paths are declared submission outputs exposed by `show --attempt`
and checkout's JSON; coordinators can read them from the materialized candidate.
Host execution remains untrusted and uses operator privileges. LFS pointers are
not expanded, external filters are not configured, and submodules are not initialized.

All cargo commands used `TMPDIR=$PWD/target/tmp`, `nice -n 19 ionice -c 3`,
`--locked --offline -j 3`, and `--features state-store`.

- `cargo clippy --all-targets`: completed successfully; existing warnings remain,
  none point to changed lines.
- `cargo test --lib store::results`: 5 passed.
- `cargo test --lib verification`: 20 passed, 1 socket-only failure, 2 live tests ignored.
- `cargo test --test cli`: 66 passed, 25 failed, 1 live test ignored. Of the failures,
  24 were denied socket binds. `ticker_replaces_result_jobs_bound_to_an_older_task_revision`
  also observed a timing race (a job was claimed before its initial observation).
- `cargo test --test cli result_`: 3 passed, 1 live test ignored. This rerun includes
  the new workflow, the retained submission/verification/integration workflow and
  the ticker timing test, which passed without changes.

The sandbox prevented the following Unix-socket tests from exercising their workflows;
all failed at a bind with `Operation not permitted`, except `outcome_success_path`,
whose Python fixture server logged the same bind denial before its socket wait timed out.

- `canonical_ownership_cli_adopts_recorded_coordinator_without_prompting`
- `controller_captures_uncommitted_worker_edits_for_submission_and_verification`
- `hot_paths_skip_the_whole_store_check_and_the_ticker_checks_off_its_pass_then_pauses_admission_and_effects_on_corruption`
- `launch_reserve_records_operator_reason`
- `native_ticker_claims_legacy_routine_and_restart_delivers_without_rerun`
- `outcome_success_path`
- `rejected_reservation_writes_no_decision`
- `ticker_canonical_notification_confirms_or_retains_ambiguity_after_owner_death`
- `ticker_canonical_observations_commit_cancel_and_restart_in_the_shared_pool`
- `ticker_coordinator_prime_confirms_or_recovers_once_across_restart`
- `ticker_coordinator_start_then_prime_recover_without_replaying_start`
- `ticker_local_and_remote_launches_acknowledge_once_and_recover_lost_replies`
- `ticker_native_briefs_confirm_or_recover_uncertainty_without_replay`
- `ticker_native_copy_publishes_announces_and_does_not_recopy_after_restart`
- `ticker_native_merged_finalization_resolves_and_replays_notice_after_restart`
- `ticker_notifications_recover_across_restart_and_reconcile_through_cli`
- `ticker_remote_briefs_confirm_or_recover_uncertainty_without_replay`
- `ticker_tokens_use_supervised_local_remote_and_coordinator_refreshes_after_restart`
- `profile_preparation::tests::launch_ingress::worktree_verification_and_stop_select_only_their_launch_provenance` (verification filter)

The sandbox also denied these TCP-socket binds with `Operation not permitted`:

- `integration_releases_project_ownership_during_the_candidate_check`
- `operator_verify_releases_project_ownership_during_the_check`
- `ticker_auto_chain_releases_verified_integrated_and_fan_in_dependents`
- `ticker_auto_integrates_two_results_serially_and_recovers_stale_and_crash`
- `ticker_auto_verification_releases_project_ownership_during_the_check`
- `ticker_auto_verifies_once_and_recovers_after_kill`

The three checkout-root-sensitive `copy_jobs::tests` named in the task were not
run: this task does not change copy jobs. No sandbox socket workaround was used.
Live tests remained ignored; no owner project, server, ticker or agent data was accessed.
Integration policy and ownership guidance were preserved. No push was performed.
