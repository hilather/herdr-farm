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
| Worker idle notice threshold | 300 s (HERDR_FARM_WORKER_IDLE_SECS: 1–3600) | Scale; at least the native-call quiet window plus two nominal passes (16.5 s at test scale), since idle is sampled on resting passes only |

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
  Brief readiness uses a retry-scaled two-minute safety margin, capped at one
  quarter of the original signed approval window. The approval expiry itself
  remains unscaled. An unready worker observed inside that margin fails once as
  `brief_not_ready`; an already expired approval keeps `brief_approval_expired`.
  Readiness labs therefore use an explicit short signed validity (60 seconds),
  not an assumed scaled approval lifetime.
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
rather than `ci` because its owner-host runtime can reach about 13 minutes
(~50 resting passes at the unscaled 15.5 s advisory window). Its isolated worker
uses a 3600 s wall budget so the default 600 s limit cannot end it mid-workflow.
The working-to-idle rearm waits for the idle monitor’s logged reset transition. Run it
with `cargo nextest run --features state-store --profile nightly`. Its dedup
window observes 30 completed passes in one ticker via the published metrics `tick`
counter, avoiding rename-publication inode reuse and missed bursts. The counter
increases within each ticker process; the test reads a fresh baseline after a
restart. Older metrics files without `tick` deserialize with zero. Nightly allows
20 × 60 seconds rather than 15 × 60: the ~13-minute slow-pass workflow otherwise
has only ~2 minutes of margin, despite its worker's 3600-second budget.
Additional quiet tickers must not be inserted before the one-notice assertion: a restart intentionally starts
a new idle stretch, so enough elapsed native observation time can emit a second
notice. The explicit restart and working-to-idle rearming assertions remain.

TEST-FLAKE-10b sandbox validation: lib 529 passed, 84 Unix-socket permission
failures, 13 ignored; clippy completed with no warnings on changed lines. The
focused workflow (both cargo test and the nextest nightly profile) stops at
`launch demo draft` with `state store: Conflict` on
both this follow-up and its unchanged parent f9b54ed. Main e240dc8 instead
reaches the sandbox-denied socket bind, including with the same scratch
location. The conflict therefore predates the pass-counter change, but is not
confirmed sandbox-only; the complete nightly workflow still needs validation.

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

The public-store E2E
`recorded_delivery_is_visible_to_independent_connections_before_writer_drops`
records a claimed delivery with the same 20 ms receipt deadline, then checks
the outcome from an already-open reader and a fresh connection before dropping
the writer. It also checks the persisted outcome event and reopening after
writer shutdown. The test passes locally; it does not establish the cause of
the CI owner-death fixture's stale Claimed reads. The record path uses an
immediate transaction and commits before returning; runtime snapshots open a
fresh connection. The Python helper performs a SELECT without an explicit
BEGIN or a write, so keeping that connection open alone does not pin a read
transaction across delivery.

With delivery tracing enabled, outcome recording also reports the SQLite
version, database path and device/inode, autocommit state after commit, and a
delivery read from a separate read-only connection while the writer remains
alive. These observations distinguish writer-local visibility from visibility
through a fresh connection at the exact recording boundary. They are disabled
by default and do not change receipt success or retry decisions.

## Load-driven inventory expiry (MAINT-IO-INTERRUPTED-2)

Observation heads, controller hints and routine admission share an unscaled
short-read allowance, initially 500 ms. Inventory deadline expiry doubles it
for subsequent reads up to 2 seconds. Caller deadlines and the 10-second
inventory ceiling still win; no in-flight budget is renewed. The allowance
remains monotonic for the process lifetime: a fast cached read does not prove
that the next cold schema load or scheduling delay will be fast. The cap bounds
the cost, and ticker restart resets it. This avoids oscillation under mixed
projects and load.

Inventory expiry and cancellation retain typed Deadline/Cancelled causes with
reader, project, record and byte context. SQLite progress interrupts retry inside
the original budget; an unattributed store interrupt is typed Interrupted.
Routine admission retries without submitting work or entering failure cooldown.
Observation expiry leaves liveness unknown until a fresh matching head succeeds.
The existing indexed head query and pass-scoped read-only connection reuse are
preserved. Public inventory tests exercise expiry, cancellation, in-query
interruption/recovery and 20 cold reads of an isolated 55,000-event copied store.

On 2026-10-13, the synthetic copy measured 70.424 ms median and 113.533 ms
maximum during concurrent low-priority compilation (20 fresh head reads).
After compilation, the final code measured 7.996 ms median and 26.039 ms maximum,
below the initial 500 ms allowance. These are isolated synthetic-history
measurements, not reads of an owner's store.

TERMINATION-CONTENTION-2: exit-recording lock waits are 30 seconds in production,
scaled with a 250 ms floor in labs; poll intervals have the usual 20 ms scaled
floor. Live exit watches and contention retries use the existing 15-second
recovery cadence. Escalation is three deferrals or two minutes (scaled), with a
stable persisted notice and volatile counters. New coverage in
`tests/canonical_worker.rs` holds the root barrier across live observations,
actual process exit and repeated maintenance bursts, and checks escalation in
inbox, health and attempt views. `tests/termination_contention.rs` is socket-free:
a real `inbox done` waits through a 350 ms maintenance hold and an unrelated
store event, then persists exactly the requested acknowledgement. Its explicit
three-second lock-wait override is literal; every CLI process still uses the
shared lab scale. Existing transfer fences and ROOT-LOCK-FAIRNESS maintenance scheduling remain.
Exit-watch labs retain their ticker across the held lock so inventory is already
available, and use 10–20 second safety deadlines with scaled product waits.
Retained exit watches remain admissible while maintenance observations are
pending; their recording phase still uses the bounded exclusive fence. Failed
attempts reach the stop path even when their supervised process remains live,
preserving the expired-brief ownership-release workflow.

Sandbox validation: lib 529 passed, 84 socket-only failures, 13 ignored;
canonical-worker 13 passed and 67 socket-only failures; ticker 1 passed and
4 socket-only failures; ticker-jobs 11 socket-only failures; ticker-timing
5 passed. The socket-free acknowledgement passed with a measured 362 ms
maintenance hold and 392 ms total acknowledgement latency. The new exit-watch
assertions require the steward's outside-sandbox run; fixture socket creation
was denied before those assertions. Clippy completed with no warnings in
changed lines; existing warnings elsewhere remain.

## Automatic health delivery isolation

Ticker labs whose workflows assert exact event, inbox or prompt sets set
`[telemetry] health_notify = 'off'` in their temporary owner's config. This avoids
health evaluation timing injecting unrelated notices; assertions remain exact.
The telemetry health episode and coordinator critical-notice wake workflows
retain the default policy. See `telemetry/contracts-health.md` for deployment
triggers and the no-bound-usage coverage caveat.

## Launch lock waits and telemetry completion

Operator launches queue root-exclusive stages through `.execution-turn.lock`.
Shared stages release partial acquisitions while retrying, so a queued writer
cannot keep a launch waiting beyond its remaining `--budget-seconds` deadline.
Expiry returns typed `OperatorLockContention`. No root-wide lock spans the
whole launch; different projects and same-project launches can overlap between
exclusive stages. Profile preparation retains exclusion of running checks.
Capacity admission and refusal are unchanged.

Telemetry starts asynchronous collection on the first eligible pass after
controller services, at idle CPU and I/O priority. Collection reads at most
8 MiB per turn, resumes persisted cursors, and batches up to 2,000 lines per
transaction. Subsequent collection intervals retain the native-call quiet
window (15.5 seconds in accelerated labs). The recorded-source/new-sidecar
workflow allows 60 seconds for idle-worker completion while retaining exact
usage totals, paused operating intervals, and usage gate assertions.

Spool CLI replay admits sidecar writers for at most one unscaled second per
attempt, without a canonical transaction or runtime mutation guard. Unresolved
SQLite contention retries every 30 seconds via `timing::retry`; health warns at
120 seconds and becomes critical at 600 seconds through the same scaling helper.
The append-only worker report survives failures. Owner retry evidence persists
across ticker restarts, so restarting does not reset the log suppression or age.

## Resting worker attention and stop requests

Resting passes query the existing retained-attempt partial index before reading
worker attention snapshots. Projects with no running, unterminated attempt clear
their volatile idle stretches and skip the snapshot, including paused projects.
A running worker still receives the existing advisory observation behavior.
Ticker project boundaries check the stop file during discovery, canonical
controller work, advisory admission, attention and telemetry discovery. A stop
CLI timeout leaves its request in place for the running ticker to observe;
successful stop or a subsequent start retains the existing cleanup behavior.
Controller intervals, native observation windows and executor draining are
unchanged.

The telemetry workspace live-fleet fixture keeps its synthetic attention lane
fresh with a one-day sample interval. Its samples remain 30 seconds apart and
the open wait remains exactly 360 seconds, measured between sample timestamps.
Workspace reads do not extrapolate that duration to wall time, but attention
reads censor the open interval once the last sample is older than twice its
recorded interval. The production 30-second interval left only 50 seconds for
all surface comparisons and could remove the expected wait under host load.
The larger fixture interval avoids that unrelated freshness deadline without
weakening any duration or surface assertions or changing production timing.

The `heavy` nextest group (one test at a time) includes the lib tests
`admission::tests::admission_resumes_after_a_full_page_of_unsigned_candidates`,
`memory::regression_tests::concurrent_revision_reference_and_collection_never_lose_committed_bytes`,
and `canonical_worker::tests::launch_advancement_recovers_each_boundary_then_delivers_brief_and_stops`
in default, fast and ci to reduce contention under parallel load. Admission retains
its test-only 30-second setup helpers and product 2-second admission deadline
(`src/admission.rs`); the 20-iteration memory race has no test deadline and retains
the product 250 ms SQLite busy bound (`src/store/mod.rs::BUSY_RETRY_BOUND`).
Launch recovery raises its test-only shared deadline from 30 to 45 seconds, matching
the controller operation budget (`src/canonical_controller.rs`, also documented
in `docs/canonical-worker-launch.md`); all assertions and product timing remain.
