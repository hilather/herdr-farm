# Test ticker timing

`src/timing.rs` is the shared process timing source for the CLI, ticker, and
public store delivery retries. `HERDR_FARM_TEST_TIME_SCALE=0.02` changes a nominal
15-second pass to 500 ms. A `OnceLock` reads only the actual process environment;
configuration files and injected `Ctx` environments cannot activate it. Factors
must be finite, positive, and no greater than one; other values are ignored.
Unset values return the original durations exactly. Production doctor output
has no additional line; an enabled override prints `test time scale active`.

Accelerated isolated test command environments use `tests/support/time-scale.txt`, so
CLI writes, foreground tickers, and detached tickers inherit the same factor.
Pure projection labs with synthetic nominal timestamps omit the override on every
CLI/ticker process in that lab. No owner roots, running services, or native agent
CLIs are needed. The new timing
acceptance suite uses real CLI processes and persisted ticker metrics/logs,
without sockets. It checks three bounded passes, the minimum pass floor, invalid
values, doctor diagnostics, normal draining, and the unset 15-second cadence.

## Compiled agent fixtures

`tests/support/agent_build_cache.rs` caches Rust stand-ins in
`CARGO_TARGET_TMPDIR/agents/<digest>/agent`, keyed by the exact source bytes and
compiler flags. A per-entry OS file lock serializes builds across integration
test processes; failed or interrupted builds never publish an executable.
Each lab receives a separate `fs::copy` with mode `0700` at its original agent
path, preserving per-lab profile freezing and replacement workflows. The copy
is renamed into place so replacing a running stand-in remains possible.
Lab-specific constants remain part of the source and therefore get distinct
cache entries. No test assertions or production configuration change.

The cache persists across test runs. Remove `target/tmp/agents` (or the
`agents` directory under a custom Cargo target's temporary directory) after
changing Rust toolchains to rebuild fixtures. To measure cold-cache compiler
counts, clear that directory before the run and count test-time `rustc`
invocations separately from Cargo's crate builds; a warm-cache run should
compile only previously unseen source/flag combinations.

## Policies scaled

| Policy | Production default | Rule |
| --- | --- | --- |
| Nominal ticker pass | 15 s | Shared cadence factor; 500 ms at test scale |
| Admitted canonical sidebar batch quiet window | 15 s | Preserve unscaled 15 s native-call window plus scaled pass (15.5 s at test scale); production remains 15 s |
| Canonical work pass | 250 ms | Scale; minimum 50 ms |
| Stop wait / telemetry drain wait | 60 s | Scale; minimum 50 ms |
| Idle exit | 300 s | Scale; minimum 50 ms |
| Lock acquisition retry window | 1 s | Scale; minimum 250 ms so transient status/start probes can release their OS lock |
| Interactive project lock wait / poll | 30 s / 100 ms | Retry only typed would-block lock contention; wall-clock timing is unscaled. `HERDR_FARM_LOCK_WAIT_SECS` overrides the bound (`0` disables waiting); a notice appears after 2 s. Partial ownership is released before sleeping. |
| Lock poll | 25 ms | Scale; minimum 20 ms |
| Stop poll | 250 ms | Scale; minimum 50 ms |
| Main loop wake/spool poll | 500 ms | Scale; minimum 20 ms |
| Telemetry shutdown polls | 50 ms | Scale; minimum 20 ms |
| Copy/background queue retention | 180 s | Scale; minimum 50 ms |
| Canonical launch queue retry | 1 s | Share canonical 250 ms pass factor/floor; 200 ms at test scale |
| Canonical worker queue retry | 2 s | Share canonical 250 ms pass factor/floor; 400 ms at test scale |
| Other job failures, notification/token refresh retry | 30 s | Shared cadence factor; 1 s at test scale |
| Worker termination/recovery successful recheck | 15 s | Shared cadence factor; 500 ms at test scale |
| Routine cooldown retention / retry | 120 s / 30 s | Shared cadence factor; 4 s / 1 s at test scale |
| Verification admission cooldown | 30 s | Shared cadence factor; 1 s at test scale |
| Legacy delivery retry | Exponential 15–300 s with deterministic jitter | Shared cadence factor |
| Canonical store delivery retry | Exponential 1–256 s (300 s cap) | Shared retry factor, minimum first retry 20 ms; exponential ratios retained |
| Worker startup / not-ready / blocked debounce | 300 s / 60 s / 30 s | Shared cadence factor; 10 s / 2 s / 1 s at test scale |
| PR polling interval | 120 s | Shared cadence factor; 4 s at test scale |
| Remote polling / retry | 60 s / 120 s | Shared cadence factor; 2 s / 4 s at test scale |
| Local report, local/canonical observation freshness | 60 s | Scale, bounded below by unscaled execution budget + nominal pass |
| PR consumed observation cooldown / untouched pending retention | 120 s / 120 s | Shared cadence cooldown factor; pending retention bounded below by 30 s execution budget + nominal pass |
| Remote queued observation freshness | 60 s | Scale, bounded below by 30 s execution budget + nominal pass |
| Worker brief acknowledgement window / poll | 6 s / 500 ms | Scale; 50 ms / 20 ms floors |
| Telemetry scan/operating slots | Nominal pass | Derive slots using nanoseconds, safe below one second |
| Telemetry collection / attention interval | 300 s (or configured collection interval) | Scale consistently in collection scheduling and stored attention sample interval; both retain at least the native-call quiet window (15.5 s at test scale) |
| Analytics / health refresh interval | 60 s / 300 s | Scale elapsed-time eligibility |
| Worker idle notice threshold | 600 s | Scale; at least the native-call quiet window plus two nominal passes (16.5 s at test scale), since idle is sampled on resting passes only |

Coordinator priming, launch, and legacy brief/notification delivery advance on
scaled passes and queue retries. Their external Herdr API acknowledgement call
budgets stay unchanged. Test quiet assertions count completed metrics publications
rather than wall-clock seconds. The bounded foreground option `ticker run
--passes N` requires an active override, records completion in the ticker log,
and drains normally. Bounded runs finish their requested passes even in an idle
root; ordinary runs retain idle exit and their existing log messages.

Persisted retry deadlines are actual UTC timestamps, produced by the scaled
backoff and compared directly to actual UTC time. No fake clock or timestamp
rewriting is used. Restarting a lab with the same factor preserves eligibility.
The main cadence family uses `ceil(scale * 30) / 30` as its common factor:
blocked debounce is the smallest whole-second member (30 s -> at least 1 s).
Main passes, background cooldowns, remote polling/retry, PR polling/retention,
and startup/not-ready/blocked comparisons keep their production ratios. At
0.02 this yields 500 ms < 1 s < 2 s < 4 s < 10 s. Canonical pass/launch/worker
retries share a minimum factor of 0.2 (50 ms < 200 ms < 400 ms). Store delivery
backoffs share a minimum factor of 0.02, preserving 20 ms < 40 ms even at a
smaller requested scale. Unset durations remain exact.

Canonical sidebar calls are unscaled external work. A newly admitted advisory
batch receives the production 15 s quiet window plus a scaled pass before the
next observation batch can preempt it. Executor cancellation and priority stay
unchanged; canonical effects still use their fast passes. Collection scheduling
and stored attention sample intervals share the same minimum quiet window, so
real sidebar passes do not fabricate observation gaps while waiting on native
calls. Test observation
deadlines retain their original generous wall-clock bounds. No schema or stored payload
shape changes are required; retention/backup classifications are unchanged.

## Policies deliberately unchanged

- External command timeouts: git, gh, ssh, rsync, Herdr calls/bridge acknowledgement
  budgets, native worker invocations, profile preparation and source-tree reads.
- Executor execution budgets and process kill/cleanup grace periods; shortening
  them would change whether real external work succeeds.
- Canonical operation claim leases (including 30 s brief, 60 s notification,
  and 300 s integration/verification/finalization leases): these fence external work
  governed by unscaled budgets. They already exceed a nominal pass at every
  accepted factor. Observation freshness leases are scaled with the budget floor
  described above; an execution lease is not a retry delay.
- Owner-signed approval validity, launch reservation/authority deadlines, budget
  wall limits, routine schedules, persisted owner-specified deadlines, token
  metadata TTL sent to Herdr, and worker receipt validation freshness.
- Owner outage/integrity intervals and integrity execution budgets, SQLite busy
  waits, telemetry writer budgets/jitter, maintenance retention and backup ages,
  inbox done retention, reminder cooldowns, and interactive watch refresh periods.
- Fixed sleeps used to test external process cancellation/escape prevention.

The scale is an opt-in lab contract: do not mix scaled processes with unscaled
processes on a root, or apply it to an owner's existing persisted deadlines.

## Test scheduling

`.config/nextest.toml` keeps the default pool at six test threads. The
`canonical_worker`, `operator_launch`, `worker_login_share`, `replay_suite`, and
`thread_sandbox` binaries share a `sandbox` group capped at three concurrent
tests in the default, fast, and CI profiles. Profile filters still determine
which tests run; the group only limits concurrency. The telemetry scale group
retains its separate single-thread limit.

GitHub Actions distributes the CI selection across eight `hash:N/8` partitions.
Each runner uploads its JUnit file as `junit-N`; sum the testcase `time`
attributes in each file to compare test-seconds across partitions. These sums
measure test work, while runner wall time also includes scheduling and setup.

## Validation and timings

See [sandbox validation](test-tick-validation.md) for commands, every group timing,
baseline comparisons, and the exact socket-only failure list. The socket-free
telemetry query group dropped from 66.43 s to 11.15 s (16 tests passing); CLI
went from 159.37 s to 57.67 s with the same socket failures. Successful socket
workflows still require the steward's outside-sandbox cargo-nextest run.

W-COORD-2c extends `canonical_coordinator`'s public CLI replacement workflow:
with the accelerated ticker running, `open demo --reprime` must finish within
five seconds. Replacement no longer tries to acquire shared root ownership
inside its own exclusive root guard. Coordinator native calls and acceptance
polling retain project ownership with a shared root; exclusive root ownership
is limited to binding publication and local conflict inspection. The short
publication acquisition uses the normal job retry deadline (and the existing
bounded exclusive acquisition window). No background canonical coordinator
replay loop is involved. Socket-bound validation requires an unrestricted lab;
this sandbox cannot bind Unix sockets.

Legacy ticker labs must create projects with `new --legacy` (or the public
`project::create` API). The default `new` creates a paused canonical project;
legacy tickers cannot exercise their intended workflows on that project.
Canonical labs originally written to migrate legacy state retain explicit
legacy creation before pausing and applying their migration.

## Canonical service fairness (EFFECT-LOCK-FAIR-1)

Main-pass services retain their nonblocking project guard acquisition and order.
An actual `effect.lock` contention failure reserves a service opportunity before
that project's next canonical observation batch. Existing batches drain first;
maintenance admission then skips at most three otherwise eligible opportunities.
A pass without service contention clears the reservation. Root-exclusive effect
passes defer services as before and neither clear nor consume the reservation.
Thus recurring maintenance cannot win every service opportunity, while persistent
external contention cannot indefinitely suppress maintenance. There are no new
locks, guard waits, schema changes, or sidecar migrations.

Service contention is logged separately by project and identical service error,
at most once per real minute, with `count=N` failures since the previous line.
The first failure emits immediately. Every failure increments M92 before log
suppression; the minute interval is not accelerated in labs.
The new `canonical_worker` lab uses two running attempts, recurring observations,
and public project guard ownership to force contention, waits for the unscaled
minute summary to check accumulated counts and suppressed log lines,
cancels one attempt within the existing 60-second lab budget, and checks that
observations continue while the other worker remains running. Socket-bound
validation still requires the steward's unrestricted environment.

INVENTORY-BUDGET-1d keeps the coordinator replacement fixture at 350 retained
launches: three counted records per launch still exceed the old 1,024 candidate
cap. Its lab stop retries the public `ticker stop` command for up to 30 seconds,
since a stop requested mid-pass can exceed the scaled CLI stop wait. Production
stop timing and resting-pass observation remain unchanged; budgeting the full
worker-attention snapshot is follow-up work.

## Canonical result wake labs (COORD-WAKE-ON-SUBMIT-1)

The worker result-wake setup retries the public `ticker stop` for up to 30 real
seconds, with 50 ms between unsuccessful calls, matching the coordinator lab
helper (LAB-STOP-WAIT-1). A native call admitted before the stop request can
outlast the scaled CLI wait (60 s becomes 1.2 s at the shared 0.02 scale).
The helper still requires a successful stop before reserving the worker;
all subsequent workflow assertions remain unchanged. Product timing is unchanged.

The public stop call-site audit leaves the other sites unchanged: the
coordinator helper already retries; coordinator and legacy ticker Drop handlers
are best-effort cleanup; the four legacy ticker lifecycle stops check stale
requests and replacement without native work; the two `ticker_timing` stops
exercise production timing and a socket-free accelerated lab; `live_phase_a`
and its support Drop handler belong to opt-in live acceptance. This accounts
for the eleven other literal public stop sites, including `tests/support/live.rs`.
Foreground stop-file helpers are separate from the public stop retry.

Canonical result wakes use the shared claim retry/backoff timing and the existing
executor. Native calls retain their unscaled budgets; a wake job has a 45-second
real deadline, and its admission uses the canonical work pass while queued.
The normal healthy ticker target is one 15-second pass plus that budget (60 s)
after idle is observable. Contention, missing readiness and native failures may
exceed it; a working coordinator is never intentionally interrupted. The worker
E2E asserts the real 60-second bound after setting the fake native coordinator
idle, and asserts only one prompt across repeated submissions and ticker restarts.
All lab CLI and ticker processes use `tests/support/time-scale.txt`; no owner
process timing or project configuration is changed. Socket fixtures still need
the steward's unsandboxed run. Context marks results SEEN but only DONE consumes
the worker-result wake; the socket-free lost-attempt CLI workflow verifies this.

Canonical launch phases additionally preserve their real execution budget:
initial claims and live advancement use at least the remaining unscaled phase
deadline plus five seconds for receipt persistence. The configured launch lease
(180 s production; 30 s accelerated floor; explicit override 1–300 s) remains
the minimum between phases. Renewal never revives an expired claim. This prevents
a short lab lease from truncating a valid native phase; crashed launchers still
enter observation at expiry. See `docs/canonical-worker-launch.md` for evidence
and E2E coverage of slow creation and killed-launcher recovery.

Expired unreleased targets select termination observation alone. It records the
typed expiry outcome, then stops the gated worker in the same reconciliation;
it never renews or reclaims the expired launch.

The idle-worker multi-stretch E2E workflow runs in nextest's `nightly` profile
rather than `ci` because its owner-host runtime is about seven minutes. Run it
with `cargo nextest run --features state-store --profile nightly`. Its dedup
window observes 30 completed passes in one ticker. Additional quiet tickers must
not be inserted before the one-notice assertion: a restart intentionally starts
a new idle stretch, so enough elapsed native observation time can emit a second
notice. The explicit restart and working-to-idle rearming assertions remain.

The canonical notification owner-death fixture retains a present pane and matching
working agent, adopts the coordinator through the public CLI, then records fresh
post-adoption identity evidence before activation. An unrecorded route provides
no reachable session to refresh the accelerated ticker idle-exit deadline; a
present route without adoption cannot authorize activation. Helper exceptions,
ticker stderr and ticker logs remain in failed-wait diagnostics. The fixture also
records each helper invocation and dumps public operation inspection, the runtime
snapshot, notification authority validation, the controller dispatch hint, effect
pause reason, lock availability and executor metrics on a failed wait. The dump
includes the wall clock so persisted due and lease times can be compared directly.
Helper stage timestamps distinguish request reads, nonblocking lock checks,
SQLite claim reads, effect recording and reply buffering/flushing. The same CLI
workflow also confirms a helper that reads the request to EOF, and requires a
helper stalled after effect recording to become ambiguous within 30 seconds and
before its captured claim expiry. These are regression checks for the existing
10-second command cap and cleanup allowance, not evidence that the intermittent
CI delivery stall can be reproduced in this sandbox. Outcome persistence retries
busy and I/O failures without repeating delivery, retaining ownership within the
claim lease and worker control deadline. The public store/dispatch E2E workflow
in `tests/operation_recording.rs` holds a competing SQLite writer beyond one busy
attempt and checks both successful recording and the original control deadline
after delivery. Queue drain forwards runner errors, but depends on a later ticker
pass, and log writes previously ignored errors. Neither explains the original CI
log gap conclusively. Failed completions are now logged at executor completion as
well as queue drain; log write failures go to stderr.
Failure dumps include operation events and delivery/executor samples at `sent`
and two seconds later. Lock probes release each guard before the next probe, and
success waits fail explicitly on ambiguous outcomes or expired claims.
Unix socket restrictions prevent exercising this fixture past socket creation in
the sandbox.

Root safety work (recovery, termination and briefs) precedes launch advancement
across projects, including while safety work backs off. Queue fairness promotes
jobs after three failed turns or one recovery observation interval of waiting.
The root writer turn prevents fresh shared probes from overtaking acquisition.
Cancellation and completion requests offer termination immediately, independently
of passive launch-observation hints. Requested stops rotate among themselves
before other project-local effects; their workers still validate retained launch
and ownership evidence before stopping anything. The socket-free cancellation
workflow covers a Running attempt whose passive monitor inventory is not yet
populated, including admission beside another project's reserved launch and
retention of capacity when exact worker evidence is missing. The shared-root
workflow also checks stop hints before starting its
ticker, then retains its refused-commit and cross-project launch assertions.
Native creation requests retain their unscaled 15-second external budget. On
timeout, observation and cancellation run within the remaining 45-second phase
and five-second lease reserve; labs do not scale that execution budget. The
supervisor preserves its public exit-200 failure contract and exposes timeout
through the output's `timed_out` flag, including when its surviving helper
finishes before the collector deadline. The
shared-root recovery E2E uses 30 completed passes, and the hung-creation E2E
uses observable terminal state with a generous 90-second wait deadline.

Root fairness recovery preserves socket observation errors before considering
remaining native observation time. A short lease alone does not count as a
native timeout. Pending-termination labs keep the ticker alive through the persisted
termination and next launch after removing journal deferral. Completed passes
alone do not imply executor completion.
The journal-deferral trigger raises a SQLite constraint violation, exposed by
the store as `Conflict`; the lab waits for that queue failure tied to the exact
attempt. It checks the post-cancellation hint revision and requires the same
ticker to commit termination after trigger removal before the next launch.

LAB-REQUESTS-TORN-1: canonical worker, operator viewer, and replay request-log
readers only parse newline-terminated JSON records. Polling during an append
ignores the unfinished final record until its newline arrives; malformed complete
records still fail the harness. Product behavior and workflow assertions are unchanged.

The owner-death fixture enables temporary stderr diagnostics with
`HERDR_FARM_TRACE_DELIVERY=1`. They show prepare, claim, delivery budgets and
command completion, each receipt attempt, the dispatch result, and executor
completion/reply delivery. Tracing is off by default. Cancellation permits one
receipt attempt bounded to 20 ms and the original control/claim deadline, so an
in-flight cancelled effect can persist Ambiguous. Receipt retries check
cancellation before each attempt and stop immediately after a failed attempt
when cancellation is active; they never repeat the external effect.
