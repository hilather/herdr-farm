# PROBE-READINESS-1 validation

The native readiness wait retries incomplete/empty command identities and
vanishing child reads, while complete executable/argument mismatches remain
fatal. Readiness uses the caller/configured deadline (at most 120 seconds),
with ten seconds reserved for acknowledgment, final checks and cleanup. Screen
diagnostics use fixed categories and preserve the last useful readiness
observation when a request expires at the boundary.

Failure reports use existing immutable canonical `profile.native_failed` events:
4 KiB maximum per record, latest 32 returned by profile/telemetry inspection.
No canonical or sidecar schema change, new crate, or automatic launch retry was
needed. The longer wait handles cold startup without a second launch under load.

Changed files: `src/profile_preparation/native.rs`,
`src/worker_supervision.rs`, `src/worker_supervision/observation.rs`,
`src/store/native_profiles.rs`, `src/cli.rs`,
`src/telemetry/{codex.rs,collectors/mod.rs}`, `tests/profiles.rs`, the C/Python
fixtures in `tests/fixtures/probe_readiness_*`, `docs/profiles.md`, and
`docs/telemetry/contracts.md`.

All Cargo invocations used `TMPDIR=$PWD/target/tmp` and
`nice -n 19 ionice -c 3 cargo ... --locked --offline -j 3`.

- `cargo check --features state-store`: passed; existing unrelated warnings.
- `cargo test --features state-store --lib profile_preparation`: 13 passed,
  15 socket-only failures, 3 ignored live-agent tests; rerun gave the same result.
- `cargo test --features state-store --test profiles`: 7 passed, 6 socket-only
  failures. Setup-failure retention and the existing failure/persistence workflows passed.
- `cargo clippy --features state-store --all-targets`: passed; existing warnings
  remain, with no warnings on changed lines. The new test helper's unnecessary
  unwrap was replaced with an explicit match.
- `git diff --check`: passed.
- C fixture compiled with `cc -Wall -Wextra`. Direct `/proc` observation confirmed
  an empty title and a 4096-byte non-NUL-terminated title, followed by restoration
  of the exact original command. This does not replace socket-dependent E2E checks.

## Socket-only failures

The following library tests all fail at the existing Unix listener bind in
`src/profile_preparation/tests/launch_ingress.rs:25`, with `Operation not permitted`.
No sandbox workaround was attempted.

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

The following CLI E2E tests fail only because the fixture's Unix socket bind
returns `Operation not permitted`:

- `native_readiness_accepts_cold_start_beyond_thirty_seconds`
- `native_readiness_rejects_changed_arguments`
- `native_readiness_rejects_changed_executable`
- `native_readiness_retries_empty_process_title`
- `native_readiness_retries_non_terminated_process_title`
- `native_readiness_timeout_retains_categories_visible_in_cli`

Fixture login sharing is disabled, so no owner credentials are used. The steward
must run these native workflows outside the hard sandbox; their readiness and
categorized timeout behavior cannot be verified here. No live agent CLI, owner
project/server/ticker, or paid/live external service was used. The known
`copy_jobs::tests` cases were not part of these focused suites and were unchanged.
