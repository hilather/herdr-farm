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

## Policies scaled

| Policy | Production default | Rule |
| --- | --- | --- |
| Nominal ticker pass | 15 s | Shared cadence factor; 500 ms at test scale |
| Admitted canonical sidebar batch quiet window | 15 s | Preserve unscaled 15 s native-call window plus scaled pass (15.5 s at test scale); production remains 15 s |
| Canonical work pass | 250 ms | Scale; minimum 50 ms |
| Stop wait / telemetry drain wait | 60 s | Scale; minimum 50 ms |
| Idle exit | 300 s | Scale; minimum 50 ms |
| Lock acquisition retry window | 1 s | Scale; minimum 250 ms so transient status/start probes can release their OS lock |
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
