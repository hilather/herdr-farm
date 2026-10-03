# Test ticker timing

`src/timing.rs` is the shared process timing source for the CLI, ticker, and
public store delivery retries. `HERDR_FARM_TEST_TIME_SCALE=0.02` changes a nominal
15-second pass to 300 ms. A `OnceLock` reads only the actual process environment;
configuration files and injected `Ctx` environments cannot activate it. Factors
must be finite, positive, and no greater than one; other values are ignored.
Unset values return the original durations exactly. Production doctor output
has no additional line; an enabled override prints `test time scale active`.

Every isolated test command environment uses `tests/support/time-scale.txt`, so
CLI writes, foreground tickers, and detached tickers inherit the same factor.
No owner roots, running services, or native agent CLIs are needed. The new timing
acceptance suite uses real CLI processes and persisted ticker metrics/logs,
without sockets. It checks three bounded passes, the minimum pass floor, invalid
values, doctor diagnostics, normal draining, and the unset 15-second cadence.

## Policies scaled

| Policy | Production default | Rule |
| --- | --- | --- |
| Nominal ticker pass | 15 s | Scale; minimum 50 ms |
| Canonical work pass | 250 ms | Scale; minimum 50 ms |
| Stop wait / telemetry drain wait | 60 s | Scale; minimum 50 ms |
| Idle exit | 300 s | Scale; minimum 50 ms |
| Lock acquisition retry window | 1 s | Scale; minimum 250 ms so transient status/start probes can release their OS lock |
| Lock poll | 25 ms | Scale; minimum 20 ms |
| Stop poll | 250 ms | Scale; minimum 50 ms |
| Main loop wake/spool poll | 500 ms | Scale; minimum 20 ms |
| Telemetry shutdown polls | 50 ms | Scale; minimum 20 ms |
| Copy/background queue retention | 180 s | Scale; minimum 50 ms |
| Canonical launch queue retry | 1 s | Scale; minimum 20 ms |
| Canonical worker queue retry | 2 s | Scale; minimum 20 ms |
| Other job failures, notification/token refresh retry | 30 s | Scale; minimum 20 ms |
| Worker termination/recovery successful recheck | 15 s | Scale; minimum 20 ms |
| Routine cooldown retention / retry | 120 s / 30 s | Scale; minimum 20 ms |
| Verification admission cooldown | 30 s | Scale; minimum 20 ms |
| Legacy delivery retry | Exponential 15–300 s with deterministic jitter | Scale generated deadline; minimum 20 ms |
| Canonical store delivery retry | Exponential 1–256 s (300 s cap) | Scale generated deadline; minimum 20 ms |
| Worker startup / not-ready / blocked debounce | 300 s / 60 s / 30 s | Scale, round up to whole seconds |
| PR polling interval | 120 s | Scale, round up to whole seconds |
| Remote polling / retry | 60 s / 120 s | Scale; 50 ms / 20 ms floors |
| Local report, local/canonical observation freshness | 60 s | Scale, bounded below by unscaled execution budget + nominal pass |
| PR consumed observation cooldown / untouched pending retention | 120 s / 120 s | Scale cooldown with 50 ms floor; pending retention bounded below by 30 s execution budget + nominal pass |
| Remote queued observation freshness | 60 s | Scale, bounded below by 30 s execution budget + nominal pass |
| Worker brief acknowledgement window / poll | 6 s / 500 ms | Scale; 50 ms / 20 ms floors |
| Telemetry scan/operating slots | Nominal pass | Derive slots using nanoseconds, safe below one second |
| Telemetry collection / attention interval | 300 s (or configured collection interval) | Scale consistently in collection scheduling and stored attention sample interval |
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
Historical timestamp-based state uses whole-second comparisons, so its scaled
threshold rounds upward rather than becoming zero. No schema or stored payload
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
