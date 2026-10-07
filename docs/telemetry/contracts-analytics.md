# Analytics contracts: registry, query service and aggregate revisions (TM4.1)

Plan card TM4.1 (doc 12), doc 07 §1–§7, doc 08 §4, doc 10 §3–§4. Common rules
are [contracts.md](contracts.md) §0. Code: `src/telemetry/analytics/`
(`registry.rs`, `query.rs`, `lifecycle.rs`, `store.rs`); sidecar stream
`analytics`, migrations under `migrations/telemetry/analytics/`. This is the
**stable read contract** shared by the workspace UI (TM4.2) and exports
(TM4.3). Telemetry never grants launch, changes budgets or accepts results;
nothing here writes `state.db`.

## 1. Metric registry (`analytics-registry.v12`)

Version history: v12 adds MET-VERIFY-1 M100–M103 (section 13). v11 adds MET-COORD-1 M80–M87 (contracts-accounting.md; M88–M89 reserved) and MET-LAUNCH-1 M90–M95 (section 12; M96–M99 reserved). v10 adds MET-REWORK-1 M65–M69 (section 10) and MET-WORKER-1 M70–M77; M78–M79 are reserved pending definitions. v8 adds bounded lifecycle `role` and M37.lineage-v2 (LINEAGE-1); M37.fleet-v1 remains absent as `definition_superseded`. v1 TM4.1; v2 adds `verification_flip_rate` (DG6, #198); v3 adds M30 `M30.submission-v1` (DG1, #202); v4 adds M10 `M10.v1` (DG2, #204); v5 adds M03 operating throughput (DG3); v6 adds M05 lifecycle tokens and specifies M19’s missing timed history; v7 adds M13 never-running exclusions, M04/M05 partial values and M31 lifecycle sampling, M35 attempt-time concurrency and M40 reported/merged quota windows, plus the bounded lifecycle `profile` dimension (TFIX-4, no definition change). Previous M13.slice-v1, M04.cost-v1, M05.tokens-v1, M31.attention-v1, M35.fanout-v1 and M40.quota-windows-v1 definitions remain only as absent (`definition_superseded`). v9 adds MET-NOW-A `M51.v1`–`M58.v1` and MET-NOW-B `M60`–`M64` (sections 8 and 9).

`telemetry <slug> metrics registry [--json]` prints one declared table
(`registry.rs`) of every metric `telemetry report` or `query` can name:
M01–M58, M60–M69, M70–M77, M80–M87, M90–M95, M100–M103 and lane C's `flaky_tests` and `verification_flip_rate`. A change is a new registry version, never
an edit in place of a published definition. Per metric:

| field | meaning |
|---|---|
| `id`, `name`, `unit` | doc 07 identity; units are explicit (`ratio` values are the unreduced string `"n/d"`) |
| `definition` | the current definition `query` serves by default |
| `versions[]` | every servable definition: `provider` (`native`, `central_report`, `lane` + stream, `absent` + reason), `cohorts` (first = default), `window` (`half_open` or `since_only`), `time_basis`, `dimensions` |
| `family` | `lifecycle`, `consumption`, `cost`, `tools`, `attention`, `fleet`, `services`, `review_quality`, `paired_quality`, `seeded_quality`, `proxy`, `replay`, `freshness`; `proxy: true` for the proxy family, which never stands in for a validated-quality metric |
| `certification` | `{status, evidence, restriction}` from [certificate-core.md](certificate-core.md) §5 and [certificate-quality.md](certificate-quality.md) §2: `certified-live`, `certified-fixture`, `restricted`, `unavailable`, `fixture` (TM4.1's own fixtures only) or `absent` (no producer) |
| `active`, `activation` | families activate independently: lifecycle/accounting/operational at TM2.6 (core certificate); proxy at TM3.7; validated quality at TM3.5 (`QUALITY_CERTIFICATE`, now `certificate-quality.md`, a fixture certificate: `activation.production` names the producer certificate production quality still waits for); replay (M49) at TM4.6 (fixture suite v1, [contracts-replay.md](contracts-replay.md)); freshness (M50) active at TM4.5, evaluated per recommendation only (below). Without the quality certificate the quality families answer `unavailable: awaiting_quality_certificate` |

Native definitions added by TM4.1 (the certified `slice-v1` ones stay
servable by name): `M01.cohort-v1` accepted tasks, `M02.cohort-v1`
acceptance rate, `M06.cohort-v1` lead-time p95 (nearest rank, ms, accepted
tasks with both times; failed/open counted, never given a time),
`M07.cohort-v1` attempt amplification. Absent producer: M19 (`blocked_intervals_not_recorded`).
M49 (`M49.v1`, central provider) is produced by the replay suite ([contracts-replay.md](contracts-replay.md)).
Historical absent definitions for M03, M05, M19 and M30 remain servable by name.
M50's current definition `M50.recommendation-v1` (TM4.5,
[contracts-health.md](contracts-health.md) §5) has provider
`per_recommendation`: `query --metric M50` answers `unavailable:
per_recommendation` (the value exists only inside a `telemetry <slug>
recommend` answer) and analytics refresh never tracks it; `M50.v1` stays
servable by name as absent. `metrics registry --json` gains the additive key
`freshness` (definition and the `stale_below` threshold, `1/2`).

## 2. Query service (`analytics-query.v1`)

```
telemetry <slug> query --metric M..[,M..] [--cohort activity_window|terminal_cohort|assignment_cohort]
    [--from MS] [--to MS] [--as-of MS | --as-of-seq N] [--by DIM] [--horizon-ms MS]
    [--drill BUCKET --page-size N --cursor C] [--json]
```

Read-only (opens both stores strictly read-only, creates no file in the project; a multi-page drill-down creates the per-user cursor key under the config directory on first use, [contracts-export.md](contracts-export.md) §4). `--metric`
takes a registry id (current definition) or an explicit definition
(`M02.slice-v1`).

**Request rejections** (exit status 1, stderr `query rejected: {code, ...}`):
`ambiguous_cohort` for `completed_task` (never read as success-only; the
diagnostic lists the accepted enums), `unknown_cohort`, `unknown_metric`,
`unknown_definition` (with the known ones), `empty_window` (`from >= to`),
`page_size_out_of_range` (1–500), `horizon_out_of_range`,
`drill_needs_one_metric`, `invalid_cursor`, `cursor_expired`,
`cursor_foreign_project`, `cursor_revoked`, `cursor_key_unusable`,
`cursor_mismatch`, `restart_required` (cursor codes:
[contracts-export.md](contracts-export.md) §4).

**Per-metric diagnostics** (the result's `status: unavailable` with `reason`
and `diagnostic`): the family's inactive reason; the absent producer's reason;
`cohort_unsupported` (with `supported`); `window_end_unsupported` (a
`since_only` definition given `--to`); `horizon_unsupported`;
`high_cardinality_dimension` (`task_id`, `attempt_id`, `session_id`, ...: use
`--drill`); `dimension_unsupported` (with `supported`); `too_many_cells`
(more than 64); `no_revision_as_of`.

**Response** (`--json`): `{schema_version: 1, contract, request (normalized
echo), query_unix_ms, results: [...], drill?}`. Each result:

| field | meaning |
|---|---|
| `metric_id`, `name`, `definition`, `family`, `proxy`, `unit`, `registry` | registry identity |
| `cohort`, `time_basis`, `window {from_unix_ms, to_unix_ms, semantics}`, `horizon_ms`, `by` | the evaluated cell |
| `status` | `available`, `empty` (a ratio with an empty denominator: `value` null, `reason` `empty_denominator`/`no_samples`), `partial`, `unavailable` |
| `value`, `reason`, `numerator`, `denominator` | unknown is never 0 (§0) |
| `exclusions` | counts by reason (`open`, `outside_window`, `terminal_time_unknown`, `not_assigned`, `assignment_time_unknown`, or the lane's `excluded`) |
| `coverage` | `{state: complete|partial|unknown|unavailable, known, expected, missing, reasons}` for native definitions; a lane definition that does not state its expected population is `unknown` (never 100 %) with the lane's own `coverage` under `lane` |
| `breakdown`, `cells`, `censored`, `provisional`, lane `detail` | native outcome counts; per-dimension cells; assignment-cohort unfinished tasks; the full lane or report body for lane/central definitions (its keys are lane-native fields, never labels) |
| `projection` | live: `{mode: live, revision: null, matches_revision, content_digest}`; as-of: `{mode: revision, revision, kind, supersedes, superseded_by, current_revision, restated, recorded_unix_ms, content_digest}` |
| `source_watermarks` | `canonical {events_head, lifecycle_digest, last_event_unix_ms}`; for lane definitions also `sidecar {streams, last_collect_unix_ms, codex_usage_rowid, valuation, rate_cards}` |
| `event_cutoff_unix_ms`, `observation_cutoff_unix_ms` | latest occurrence time in the cohort; knowledge time (query time live, `recorded_unix_ms` as of) |
| `lag_ms`, `lag_reason` | native: 0 (canonical read directly); lane: observation cutoff − last collect, or null with `collection_not_run`/`no_collect_recorded` |
| `rate_card_revision` | priced metrics (M04, M12, M14, M24, M34, M37, M53, M66, M69, M94): `{valuation_revision, valuation_digest, rate_cards {count, digest}}` or unavailable (`not_priced`, `collection_not_run`); otherwise null |
| `certification`, `activation` | as the registry |
| `as_of` | the requested knowledge time or sequence, as-of results only |

Text form: one line per metric
`<id> <name> <definition> <cohort> <value> numerator=<n> denominator=<d> coverage=<state> <live|revision N>`,
dimension cells indented, then drill rows.

## 3. Cohorts and native lifecycle semantics

Plan doc 07 §1. `T`/`A` evidence is exactly contracts §6 (`metrics::task_evidence`).

- **`terminal_cohort`**: tasks with a terminal disposition (`accepted`,
  `succeeded_without_evidence`, `failed`, `cancelled`), their whole lifecycle
  included; succeeded, failed and cancelled tasks are all members. Terminal
  time: acceptance → when the evidence completed (verify_only: first verified
  result; integrate route: first integrated commit); otherwise the latest
  terminal lifecycle mark of its attempts (the canonical store keeps no task
  terminal time). A bounded window places a task by that time in `[from,
  to)`; a terminal task with no time is excluded as
  `terminal_time_unknown` and makes coverage `partial` (never dropped
  silently, never placed by guess). Open tasks are the `open` exclusion.
- **`assignment_cohort`**: tasks first assigned (earliest dispatch decision,
  else reservation mark) in the window, followed to `--horizon-ms` (or to
  now): an outcome after the horizon, or an open task, is `unfinished`
  (`censored.unfinished`, `provisional: true`), counted in the denominator.
  Tasks never assigned are `not_assigned`.
- **Replay candidates** (TM4.6 `replay_candidates`) are evaluation
  artefacts measured by M49: every native cohort excludes them first, as
  `replay_candidate` (in `exclusions` and the drill bucket
  `excluded.replay_candidate`, shown when non-zero). The central report's
  M02 `excluded` and M07 `excluded` carry `replay_candidate` always, and its
  `tasks` summary counts them as `replay_candidates` outside `T`.
- **`activity_window`**: events in the window; native M03/M30 and lane definitions
  (`since_only`: `--from` is the lane's `--since`).
- Dimensions for native definitions: `route`, `task_class` (latest
  classification, else `unclassified`), `agent_kind` (the attempts' effective
  profile kind, `mixed`, `unknown` or `unassigned`), and `profile` (effective
  profile name, with the same mixed/unknown/unassigned rules). At most 64 cells.

### Lifecycle consumption and blocked time

M05 (`M05.tokens-v2`, fixture certification) uses M04’s contracts §6 T/A
cohort and since-only window: a task qualifies when any attempt was decided
at or after `since`; every attempt of that task contributes its full lifecycle
input plus output tokens. Counted ledger normalization matches M08/M09;
reasoning is already in output and cache reads in input. Separate native
children follow attempt ownership, including guardians and subagents;
uncertified fork replay and unowned children stay excluded. Divide by count(A)
as an exact unreduced `tokens/tasks` ratio. An empty A gives null with
`empty_denominator`; any attempt missing counted usage gives
`value: {status: partial, reason: lifecycle_usage_incomplete, tokens,
denominator, attempts_without_usage}`, with per-reason coverage and the
observed numerator retained as a subtotal. Never-running attempts without
usage use M04’s exemption; bound usage still counts. Text explicitly labels
the subtotal partial and gives its denominator and missing-attempt count. `M05.v1` remains the historical
absent definition.

M19 (`M19.blocked-v1`; historical `M19.v1` remains absent) would divide
assignment-cohort task time spent blocked by total observed task time, censoring open tails and observation gaps like attention intervals.
It remains unavailable with `blocked_intervals_not_recorded`: canonical
`task.changed` events before 0072 retain state and sequence but no transition timestamp;
0072 records informational insertion times for new events only;
attempt lifecycle timestamps do not record task blocked entry/exit, and
attention samples describe agent activity, not task state. A future producer
needs durable timestamped task state transitions, a known initial state and
observation boundaries to censor gaps and open intervals. Current blocked
state or event order cannot establish elapsed blocked time.

### DG1: first-candidate independent verification (M30)

Registry v3 adds native `M30.submission-v1`; the historical absent `M30.v1`
remains explicitly servable as `no_producer`. This follows dictionary doc 07
M30: accepted first candidates / adjudicated first candidates, rather than
all submitted tasks. No minimum applies to the descriptive query/report ratio.

The cohort is `activity_window`, placed by the first submission's
`created_unix_ms` in `[from,to)` (`first_submission_time`). First means earliest
submission across every attempt and contract revision, with submission ID as
the deterministic timestamp tie-breaker. Retries never replace it; later
candidates neither supply its receipts nor remove an accepted first candidate.
Verification may occur after the submission window. Replay candidates stay
excluded. Required policies come from that submission's contract revision.
Every required policy must have an accepted independent verification run joined
to its `verified_results` receipt for that submission and policy digest. A
required policy rejection without subsequent acceptance adjudicates the first
candidate as rejected. Partial policy acceptance is pending; a contract with no
recorded policies is `policy_unknown`, never assumed accepted.

Pending and unknown-policy cases stay outside the denominator and appear in
`exclusions`, `pending` and partial coverage (`known` adjudicated, `expected`
submitted in-window). Empty denominators return null / `empty_denominator`,
never zero. Dimensions include `policy` (sorted required policy IDs plus a digest of their
immutable bodies) and
`task_class`, plus `route`, `agent_kind` and `profile`; native drill buckets and revision
refresh/rebuild are supported. Report and export share this evaluation.

`compare --metric M30 --by configuration` uses the same submission cohort;
the arm is the first submission attempt's dispatch configuration, frozen across
later attempts. M30 is compared separately from terminal/assignment metrics.
Counts remain visible below the 20-adjudicated-candidate comparison minimum;
values and rankings are suppressed as `insufficient_data`. Policy-body digest
mixes are reported per arm and differing mixes prevent rankings, as differing difficulty
mixes already do. Query policy/task-class strata remain separately available.
There are no new tables, migrations or retention/backup classifications.

First-candidate history is loaded only for M30 native queries/comparisons or
report derivation. Other lifecycle queries retain the original lifecycle
watermark; canonical file/head identities invalidate candidate-dependent
aggregates. Policies and per-submission verdict lookups are prepared once and
use canonical indexes. Reports reuse central task evidence and maintain the
M30 body in the validated central-provider aggregate; stale or missing bodies
fall back to the identical evaluator without a second rich lifecycle load.


### DG3: observed accepted throughput (M03)

Registry v5 adds `M03.operating-v1`; historical absent `M03.v1` remains
servable. Unique authoritative task acceptances are placed by their first
acceptance evidence time in `[from,to)`; retries and additional receipts never
count again, and replay candidates remain excluded. Query, report, export and
aggregate revisions share this evaluator. Configuration comparison remains
`comparison_unsupported`: the project operating denominator cannot be
attributed to configuration arms from available observations.

Sidecar stream `operating` version 1 stores `operating_intervals`,
`operating_clock` and `operating_gaps`. The ticker telemetry worker observes
canonical Active/paused state and control epoch at most once per nominal
15-second cadence, after controller services and before expensive collection.
Accelerated 250-ms controller ticks do not accelerate operating samples.
`HERDR_FARM_TELEMETRY_COLLECT_SECS=0` disables all ticker telemetry writes
and sidecar creation. Automatic operating observations use only existing
sidecars, including paused projects. A configured native-source collect can
create the sidecar and start its first observed prefix; a project with neither
a sidecar nor recorded native sources has no telemetry worker or writes.
Explicit public `operating::observe` calls remain an opt-in producer API.
Adjacent Active observations from
the same run/control epoch extend one `[start,end)` interval. A gap longer
than **N=3** nominal passes closes at the last observed endpoint; restart,
control-epoch changes and pause/resume split intervals. No failed/missed pass
is extrapolated. Short pause/resume between observations changes the canonical
control epoch and therefore breaks continuity too. Operating state reads and
all sidecar
writes run on the telemetry thread. Scheduling uses file-existence checks and
a bounded-cadence canonical source-eligibility query for absent sidecars. That
read defers on SQLite/file-lock contention instead of waiting on the
controller. The
operating reader releases its canonical file lock before opening or waiting
on the sidecar writer. A busy worker misses
a sample instead of creating an unbounded queue; the next observation detects
the gap. Crash/stop tails remain censored. No post-stop wall time is counted.

M03 unions and clips recorded endpoints to the window, counts acceptance
transitions independently (including those in observation gaps), and returns
the exact unreduced rational `accepted * 3600000 / operating_ms` tasks/hour.
`denominator` is the rational operating hours `operating_ms/3600000`.
Zero hours return null / `zero_operating_hours`; no observations return null /
`operating_hours_not_recorded`. Missing acceptance times, recorded gaps,
windows extending before/after observations, and unbounded tails make coverage
partial, with explicit reasons and `censored.open_intervals`; none become
complete by dropping unobserved time. No categorical dimension or denominator
drill-down is available. M03 watermarks include the sidecar; mutation frontiers
invalidate its revisions on interval/gap/clock changes. Reports read fresh M03
separately from the maintained central body, whose other metrics do not depend
on operating observations; ticker heartbeats do not invalidate M13/M40 caches.
Fixed windows are
reproducible; as-of reads use immutable revision bodies.

Retention class `sidecar.operating_intervals` retains all three tables without
a TTL: they are durable source facts that native rollouts cannot reproduce.
Full sidecar backups include them, with table row counts in the inventory.
Restores retain those observations; a new ticker run records restart coverage
rather than extending a restored open tail.

## 4. Aggregate revisions (stream `analytics`, version 3)
## 4. Aggregate revisions (stream `analytics`, version 4)

`migrations/telemetry/analytics/0001_aggregate_revisions.sql`:

- `analytics_cells(cell PK, metric, definition, cohort, window_from_unix_ms,
  window_to_unix_ms, horizon_ms, dimension, tracked_unix_ms,
  checked_unix_ms)`: the tracked cells; `cell` is the canonical JSON key
  `{by, cohort, definition, from, horizon_ms, metric, to}`. Only
  `checked_unix_ms` is mutable.
- `analytics_revisions(revision PK, cell, kind initial|restatement,
  supersedes, body, content_digest, watermarks, registry, recorded_unix_ms)`:
  append-only (triggers refuse update and delete). `revision` is the
  projection sequence (`--as-of-seq`), `recorded_unix_ms` the knowledge time
  (`--as-of`). `content_digest` = sha256 of the canonical JSON
  `{body, lineage}` (empty buckets omitted); `watermarks` is provenance, not
  content.
- `analytics_lineage(revision, bucket, ordinal, entity_kind task|attempt,
  entity_id, attrs)`, append-only: the drill-down lineage of each revision.
  Stream 4 exposes these exact columns through a view over `WITHOUT ROWID`
  lineage rows and interned exact entity/attrs values; ordinal order and
  immutable revision bodies/digests are unchanged. Disposable provider caches
  may share an exactly equal M40 byte range with an immutable revision; expiry
  evicts that cache and leaves the existing source-evaluation fallback. Legacy
  JSON is copied byte for byte. See certificate-scale §4.15.

Commands (writes only this stream's tables; needs an existing sidecar, else
`unavailable: collection_not_run`):

- `analytics refresh [--metric M --cohort --from --to --horizon-ms --by]`:
  evaluates tracked cells whose inputs changed (first run: every active metric's
  default cell except the self-observing M73/M74). Clock-dependent cells always evaluate. Providers share pinned
  canonical and sidecar read snapshots. Serialized revisions are committed in
  short immediate transactions after rechecking the live input generations;
  `deferred` lists cells whose inputs changed during evaluation, leaving them
  due for the next refresh. `comparison_deferred` reports the same condition
  for the workspace comparison. Unchanged cells only advance `checked_unix_ms`.
  `evaluated` lists the cells evaluated and `write_lock_ms` reports cumulative
  time inside successful immediate transactions, excluding acquisition waits.
  A late correction therefore appends a `restatement`
  superseding the previous revision; earlier revisions stay readable byte for
  byte. Racing refreshes append once.
- `analytics rebuild [--verify]`: recomputes every tracked cell from the
  sources and compares it with the latest revision (`identical`), re-digesting
  the stored bytes (`stored_intact`); without `--verify` a differing cell is
  restated.
- `analytics snapshot`: latest content per tracked cell without revision
  numbers or times: two projects rebuilt from the same sources print the same
  bytes.
- `analytics revisions [--metric M]`, `analytics status`, `analytics plans`.
- Ticker: on its telemetry pass (default every 300 s) the lane refreshes
  tracked cells, at most once a minute, and only once an operator has run
  `analytics refresh` (a tracked cell exists).

Restrictions: a revision's content is whatever its sources held when it was
recorded; the sources themselves keep no history (certificate-core R4), so a
rebuild reproduces the latest content, and older content only survives in the
stored revisions. Lane values that depend on the wall clock (e.g. censoring
at a horizon) restate as time passes. As-of answers exist only for cells
refreshed by then (`no_revision_as_of` otherwise).

Foreground `analytics refresh` retries writer admission with 10–50 ms
jitter and a 30 s cumulative writer-wait budget across its write transactions.
It holds no writer transaction while waiting. After admission it rechecks live source
generations and the canonical file/WAL identity against the prepared snapshot;
stale cells and presentation bodies remain deferred. The canonical head read
runs before admission, outside the sidecar writer. Ticker refreshes try writer
admission once and defer contention to the next pass. Other errors remain
visible. The bound covers admission waits, not evaluation, writes or the
existing sidecar-open wait.

## 5. Pagination and drill-down

`--drill <bucket>` pages the identities behind one native metric: buckets
`numerator`, `denominator` (M02, M07, M30), `outcome.<disposition>`,
`excluded.<reason>`; rows `{entity, id, ...attrs}` (tasks: disposition,
terminal and assignment times, attempts, route, agent kind, task class;
attempts: task, state, decision time), ordered by id. `--page-size` 1–500
(default 100); `next_cursor` is null on the last page,
`next_cursor_expires_unix_ms` its expiry. The cursor (TM4.3,
[contracts-export.md](contracts-export.md) §4) is authenticated with a keyed
MAC (per-user key under the config directory), scoped to the project and
expires after 30 minutes; it binds the normalized request, the snapshot's
`content_digest`, the pinned revision, the bucket and the position. A first
page whose live content matches a stored revision is pinned to it: later
pages read that revision's lineage and neither duplicate nor skip rows while
new data arrives. An unpinned live snapshot that changed answers
`restart_required`, never a mixed page. Exports (`telemetry export --drill`)
page through this same cursor. High-cardinality
identities appear only in drill rows, never as metric labels or dimension
values. Lane definitions answer `drill_unsupported` and drill through their
lane's ledger commands.

## 6. `telemetry report`, the fleet pane and indexes

`telemetry report` and the fleet pane read through the query service's read
path (`analytics::query::report`: the central slice metrics, then each lane's,
a lane key replacing a central one), then fills missing rows from the registry.
Both text and JSON list every registry metric in registry order, including
unavailable values and reasons. M01 and M06 use their native query definitions;
M50 reads “per recommendation (see `telemetry PROJECT recommend`)”. DG1 adds
the native M30 body; the other
report bodies retain their contracts. The report keeps each lane's own
definitions and does not apply the
registry's activation gate (every current family is active). For every
lane or central metric the report prints, `query --metric <its definition>`
returns that
body as `detail` (`report_and_query_share_one_read_path`). The `analytics`
lane adds no report keys.

`analytics plans` prints `EXPLAIN QUERY PLAN` for the hot reads with a verdict
(`indexed`, `full_scan_inherent` for the table a cohort aggregates in full,
`needs_index` for any other scan or an automatic index):

| query | store | verdict | owner and proposed index |
|---|---|---|---|
| as-of by sequence / time, latest, next, lineage page, buckets | telemetry.db | indexed (`analytics_revisions_cell`, `analytics_revisions_cell_recorded`, lineage primary key) | analytics (this stream) |
| `lifecycle_attempts`, `lifecycle_contracts`, `lifecycle_classes`, `lifecycle_replay_candidates` | state.db | one pass over the aggregated table; every correlated lookup an index search | canonical |
| `lifecycle_acceptance_times` | state.db | one pass over `verified_results` or the contracts it joins (either may drive the join), every other lookup an index search | canonical: `verified_results_by_submission` added by migration 0067 (TM5.1, certificate-scale.md §5); before it, an automatic index per run |
| central M08/M13/M15 source scan | telemetry.db | one pass over `rollout_sources`; `codex_usage` searched per source by `path_digest` | codex stream 3: `codex_usage_by_path` (TM5.1); before it, `codex_usage` was scanned once per source |
| usage by session | telemetry.db | indexed (primary key) | codex |

## 7. Comparisons and experiments (TM4.4)

`telemetry <slug> compare` and `experiments plan|report` are specified in
[contracts-evaluation.md](contracts-evaluation.md). They read cohorts
through this service's lifecycle evaluation (the same membership and
lineage as `query`), add no metric and no sidecar stream, and never write.
The registry's `metrics registry --json` gains one additive key,
`comparison` (`analytics-comparison.v2`: comparable definitions M02/M07/M30,
bootstrap method, seed and level, the 20-task cell minimum, the
`beta_binomial_eb.v1` pooling prior, `hajek_ipw.v1` and the paired M42
reference); the text form and every metric entry are unchanged.

Analytics stream 2 adds disposable `analytics_workspace_metrics` and
`analytics_workspace_comparisons` rendering projections. Metrics follow their
revision deletion; comparisons retain the latest row and the analytics retention
window. Rebuild recreates metric projections from recorded revision bodies.

DG6 adds `verification_flip_rate.v1` in registry v2: quality lane,
activity window, since-only, completed verification record time, per-project
pair ratio with per-policy id/digest drill-down. See contracts-quality.md §6.
The prior metric definitions retain their meanings.
Analytics stream 3 (`0003_input_frontiers.sql`) adds durable source-table
mutation generations, checked-cell inputs and validated provider aggregates.
Writable open checks trigger installation against `PRAGMA schema_version`
even when all stream versions are current, so newly created source tables
receive mutation triggers. Rebuild continues to bypass provider aggregates.

### TFIX-4 lifecycle profile dimension

`profile` reads the effective profile name from sealed `attempt_inputs`.
Like `agent_kind`, a task with no attempts is `unassigned`, a missing name is
`unknown`, and multiple distinct names across attempts are `mixed`. Names are
bounded by the canonical profile contract; attempt IDs are never labels.
The checked `lifecycle_attempts` query extracts kind and name in one indexed
join. Maintained lifecycle bodies now encode a seventh attempt element;
old six-element bodies fail decoding and fall back to the canonical load.
The v7 registry stamp invalidates old maintained inputs; `analytics refresh`
or `analytics rebuild` writes the new body. Historical revisions remain intact.
Configuration comparisons retain content-addressed arms and their frozen
profile/model/effort labels; a profile name does not replace configuration identity.

Schema 72 supplies event insertion times going forward. M19 remains unavailable:
historical blocked intervals still lack complete timed history; no historical
times are inferred. Lifecycle `--by role` uses immutable launch lineage and
labels tasks without it `unknown`.

## 8. Stored quality and consumption metrics (MET-NOW-A)

Registry v9 adds `M51.v1`–`M58.v1`, all certification `fixture`, evidence
`tests/telemetry_stored_metrics.rs`. The analytics lane produces them for
`telemetry PROJECT report [--since MS]` and `query --metric M51,...,M58`.
They use since-only activity windows; `--to` and `--by` retain the lane
rejections (`window_end_unsupported`, `dimension_unsupported`). Setup cells
are embedded as `by_profile`, using the lifecycle effective profile name
(TFIX-4), with `unknown` when absent. Text reports print each profile,
severity and currency cell. Ratios are exact unreduced strings; monetary
ratios preserve decimal amounts without floating-point division. No metric
adds stored rows, schemas, prompts, policy command text, filenames or diffs.
Existing input generations invalidate cached projections; normal analytics
refresh records derived aggregates under the existing retention/backup classes.

| id / name | definition and window | unavailable / empty reasons |
|---|---|---|
| M51 `verification_strength` | Distinct submissions with accepted verifier evidence whose executed signed policy includes a command / distinct accepted submissions. Window by first acceptance. `by_profile`, `file_presence_only`, `policy_unavailable` accompany the project share. | `empty_denominator` (null); `acceptance_policy_unavailable` for missing, malformed or digest-mismatched policy evidence unless another executed policy proves a command. |
| M52 `defect_density_by_setup` | M21's current validated unique non-seeded finding roots, windowed by discovery arrival, / distinct candidate submissions with completed reviews, windowed by completion. Repeated reviews of one candidate do not increase its denominator. Attribute findings and denominator to the candidate author's profile. Severity uses the discovery claim's owner-assigned `finding_severity.v1`; every severity cell shares the profile's reviewed denominator. | `no_reviews`; profile cells with no denominator use null `empty_denominator`. |
| M53 `cost_per_validated_finding` | Latest published-rate attempt estimates (primary plus eligible children) for the deduplicated union of attempts bound by `review_sessions.attempt_id` and all attempts of tasks bound by `review_briefs`, including skeptical review tasks and unsuccessful attempts, / the same M21 finding count as M52. Window by attempt decision and discovery arrival. Sum once per eligible entry, separately per currency in `by_currency`; author and unrelated task spend stays outside. | `no_reviews`, `collection_not_run`, `not_priced` (or `cost`'s unavailable reason), `no_usage`, `no_priced_entries`; zero findings gives null `empty_denominator`. Attempts without a launching/running lifecycle mark are excluded from spend and missing-cost coverage. Missing launched review attempts or unpriced entries mark observed currency subtotals `partial: review_cost_incomplete`, never complete spend. |
| M54 `test_weakening_rate` | Submissions flagged by `tests-net-removal.v1` / submissions with an observed clear or flagged quality signal. Window by submission creation. Existing collection observes first candidates only; later and uncollected candidates remain explicit exclusions, never inferred clear. Proxy, `source_trust=proxy_observed`. | `collection_not_run`, `no_observed_submissions`; observed empty population gives null `empty_denominator`. Exclusions: `not_collected` and stored `weakening_reason` (`repository_missing`, `git_unavailable`, `diff_failed`, `diff_unparseable`, or `weakening_unavailable`). |
| M55 `subagent_token_share` | Native-linked child-session input plus output / primary plus child input plus output, per profile. Children follow the attempt usage `children` convention, including guardians and eligible forks; reasoning is already in output. | Common usage reasons below; `no_children` when eligible requests exist but no child usage. |
| M56 `reasoning_and_cache_share` | Per profile `reasoning_share` = reasoning/output; `cache_share` = normalized cached input/inclusive input. Primary and eligible children contribute once. | Common usage reasons; `reasoning_tokens_not_reported` affects only the reasoning component; zero component denominators are null `empty_denominator`. |
| M57 `long_context_exposure` | Input tokens in requests whose inclusive input exceeds the model's threshold / all eligible input tokens, per profile. Every model currently uses constant `LONG_CONTEXT_INPUT_TOKENS=272000`, the published long-context price boundary. Exactly 272000 is below the strict exposure cutoff. A later definition may configure model-specific thresholds. | Common usage reasons; zero input gives null `empty_denominator`. |
| M58 `fixed_overhead_per_attempt` | Median inclusive input of each attempt's first primary request, per profile. Order primary sessions by session start then session identity, requests by ledger position. Children do not become startup samples. Exact arithmetic midpoint for an even sample count, including `.5`; value is a decimal string and `samples` gives attempt count. | Common usage reasons; no startup samples gives `no_usage`. |

M55–M58 window by attempt decision (`--since` includes the selected attempts'
full retained usage). Before collection: `collection_not_run`; before a
current accounting sync: `accounting_sync_required`; no eligible normalized
requests: `no_usage`. Counted delta ledger entries remove repeated responses,
resume observations, losing OTLP surfaces and uncertified fork replay.
Native graph ownership follows parent links, never a child's cwd. Excluded
attempts/records remain in each profile's `excluded` and partial coverage:
`not_bound`, `no_usage`, `quarantined`, `cli_version_uncertified`,
`schema_unrecognized`, `records_not_accepted`, and the graph's linkage reasons
(`parent_cycle`, `parent_not_collected`, `no_native_parent_evidence`,
`inclusion_unknown`, `predates_collection`, `pending_reread`,
`fork_replay_not_certified`). These shares describe observed eligible usage;
partial coverage never claims complete attempt exposure.

Policy classification reuses `ExecutionPolicy::parse` and checks the stored
policy digest against executed verifier evidence. File-presence-only means
all checks are recognized nonempty-file idioms (`/usr/bin/test -s FILE` or
`git grep --quiet --no-index -e . -- FILE...`, allowing the first two flags
in either order; only `/usr/bin/git` or `/bin/git`, and `/usr/bin/test` or
`/bin/test`, are recognized), with no toolchain. A toolchain, another command or an executed named
or stress command that is not a recognized presence check makes the policy
`command`. This is a conservative syntactic classification of retained signed
policies, never another execution or a claim about test adequacy.

## MET-NOW-B: flow and operations (registry v9)

Registry v9 adds M60–M64, using the `operations` lane, activity cohorts and
since-only windows. Query, report, exports and revision refresh share these
producers. `--to`, dimensions and drill-down are unsupported; historical
as-of queries use recorded analytics revisions. All values are descriptive
metadata, never dispatch, acceptance or capacity authority.

| ID / definition | Producer and value | Missing evidence |
|---|---|---|
| M60 `M60.idle-v1` / `idle_gap_distribution` | Complement of the union of half-open attempt running intervals, clipped to the union of observed operating intervals when an operating clock exists. Otherwise only the first-to-last known running span is observed. Adjacent idle pieces merge; pauses and observation gaps split them. `value` has count, nearest-rank median/p90 milliseconds and total milliseconds. Unknown activity spans (including the prefix before lifecycle logging when pre-log attempts exist) are excluded, make the value partial, and `coverage.unknown_ms` is explicit; open runs end at evaluation time. | `no_running_intervals`, `no_observed_operating_intervals`, `predates_lifecycle_log`. An observed span with no gap has count/total zero and null percentiles. |
| M61 `M61.launch-v1` / `launch_latency_by_phase` | Attempts reserved at/after since (dispatch time fallback for pre-log attempts), excluding replay artefacts. Per-attempt decided→reserved, reserved→launching and launching→running milliseconds plus count/nearest-rank median/p90/total for each phase. `never_running` is terminal reserved attempts without running / attempts that ran or terminal reserved attempts; open launches remain `pending`. Recorded termination cause buckets give counts and shares of the same adjudicated denominator, using bounded enum metadata; missing cause is `cause_not_recorded`. | `no_attempts`, `launch_phases_not_recorded`; a summary without samples carries `no_phase_samples` and null total/percentiles; individual phases: `predates_lifecycle_log`, `phase_start_not_recorded`, `phase_end_not_recorded`, `invalid_phase_order`. Empty adjudicated denominator is null with `empty_denominator`. |
| M62 `M62.load-v1` / `machine_load_at_launch` | First launch-start host load sample (1/5/15-minute decimal strings), joined to reached-running / never-running / pending outcome (pre-lifecycle history without a running mark is `unknown`, with null reached-running). The native resource adapter takes one bounded `/proc/loadavg` read before its creation request; recovery cannot overwrite it. Verification runs expose their existing verifier-owned one-minute sample and verdict, placed by run creation time. Report retains per-attempt/per-run observations and counts known launch/verification samples. | `load_samples_not_recorded` when neither source has known load; rows carry `launch_load_not_recorded`, `verification_load_not_recorded`, or `unreadable_or_invalid`. Load is host-wide, not attributed to this project. |
| M63 `M63.storage-v1` / `storage_growth` | Ticker samples logical regular-file bytes of `.state/state.db`, `.state/telemetry.db`, `.state/worktrees` and `.state/worker-output`, at most once per actual hour (unscaled), bounded to 90 days and 2160 rows. Growth is `(last_bytes-first_bytes)*86400000/elapsed_ms`, exact unreduced bytes/day in JSON, including negative growth; text renders a decimal with two places and the unit `bytes/day`. `since` selects samples; it never invents a boundary sample. | `storage_not_sampled`; growth `insufficient_storage_samples` with fewer than two endpoints; `scan_incomplete` for unreadable or traversal-budget-exhausted endpoints. Missing directories are zero. Symlinks and special files are excluded, content is never read, worker output has a 100000-entry traversal budget; each worktree has its own 100000-entry / 2-second budget, with an outer 4096-worktree / 30-second cap. Worktree coverage records covered and expected direct children per sample (expected is a lower bound when the outer scan stops); incomplete totals remain unknown. Historical samples have null coverage. Scanning runs in the deferred telemetry worker, outside the ticker hot path. Samples captured before operations v3 contain main-file-only state sizes; growth spanning that upgrade may include the WAL measurement change. State DB sizes include `state.db-wal` logical bytes (missing WAL is zero), and exclude SHM; telemetry DB sizes exclude WAL/SHM; output snapshots are outside the live output category. |
| M64 `M64.brief-v1` / `brief_size_vs_outcome` | Initial rendered brief `prompt_chars` from the canonical `runtime.worker_brief` operation, never prompt content. Small `<4000`, medium `[4000,16000)`, large `>=16000` characters. Per bucket: attempts, distinct tasks, accepted tasks (current authoritative receipt evidence), accepted tasks/tasks and attempts/tasks as exact unreduced ratios. A task with attempts in multiple buckets belongs to each; these are descriptive associations, not causal effects. | `brief_size_not_recorded` when no brief size is known, also an exclusion/coverage count; empty buckets have null ratios with `empty_denominator`. |

Text report prints one named line per metric with its structured value, or
`n/a (reason)`; per-attempt/run lineage stays in the JSON body. `operations`
stream v1 is `migrations/telemetry/operations/0001_samples.sql`. Launch load
writes are best effort to an existing current table with zero SQLite wait;
absence or contention does not affect launch. Storage sampling never opts an
untouched project into telemetry. Collection disabled at the ticker disables
these ticker writes as well. No canonical schema or launch receipt changes.

## 10. Work-item rework and delivery (MET-REWORK-1)

Registry v10 adds fixture-certified `M65.v1`–`M69.v1`, evidence
`tests/telemetry_rework.rs`. The analytics lane reads canonical schema 72
launch lineage, verification/integration receipts, lifecycle marks, review triage
and the accounting lane's published-rate estimates. No new schema or producer
process is required. This is metadata only (contracts §7); work-item and task
identities are never metric labels.

The activity cohort includes work items whose **first launch** is at or after
`--since` (`query --from`); it includes their entire observed chain, including
later attempts. These are since-only definitions: `--to`, horizons and query
`--by` are unsupported. The report and query detail contain the profile and
currency breakdowns described below. Launch means the earliest launching or
running lifecycle mark, not reservation or SQLite insertion time. No lineage
is inferred for historical tasks: `excluded.lineage_not_recorded` counts them;
without any recorded work item all five values are unavailable with
`lineage_not_recorded`. Missing launches are `launch_time_unknown`, earlier
work items are `outside_window`.

| id | name | definition |
|---|---|---|
| M65 | rounds_to_green | Distinct fix tasks launched no later than the candidate creation time of the first independently accepted build/fix submission; median, nearest-rank p90 and max, overall and by the first build launch's effective profile. Every required policy must have an accepted verification receipt; empty/unknown policy sets never imply green. |
| M66 | rework_cost_share | Fix, recheck and repeat review spend / all spend in cohort work items, exact decimal ratios per currency and per effective attempt profile. A repeat review/skeptic task launches after another task of the same lineage role has a terminal mark; initial concurrent reviews are excluded from the numerator. |
| M67 | escaped_defects | Current validated, unique, non-seeded review findings discovered strictly after the work item's first build/fix acceptance, counted once by their canonical discovery claim. Count and exact findings / accepted work items ratio; owner resets and duplicate merges restate the count. The finding targets the work item through its review opportunity, never title matching. |
| M68 | work_item_lead_time | First launch to first recorded build/fix integration, otherwise last terminal lifecycle mark when all attempts have terminal marks; median and nearest-rank p90 in milliseconds (max also provided). Open or missing terminal tails remain censored. |
| M69 | waste_share | Spend on failed/cancelled attempts or attempts of explicitly superseded tasks without every-policy acceptance evidence / all spend, separately per currency. Accepted attempts remain in the denominator. Union membership prevents double counting; lost attempts are not assumed failed. |

Text reports include `M66 rework_cost_share profile=PROFILE currency=CODE n/d`
and `M67 escaped_defects count=N n/d`, alongside the aggregate currency rows
and M65/M68 distribution objects.

M65 excludes unfinished/unverified work items as `not_yet_green`; M68 excludes
open/unknown tails as `open_or_terminal_time_unknown`. A distribution with no
samples is unavailable `no_samples`; even medians use the arithmetic midpoint
and retain half units exactly. M67's empty denominator is null with
`empty_denominator`. Spend never mixes currencies or uses floating point;
zero denominators are null with `empty_denominator`, absent estimates yield
`cost_not_observed`, and incomplete cost yields `work_item_cost_incomplete`
partial currency cells retaining observed numerator and denominator subtotals.
Missing complete costs are counted as `attempts_without_complete_cost`.
Attempts with neither a launch mark nor cost entries are excluded from spend
and incomplete-cost coverage, so they do not make M53/M66/M69 partial; launched
attempts without usage remain incomplete.
Missing profiles use `unknown`. Pricing revisions are exposed for M66/M69;
rate cards remain fixture-only estimates, not provider charges. Analytics
refresh/rebuild and as-of revisions retain the same bodies and invalidate on
canonical or sidecar input changes, including owner triage corrections.


MET-STATES-1: M37 checks attempt-local every-policy acceptance before supersession
or abandonment and separates accepted_then_cancelled cleanup spend. M69 excludes
accepted work from waste using the same candidate verdict; a partial policy pass
does not establish acceptance. No canonical schema or retention changes.

## MET-WORKER-1: worker and command friction (registry v10)

All definitions are `.v1`, fixture certified, metadata only, and read at query
or report time from existing stores. No schema or retention changes. M78–M79
are reserved: the owner card assigns M70–M79 but specifies eight metrics.
Worker cohorts use attempt reservation time at or after `--since`; CLI cohorts
use invocation time, additionally bounded by the CLI 90-day retention window.
M77 uses notice delivery time. Per-profile output applies to worker metrics;
CLI caller labels cannot be joined to worker profiles without guessing.

| ID / report name | Definition | Missing evidence |
| --- | --- | --- |
| M70 worker_end_states | Six end-state counts, including unknown and never_running; uses the A10 precedence for launched/bound attempts. Claude falls back to canonical ended-without-submission notices. | Coverage counts accompany metadata-free canonical end states. |
| M71 lingering_time | Last session record to terminal mark, nearest-rank median/p95 in ms; count strictly over 600000 ms. Negative/open/missing times excluded and counted. | session_end_or_terminal_time_missing |
| M72 failed_command_share | Failed exec items / all exec items; same denominator for exit classes 1, 2, 127_not_found, other_nonzero, signal. Unknown exits counted separately and mark partial coverage. | empty_denominator; exit_code_unknown |
| M73 help_lookup_share | Help / retained invocations, overall and by worker/coordinator/operator/plugin/ticker. | cli_invocations_not_collected; no_cli_invocations; empty_denominator |
| M74 command_friction | Error plus usage_error / retained invocations, by caller; ten command paths ranked by error count, ties by path. | Same as M73. |
| M75 unanswered_worker_questions | Unanswered request_user_input* calls / observed attempts, plus exact per-attempt counts. Async output alone is not an answer. | session_metadata_not_collected |
| M76 context_window_fill | Per-attempt maximum input/context-window ratio, nearest-rank median/p95 and fraction strictly over 0.8. | context_window_or_input_tokens_missing |
| M77 coordinator_reaction_time | Each retained inbox_items row with worker-result- id to first coordinator CLI timestamp at or after delivery; median/p90 ms. A CLI may follow multiple notices. | coordinator_cli_invocations_missing; worker_result_delivery_times_missing; notice_time_or_next_cli_missing |

Worker metrics carry observed/unavailable attempt coverage plus never_running_attempts.
Never-running attempts (no running mark or bound session) are exclusions from
metadata coverage and M71/M76 missing_samples, and their own M70 bucket.
Launched/bound attempts without metadata mark partial session_metadata_not_collected.
M71/M76 missing_samples counts eligible attempts without a valid sample, retaining
the sample-specific reason when metadata is otherwise complete. Sample distributions include
sample counts; empty ratios have null value and empty_denominator. Decimal
quantiles are strings. Canonical events without schema-72 times are never
assigned inferred times. M77 unmatched notices are counted, not zero-duration
samples. CLI self-observation is best effort and reports may append their own
invocation after the projection; worker-reported observations remain untrusted.
The worker provider cache group depends on the clock for retention expiry as
well as canonical and sidecar input generations; other analytics caches retain
their existing behavior. M73/M74 are live report/query metrics and are excluded
from default aggregate tracking: analytics commands observe themselves, so
tracking these metrics would make an otherwise unchanged rebuild verification
differ after every command. `analytics refresh --metric M73` (or M74) explicitly
opts into revision tracking, including these subsequent invocation changes.

## 12. MET-LAUNCH-1 launch reliability and miscellaneous metadata (registry v11)

M90–M95 use definitions `M90.v1`–`M95.v1`, certification **fixture**,
activity-window cohorts and since-only windows. M96–M99 are reserved, with
no invented metric definitions. Report, query, export and revisions share
these descriptive projections; none influences admission or acceptance.
No canonical migration or SCHEMA bump is needed.

| ID | Definition and time basis |
|---|---|
| M90 `launch_success_rate` | Operator/coordinator `launch run` invocations (excluding help/version/usage errors), placed by invocation start. Numerator: invocations whose task reaches running after that start and before the next retained invocation for that task, or whose existing attempt is still running at invocation start (idempotent launch). Denominator: launches attempted, including command failures. `by_task` includes launches and tries at/before the task's first running mark. Untargeted legacy invocations with CLI outcome `ok` are attributed to the earliest running mark after their start, before the next launch invocation and within five minutes (exclusive upper bound), without reusing a mark for another legacy invocation. `untargeted_invocations` reports all launches without recorded targets; `time_attributed_invocations` reports inferred successes; `unattributed_invocations` reports unresolved launches, retained in the attempted denominator and flagged partial. `targeted_value` and `targeted_denominator` report the rate excluding all legacy invocations. CLI exit success alone is never running evidence. |
| M91 `stuck_attempt_interventions` | Operator/coordinator `task cancel-attempt` and `launch stop --force` calls (excluding help/version/usage errors) grouped per attempt in inclusive 5,000 ms windows anchored at the first call (retries do not extend the window). All calls, including errors, participate; `errored_calls` separately counts error outcomes without deduplication. Each window is classified at its first call: `cleanup_after_acceptance` if a `verified_results` row for that attempt predates the call; otherwise `other` if a terminal lifecycle mark or recorded worker termination predates it; otherwise `stuck`. Acceptance takes precedence. Headline `value` and `per_day_utc` count only stuck windows; `breakdown`, `per_attempt`, and `per_attempt_classes` include all classified windows. Unknown targets are excluded from the headline and reported as partial. Forced stop is attributed to the task's latest reservation at invocation start. Historical evidence is checked before the call, independent of the report window. |
| M92 `ticker_error_rate` | Counts of explicitly logged ticker errors per UTC hour bucket, by `lock_contention`, `expired_inventory`, `ambiguous_outcome`, `permanent_failure`, `other`. Fixed saturating ticker counters are drained once each pass and replicated to existing sidecars. Values are root-wide: never sum across projects. Background errors enter the next completed pass. No log text is stored. Partial hours, missing pass prefixes/gaps and observation endpoints are explicit; zero-wait write contention can leave unobserved passes, which make coverage partial (`ticker_passes_not_observed`); no unobserved hour is assumed zero. |
| M93 `work_item_time_breakdown` | Items placed by first reservation across lineage tasks. Complete closed items partition first reservation→last terminal wall time into running, launch overhead and gaps between attempts. Interval unions handle parallel attempts; running takes priority over launch overhead. Never-running closed attempts retain total and running/gap observations but make launch overhead and its share unavailable (`never_running_attempt`); known launch overhead stays a subtotal. `worker_attempt_ms` separately sums running attempt durations. Shares are exact unreduced milliseconds/elapsed ratios. |
| M94 `diff_size` | Every submission placed by submission time; whole-range numstat in the disposable verification checkout, with rename folding disabled, no external diff or text conversion. Paths and diff text are discarded. Added/removed counts are retained in verifier metadata and, when present, the sidecar. Binary files make coverage partial. Profile cost/changed-line ratios divide exact single-currency priced cost of distinct submission attempts by their known submitted changed lines (an attempt's cost counted once across its submissions). Missing/binary ranges suppress the ratio. |
| M95 `memory_use` | Proposals placed by proposal time, accepted by promotion and rejected by final rejection or failed validation, excluding promoted proposals. Acknowledged `runtime.worker_brief_delivered` receipts placed by observed delivery time count facts delivered and the snapshot's `omitted_optional_count` as `omitted_for_budget`; receipts are deduplicated by operation. Consumer bindings without a receipt are prepared snapshots, never delivered facts; no receipts gives `no_brief_delivery_samples`. Remember sections count canonical distinct attempt/content candidates placed by submission time; legacy file-backed Remember intake is explicitly unavailable. Only aggregate counts leave the stores. |

Unavailable reasons: M90/M91 `launch_invocations_not_collected`; M90
`launch_target_not_recorded` (partial attribution), `empty_denominator`;
M91 `intervention_attempt_unknown` (partial); M92
`ticker_errors_not_observed`, `ticker_passes_not_observed`; M93 `no_work_item_time_samples`,
`open_work_item`, `lifecycle_time_missing`, `never_running_attempt`, `work_item_time_incomplete`, `lineage_not_recorded`, `empty_denominator` for zero
elapsed shares; M94 `no_submissions`, `submission_diff_not_observed`,
`diff_unavailable`, `binary_lines_unknown`, `diff_coverage_incomplete`,
`cost_not_observed`, `empty_denominator`; M95 `memory_history_not_recorded`,
`brief_delivery_history_not_recorded`, `no_brief_delivery_samples`, `remember_history_not_recorded`,
`legacy_remember_not_observed`. Missing times, absent collection and binary
line counts are never replaced with zero. Retained CLI history covers at
most 90 days/100000 invocations, so retries are observed-history counts.

Operations stream v2 adds `operation_cli_targets` (local typed task/attempt
IDs and force flag, tied to CLI retention), `operation_ticker_errors`
(90 days/100000 passes) and `operation_submission_diff` (lifetime counts).
Writers use existing sidecars only and zero SQLite wait. Telemetry disabled
by `HERDR_FARM_TELEMETRY_COLLECT_SECS=0` suppresses ticker samples. New
sidecars/upgrades arise from normal explicit collection, never launch alone.
Full telemetry backups include all three tables. Targets cascade with CLI
row pruning and are orphan-pruned by capture. Retention classes declare
CLI targets with CLI history and retained ticker/diff metadata separately.


## 13. MET-VERIFY-1 verification metadata (registry v12)

M100–M103 use `.v1` definitions, certification **fixture**, evidence
`tests/telemetry_verification.rs`, activity-window cohorts and since-only
windows. `telemetry PROJECT report` and `query --metric M100,M101,M102,M103`
serve the same exact ratios, with unreduced integer numerators and denominators.
M96–M99 remain reserved. Metric IDs are resolved through the registry without
assuming a two-digit suffix. These metrics never affect acceptance or admission.

| ID / name | Definition and window basis |
|---|---|
| M100 `independent_first_pass_rate` | Submissions placed by submission time. Earliest completed independent run per submission, ordered by run start time (sandbox load sample, falling back to canonical recording time) then run ID, across linked host checks and canonical `accept-N` / `accept-default-N` runs carrying toolchain metadata. Host checks without a submission link may bind by the same attempt including checks before submission; attempt-only links apply to every submission of that attempt. Excluded run reasons count each run once even when an attempt link covers multiple submissions. Green first runs / submissions with a counted first run. Later green cannot replace earlier red. Timeout, interrupted, spawn_failed, abandoned, running and other non-verdict outcomes are excluded and counted by reason; submissions without a counted run are counted separately. `by_profile`, LINEAGE `by_role` (unknown when unbound), and `by_source` (host/sandbox) use the same observed-run denominator. |
| M101 `worker_self_check_rate` | With `--since`, attempts placed by their first submission time; attempts without submission time are excluded as `submission_time_unknown`, earlier submissions as `outside_window`. An unbounded report includes every attempt, also counting `without_submission_time`. Attempts with an exact declared project-test argv match and an observed exit code (including nonzero) / attempts in the cohort. Commands are classified transiently during Codex item collection against installed signed acceptance policies, including toolchain policies and materialized owner default accept policies, plus readable owner `verification.defaults` accept entries. Matching uses launch’s quote/escape parser without shell expansion; raw argv and declaration text are discarded. File-presence policies do not classify project tests. Only `project_test` or null and classification availability survive; command text, arguments and output never enter telemetry. An observed session with no exec items counts as no self-check. Unsupported/missing argv or policy metadata makes coverage unavailable rather than zero. `observed_share` retains the known-attempt subtotal. |
| M102 `verification_minutes_per_accepted_task` | Tasks placed by first acceptance evidence time for their current contract (verification for verify-only routes, integration otherwise). Sum all retained host-check and executable acceptance run durations across those tasks (including red and timeout runs), divided by accepted tasks. Host/sandbox totals and shares are separate; integer milliseconds / (60000 × accepted tasks) is exact minutes/task. Distribution uses complete task duration totals, arithmetic median (even samples retain the exact midpoint as summed milliseconds/120000) and nearest-rank p90, with sample count. Runs outside the acceptance window still contribute to an included task; this is a task cohort, not a run activity window. |
| M103 `final_report_quality` | Submissions placed by submission time. Nonempty claimed-check lists / submissions, plus count of entries. `claimed_without_agreement` is claimed submissions whose first counted independent run is red or absent / claimed submissions. Only SQLite array lengths are read; claimed-check text never enters telemetry. |

Missing evidence: `empty_denominator`, `no_independent_runs`,
`no_host_checks_recorded`, `no_executable_policies`, `no_executable_runs`,
`first_run_metadata_missing`,
`executable_run_metadata_missing`, `verification_duration_missing`, `verification_not_observed`,
`no_complete_duration_samples`, `session_metadata_not_collected`,
`worker_session_stream_before_v17`, `project_test_classification_incomplete`.
Old executable runs lacking toolchain metadata are counted in coverage and
cannot prove a first verdict; old durations cannot become zero minutes.
Known duration subtotals remain available when the overall mean or a source split is unavailable. Earlier missing executable metadata suppresses first-verdict and claimed-without-agreement ratios instead of treating a later observed green as first.
Host metadata is retained observation history, so absence cannot prove that
no host check was ever executed. Submission/attempt IDs in drill-down are
local typed metadata; no prompts, transcripts, code or command output are read.

No canonical migration or SCHEMA bump. Ingest stream v17 adds
`codex_exec_classes`, classified as native-session metadata for retention,
forget-session deletion and full telemetry backup. Migration replays retained
Codex rollouts when the physical class table is newly installed, preserving
existing offsets after logical stream rollback; deduplication remains by session/item ID.
New verifier executions retain `duration_ms` in the existing metadata payload.

## MET-NOTICE-TIMES-1 retained notice and historical timing

M77 reads worker-result notice delivery from `inbox_items.payload.created`
(RFC 3339, converted to milliseconds), including consumed notices, and pairs
it with the first retained coordinator CLI invocation at or after delivery.
Result/ended notices are committed atomically with their originating event;
reading their retained timestamp avoids adding redundant canonical events.
M77 does not require `event_times`. Invalid delivery times and missing subsequent
CLI invocations remain unmatched, explicitly counted.

M83/M86 reconstruct unread worker-result intervals from the same inbox delivery
rows and timed `inbox.seen`/`inbox.done` events, ordered by timestamp. Historical
untimed consumption events before the first retained `event_times` sequence
and their associated notice spans are excluded. Attempts without a reserved
lifecycle mark whose reservation precedes that sequence are excluded as untimed
historical activity. M83/M84/M86 expose
`historical_censored` with separate `events` and `attempts` counts, including
for `--since` windows; these counts describe retained excluded history, not
samples inside the window. Known attempt intervals ending before `--since`
do not participate in the timing-completeness gate. Timed intervals beginning
before the window remain eligible when they overlap it. Missing timing in the
observable period still yields `historical_activity_times_missing`; historical
times are never invented. No canonical schema, event volume, retention or
backup classification changes are needed.
