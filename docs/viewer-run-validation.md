# VIEWER-RUN-1 validation

Validated on 2026-10-07 in the hard sandbox on `factory/viewer-run`.
No canonical schema changes, new crates, unit coverage, source-text assertions,
owner data access, or live-server checks were introduced.

The viewer submits one POSIX shell source argument after its pane ID, without
`--`. Its first word is quoted absolute `/usr/bin/env`, followed by the private
config assignment, worker socket assignment, and Herdr executable. Startup
observation uses up to three visible pane reads under one two-second deadline
and 64 KiB capture per read. Recognized shell execution errors follow existing
viewer cleanup and nonfatal `viewer.status = unavailable` reporting. Unsupported
reads retain the viewer; this cannot guarantee attachment or catch later errors.

The existing operator-launch CLI lifecycle E2E now asserts the exact submitted
source (including absence of a leading separator) and a silent submission with a
shell error, successful worker launch, and removal of persisted viewer ownership.
That workflow needs an outside-sandbox run to exercise its new assertions.

Commands used (each with `TMPDIR=$PWD/target/tmp`, after creating it):

```sh
nice -n 19 ionice -c 3 cargo test --locked --offline -j 3 --features state-store --test operator_launch
nice -n 19 ionice -c 3 cargo test --locked --offline -j 3 --features state-store --test canonical_worker viewer
nice -n 19 ionice -c 3 cargo clippy --locked --offline -j 3 --features state-store --all-targets
```

- Operator launch: 27 passed, 8 failed. Socket-backed fixtures are blocked below.
- Canonical worker filter: 1 passed, 1 failed, 54 filtered out. There are no
  dedicated viewer tests in that integration target; `viewer` matches two
  reviewer workflows. `reviewer_worker_submits_proposal_receipt_but_cannot_accept`
  passed.
- Clippy: exit 0; existing warnings outside changed files, none in changed lines.
- `git diff --check`: passed.

Socket-restricted operator tests:

- `canonical_worker_viewers_create_reopen_focus_and_close_only_the_recorded_tab`
- `code_launch_passes_only_selected_toolchain_environment_to_the_worker`
- `launch_run_retries_after_termination_but_refuses_an_unobserved_live_worker`
- `launch_run_with_a_dedicated_server_after_verify_interaction_reserves_both_kinds`
- `stale_profile_evidence_is_refreshed_by_launch_run`
- `verify_interaction_produces_launchable_evidence_for_codex_and_claude_from_the_cli`
- `verify_interaction_tolerates_agents_writing_into_their_execution_home`
- `launch_run_records_reviews_and_skeptical_yield_and_refuses_without_writes`

The viewer lifecycle and launch retry tests fail directly at
`UnixListener::bind` with `Operation not permitted`. The remaining six report
`native probe server exited` when starting the same Python fake Herdr whose
server entry point binds a Unix socket. These are consistent with the sandbox
socket prohibition; their captured CLI diagnostics do not include the underlying
bind exception. No socket workaround was attempted.

Canonical-worker socket-only failure:

- `a_sandboxed_reviewer_uses_its_worker_channel_through_the_spool`: Python fixture
  prints `PermissionError: [Errno 1] Operation not permitted` at `server.bind`,
  followed by the fixture startup deadline assertion.

Raw logs remain in `target/viewer-operator-tests.log`,
`target/viewer-canonical-tests.log`, and `target/viewer-clippy.log` (untracked build
artifacts). The specified `copy_jobs::tests` environmental git-tree failures were
not exercised by these integration suites or clippy.
