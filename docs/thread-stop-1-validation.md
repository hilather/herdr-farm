# THREAD-STOP-1 validation (2026-10-03)

Implemented `thread stop PROJECT ID [--reason TEXT]` with an execution-bound,
durable TOML journal. Final copy precedes graceful shutdown; agent disappearance
is verified before pane close, and local worktree process references are checked.
Only an empty, exclusively owned worktree workspace is closed. Branches, worktrees,
dirty files and records remain. Submitted terminal actions are observed on retry
rather than blindly repeated. Stopped workers can restart or resolve, are shown
as Stopped, and are excluded from parallel capacity and automatic cleanup.

Files: `src/threads/stop.rs` implements shutdown; `src/thread.rs`, `src/threads.rs`,
`src/cli.rs` and `src/doctor.rs` expose lifecycle and display changes;
`src/migration/{mod,evidence}.rs` and `src/store/{runtime,control}.rs` accept
quiesced stopped history. Existing JSON/TOML payloads carry the new fields;
no database schema, tables, crates, retention or backup classifications changed.
`docs/operations.md` and `skill/COORDINATOR.md` document the coordinator command.
New coverage is E2E in `tests/threads.rs` and `tests/migration.rs`, using isolated
projects, real Git and fake Herdr. No new unit or source-text tests.

## Checks

- `cargo build --locked --offline -j 3`: passed (default features).
- `cargo check --locked --offline -j 3 --features state-store`: passed.
- `cargo clippy --locked --offline -j 3 --features state-store --all-targets`: passed;
  existing warnings remain, none in changed lines.
- `cargo test --locked --offline -j 3 --features state-store --no-fail-fast
  --test threads --test cleanup --test artifacts --test recovery --test overview
  --test migration -- --test-threads=1`: 12 passed, 30 blocked at Unix socket bind.
  Migration: 9 passed; artifacts: 1 passed; overview: 2 passed.
- An earlier parallel migration run transiently failed the existing
  `used_legacy_evidence_migrates_and_restores_without_live_routes` test with
  “source or mapping changed since plan”; the serial suite passed.
- `git diff --check`: passed. No repo-wide formatting, owner services, owner
  agent directories or real agent CLIs were used.

## Socket-only failures

All tests below fail in fixture setup at `UnixListener::bind` with
`Operation not permitted`, before exercising the workflow. The steward must run
these outside this sandbox with nextest, particularly the two new stop tests.

### `tests/artifacts.rs`

- `a_brief_waits_for_a_pending_live_copy`
- `live_copies_get_a_receipt_and_review_notice_each_and_a_new_execution_is_announced_again`
- `merged_finalization_keeps_what_each_source_shape_proves`
- `remote_resolve_needs_the_helper_and_streams_exact_bytes_once`
- `resolve_preserves_each_version_once_and_a_lost_source_keeps_the_last_snapshot`

### `tests/cleanup.rs`

- `canonical_references_keep_the_worktree_but_an_unrelated_binding_does_not`
- `canonical_references_made_after_removal_block_the_reopen`
- `reopen_restores_the_removed_worktree_unless_its_branch_path_or_owner_changed`

### `tests/overview.rs`

- `overview_prints_groups_in_display_order_and_names_the_pane_that_needs_you`
- `unfocus_reads_herdr_stdout_reports_its_stderr_on_failure_and_survives_a_missing_binary`
- `unfocus_sends_one_line_over_one_connection_and_reads_one_reply_line`

### `tests/recovery.rs`

- `a_declined_toast_is_retried_after_restart_only_once_its_backoff_is_due`
- `a_gh_outage_outlasts_restarts_and_gives_one_item_each_way`
- `each_toast_claims_the_sorted_unseen_items_and_leaves_out_seen_and_handled_ones`
- `merged_finalization_retries_failed_copies_and_is_withdrawn_by_reopen_or_a_new_report`
- `remote_merged_finalization_waits_for_the_helper_and_an_explicit_resolve`

### `tests/threads.rs`

- `integrated_resolution_enforces_policy_readiness_and_git_ancestry`
- `prompt_refuses_a_bare_shell_a_blocked_or_unknown_agent_and_sends_otherwise`
- `report_review_ack_and_resolve_copy_home`
- `reprime_updates_only_the_priming_fields_of_the_coordinator_record`
- `resolved_done_thread_cleans_its_pane_and_worktree_but_keeps_branch_and_report`
- `restart_follows_what_the_record_reached`
- `start_restart_and_adopt_write_briefs_branches_and_launch_line`
- `stop_blocked_worker_preserves_work_and_recovers_crash`
- `stop_failed_placement_without_artifacts_and_restart`
- `thread_list_groups_every_record_and_live_state`
- `thread_start_uses_agent_arguments_only_for_the_kind_they_are_bound_to`
- `ticker_crash_after_pane_close_resumes_without_repeating_it`
- `unsafe_resolved_threads_are_kept_and_cleanup_keep_opts_out`
- `used_quiesced_project_migrates_after_final_copy_and_memory_record`
