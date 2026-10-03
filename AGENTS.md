# Agent instructions

<!-- agent-skills-pause:start -->
## Agent-skills pause

Do not load, follow, or consult `~/git/agent-skills` (`HINTS.md`, `knowledge/`, or its skills). That repository is paused for Codex. Do not run `codex-workflows`.
<!-- agent-skills-pause:end -->

## Tests

Use only end-to-end tests for new or replacement test coverage going forward.
Exercise real user workflows through public entry points and assert observable
results and persisted state. Use isolated temporary projects and deterministic
local external-service fixtures when needed; do not require live or paid services.

Do not add unit tests, source-text assertions, documentation phrase checks, or
tests that only assert values constructed by their own fixtures. Extend an
existing end-to-end workflow when it covers the behavior adequately.

Existing useful focused tests may still be run. Do not delete coverage merely
because it is not end-to-end; replace its useful guarantees with end-to-end
coverage before removing it.

## Accelerated ticker test labs

`HERDR_FARM_TEST_TIME_SCALE` is **test-only**. Set it on every CLI/ticker
process in an isolated temporary lab, using the shared value in
`tests/support/time-scale.txt` (currently `0.02`). Never set it for an owner's
root, server, or ticker. It is read once from the actual process environment;
project configuration cannot enable it. Unset or invalid values preserve the
production timing. Accepted values are finite factors greater than zero and at
most one. `doctor` warns `test time scale active` when enabled.

Controller passes/observation intervals have a 50 ms floor, retries/polling a
20 ms floor (the lock acquisition window has a 250 ms floor to outlast
transient status/start probes). Historical second-resolution comparisons round upward to whole
seconds. Freshness leases retain enough time for the unscaled external command
budget plus a pass. Processes sharing a lab must use the same scale, including
CLI commands producing persisted retry deadlines. `ticker run --passes N`
requires an active scale and exits after N completed passes with normal executor
draining. Prefer pass counts or observable state to fixed sleeps. Wait deadlines
should retain a generous safety margin; they are not external execution budgets.

The exact scaled and unscaled policies and sandbox test results are recorded in
`docs/test-tick-timing.md`. New coverage remains E2E through public entry points;
verify defaults there rather than adding unit or source-text tests.
