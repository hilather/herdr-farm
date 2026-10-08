# Verify a retained result

On Linux with the `state-store` feature, an operator can run the independently
isolated verifier through the CLI after a result has been submitted:

```sh
herdr-projects --root /path/to/projects result demo verify SUBMISSION_ID \
  --policy-id builds \
  --idempotency-key verify-submission-1 \
  --work-dir /path/to/new-verification-scratch \
  --timeout-seconds 60
```

When `--policy-file` is omitted, verification uses the installed signed contract policy directly. To export its exact bytes (no added newline), run:

```sh
herdr-farm --root /path/to/projects result demo policy \
  --submission SUBMISSION_ID --policy-id builds --out /path/to/new-policy.json
```

The output file must not already exist. An optional policy file must match the acceptance policy text in the installed signed
contract byte-for-byte, including whitespace. The command cannot substitute a
worker's claimed checks for that policy. Policy input must be a regular,
non-symlink file of at most 4,000 bytes. The check subprocess timeout is 1–3600
seconds; retained-object preparation has its existing separate bounds.

The scratch path must be absolute, must not exist, and must have an existing
parent. The command creates it with private permissions and removes its scratch
contents when finished. An existing path is refused without removing its files.
The project store must already be migrated; this command does not upgrade it.

Successful verification prints a JSON outcome with `state: "accepted"` and a
trusted receipt. A recorded rejection prints its JSON outcome and exits nonzero.
Policy mismatch is an operator input error: the command names both digests and refuses it before recording any new run. Existing historical mismatch rejections remain readable. This preserves worker metrics from new operator input mistakes.
Input/setup errors can fail before an outcome is recorded. Submission by itself
still does not verify work, satisfy prerequisites, or release attempt capacity.

Retrying the same idempotency key and exact inputs returns the historical run
with `replayed: true`; it does not execute the checks or mint a second receipt.
The replay display does not reconstruct a trusted receipt from JSON. A replay also does not certify historical results against checks added after their original run. Changed
inputs with the same key conflict. Reserved attempts remain bound to their frozen
contract revision, including after a newer ordinary contract is installed.

Operator `task cancel-attempt` refuses cancellation while a verification or
integration job for any of that attempt's submissions is `claimed` or
`ambiguous`. The message identifies the job and its state. Wait for its verdict
and delivery acknowledgement, or reconcile an ambiguous delivery, then retry
with fresh expected revisions and head. Refusal changes neither the attempt nor
the task and writes no cancellation request. A pending job does not block
cancellation; its existing revision fence and retirement behavior still apply.

This preserves the submitted commit's real verdict without weakening any
verification fence. Cancellation advances task and attempt revisions, and also
moves the store head used by the verifier's memory fence. Changing only the
automatic job's task fence would still reject a running check as stale.
Contract installation during a check still invalidates that run through the
memory fence. Internal cancellation for a proven scope violation remains atomic
with rejection. Natural worker exit retains its existing verdict deferral.
No store schema change is required.

Contracts with explicit path scopes also constrain the candidate's changed paths.
Before running checks, verification compares the signed base and candidate trees.
A changed path must match a literal write path or a declared directory prefix.
Both names of a rename are checked; read scopes and glob patterns do not grant
write permission. An unchanged file outside scope is allowed. Historical
contracts with no path declarations retain their existing semantics; version-3
output declarations require write scope.

A proven violation records `scope_violation`, verifier feedback and cancellation
in one transaction. Existing cancellation logic retains capacity unless it proves
launch never started. Failure to write the stop request rolls back the rejection
and feedback too. Feedback enters the existing pending replan queue; an operator
can use `feedback SLUG replan FEEDBACK_ID`, or the opt-in bounded replan service
can process it. Neither route by itself launches a planner model or worker.

Missing base objects, failed Git comparison or an over-limit diff instead reject
with `scope_diff_unavailable`; no verification receipt is minted. Diff capture is
bounded by the runner and limited to 10,000 changed paths. These are checks of
retained changes, not a worker filesystem sandbox or pre-write containment.

This is operator-driven production ingress. Automatic verifier dispatch and full
planner-driven decomposition remain unfinished. Local E2E coverage uses real
signatures, Git objects and verifier subprocesses with synthetic runtime fixtures;
it does not certify a live model adapter.

Receipt reuse for every contract requires version 2 of the native verifier's
contract-check record, attesting post-execution checkout identity. Older accepted receipts remain
historical facts, but cannot satisfy new dependencies, authorize integration or
release a barrier without that proof. Migration and exact replay do not invent
proof. Run verification with a new idempotency key to perform current checks and
issue fresh evidence. This also applies to contracts without scope/output
declarations. This record is separate from the Linux isolation version.

## Local integration commands

The Linux `state-store` binary exposes operator-driven local integration:
Scheduler inspection reports this capability as `operator_local`; it does not
claim automatic dependency producers or automatic integration scheduling.

```sh
herdr-projects result PROJECT configure-integration \
  --repository /absolute/repo --reference refs/heads/factory-integration
herdr-projects result PROJECT integrate VERIFIED_RESULT_ID \
  --repository /absolute/repo --idempotency-key integration-1 \
  --work-dir /absolute/new-integration-scratch
herdr-projects result PROJECT reconcile-integration \
  --repository /absolute/repo --idempotency-key integration-1
```

Configuration requires an existing branch that is not checked out. The target
is immutable after configuration; repeating the same configuration is safe.
The signed contract must route the result to `verify_then_integrate`. Integration
uses the stored policy and native receipt, checks the combined candidate tree,
and updates only the configured local ref with an expected-old-OID comparison.
Both SHA-1 and SHA-256 scratch repositories preserve the source object format.
These commands neither push a remote ref nor launch workers.

The integration scratch directory must be absolute, new, and have an existing
parent. The command creates it privately and removes it on completion or error.
An existing directory is refused without changing its contents. Stores must
already be explicitly upgraded. Commands print the integration outcome as JSON;
only an `integrated` outcome returns success.

Retry the same result and key to resume a retained candidate after interruption.
Reconciliation uses the stored operation and existing Git evidence without
building another candidate. It can confirm an already published commit or retry
a previously checked candidate under the existing fences; it is not read-only.
Ambiguous target changes remain blocked for reconciliation. The E2E injects a
failed check-state write and a failed receipt write after Git CAS, then proves
resume, confirmation, replay and exactly one integration receipt.

Automatic integration scheduling and the live dependent-worker acceptance gate
remain unfinished. This operator workflow is covered with local disposable Git
repositories, not live provider certification.

Integration also checks the exact merged commit for every required output in the
signed contract. A passing worker candidate or policy command does not establish
that a target-side merge preserved those files. Only regular Git files satisfy
`git_file`; missing entries, directories, symlinks and symlink ancestors fail.
The check runs before ref publication and during pending-operation reconciliation,
including candidates with previously stored passing policy results. A missing
output records `required_output_missing`, blocks publication and preserves worker
capacity. If that candidate has already been published, it requires reconciliation
instead of receiving an integration receipt. Completed historical records are
not retroactively certified by replay.

A completed integration receipt used for a required-output contract also needs
a stored native merged-output check record. Dependency attachment, current
dependency readiness and barrier validation refuse historical receipts lacking
that record. Migration and replay retain history without manufacturing proof.
To obtain fresh evidence, integrate the verified result with a new idempotency
key; the integrator rechecks the current combined tree. A new receipt may then
replace the old dependency evidence. Contracts with no required outputs retain
their prior compatibility behavior.

When a dependency is attached after verification, a newer unusable receipt no
longer hides an older valid receipt for the selected attempt and contract.
Attachment checks candidates under one shared two-second SQL/input budget and
rolls back its queue or contract transaction if that budget expires. It updates
only the consumer being attached. Queue mutation reads only the selected task, its retained-capacity status and
bounded graph identities/edges. Its existing publication checks, graph work and
receipt attachment share the original command deadline. Historical integrated
receipt selection and broad queue reporting still need further scaling work;
this deadline is not a claim that all queue operations are history-independent.

The native verifier now rechecks HEAD, the Git index and tracked worktree after
checks and child cleanup, before accepting a fresh result. A check that changes
those inputs is rejected with `tampered_tree`, even when it exits zero. Ignored
build artifacts may remain in the disposable checkout. Rejected run records
retain the check's actual numeric exit status when available; it is separate
from the supervisor's failure classification. Historical receipts without a
version-2 check record cannot authorize downstream reuse; fresh verification
with a new idempotency key is required. Migration and replay never upgrade them.

## Evidence reruns and stress (DG6d/e)

Executable acceptance policy version 1 retains its single `checks` argv.
Version 2 adds `rerun_on_failure` (integer 0–2, default 0) and optional named
stress checks. These declarations are validated at signed contract put, and the
operator policy must still match the signed bytes exactly. Example:

```json
{
  "version": 2,
  "checks": ["/usr/bin/git", "diff", "--quiet"],
  "rerun_on_failure": 1,
  "named_checks": {
    "locks": ["/absolute/new-verification-scratch/checkout/check", "locks"]
  },
  "stress": {
    "checks": ["locks"],
    "repetitions": 6,
    "concurrency": 2
  }
}
```

`stress.checks` lists 1–6 distinct names from `named_checks` (at most six
names, each 1–64 ASCII alphanumeric, underscore or hyphen characters).
`repetitions` is 1–6. `concurrency` is 1–5 background processes, so together
with the foreground check there are at most six check/load processes. For each
named check and repetition, background copies start alongside the foreground
check. An optional `stress.load` argv replaces those background copies with a
declared command. Each batch finishes before the next starts. Programs retain
the verifier's allowlist: `/usr/bin/git` or an executable in the copied checkout.
Stress is opt-in, intended for write paths, transactions, locks, migrations and
ticker changes; it is never enabled by default. Any foreground or load failure
fails verification. A load command should remain active long enough to overlap
the check; the verifier does not infer sustained pressure from process start.

A failed primary, named foreground check or load copy gets up to `rerun_on_failure`
additional executions, stopping at the first passing rerun. Reruns exist to
produce evidence, not to pass work: the original failure remains
`checks_failed`, with no acceptance receipt. Background load failures also fail
verification. All executions use the same private checkout;
ignored fixture/build state can persist between repetitions. Tree identity is
rechecked after all children finish, just as for version 1.

One original check timeout budget covers the primary check, reruns, stress and
load cleanup. It is never reset per repetition. Deadline exhaustion kills the
namespace's children, rejects with `timeout`, and records started but unfinished
observations as `cancelled`. Explicit caller cancellation retains the existing
no-verdict rule. Replay never executes repetitions again.

Version-2 run metadata records sequence, kind (`check`, `stress`, `load`,
`flake`), check name, repetition, outcome, numeric exit status when available,
load context and bounded per-test results. Each command streams stdout to the
existing bounded supervisor; repetition test parsing retains at most 8 KiB,
with at most 1 KiB of serialized test evidence per observation. Larger suites
are explicitly unavailable, never partial passing suites. Check stderr is
drained without forwarding supervisor protocol lines. Quality collection
projects completed flake observations, and a failure followed by a passing
rerun counts as a flip on the same tree and policy without accepting work.

## Owner-declared acceptance toolchains

Declare a toolchain once in the external owner `config.toml` (the path pinned
by project migration), then select it in a version-2 acceptance policy:

```toml
[verification.toolchains.godot]
paths = ["/bin/sh", "/usr/bin/env", "/absolute/game/.tools"]
env = ["GODOT_SILENCE_ROOT_WARNING=1"]
network = false
timeout_seconds = 600
```

Paths must be absolute files or directories. The verifier resolves ELF loaders,
shared libraries and declared script interpreters using the worker dependency
resolver. File contents and path metadata, including recursively enumerated
directory entries and dependency identities, are pinned. Files are exposed
read-only at the same paths; paths inside the contract repository are also
exposed relative to the private checkout, so ignored `.tools/` executables work.
Toolchain binds use `ro=recursive`, including repository-relative aliases, so
nested mounts cannot leave owner tools writable.
Directory contents are snapshotted as individual read-only file mounts. Empty
original directories are present read-only. Interpreters such as `/bin/sh` and
`/usr/bin/env` must be declared; they are not supplied by default.

`env` follows `thread_env` validation: at most 32 `NAME=VALUE` entries, uppercase
names, no NUL values, and no loader, Git, shell startup, `PATH` or `HOME`
overrides. Checks receive a cleared environment, fixed sandbox defaults and
these entries. Verifier control variables are not passed to the test command.
For toolchain policies, network access is disabled by default using a separate
network namespace with loopback brought up before checks. Tests can bind and
connect to 127.0.0.1 within that namespace; outbound destinations and host
loopback services remain unreachable. `network = true` inherits network access.
Policies without a toolchain retain their existing command admission, verifier environment, network
access and per-check timeout; integration retains its original lease margin.


The owner can apply the real test command to every generated code launch:

```toml
[verification.defaults."/absolute/canonical/project/path"]
accept = ["godot:./tools/run-tests.sh --headless"]
```

Use the same canonical project path key as `[safety."..."]` in the external
owner config; project-local config cannot declare defaults. At most four
commands are allowed, using the same quoted-argument syntax as `--accept`.
Generated build/fix contracts with `--write` carry `accept-default-N` policies;
plan, review, skeptic and `--contract-file` launches do not. Explicit `--accept`
policies keep `accept-N` ids and replace identical defaults (same toolchain and
parsed argv). `--no-default-accept` skips defaults for one launch. Launch output
lists policy ids and, for acceptance commands, the toolchain and argv. The
coordinator command list names configured defaults, or shows an `--accept`
example when none are configured. Invalid defaults fail preflight with their
owner-config entry. Outputs, defaults and explicit accepts together must fit
32 policies for `verify_only` or six for `verify_then_integrate`; excessive
counts fail before launch, with no truncation.

For generated code contracts, use repeatable
`launch PROJECT run --write … --output … --accept 'godot:./tools/run-tests.sh --headless'`.
This creates a policy with `version`, `toolchain`, `checks` and
`toolchain_digest` before the contract is signed. Arguments support single and
double quotes and backslash escaping; no shell expansion takes place.

The worker also receives the selected toolchain's validated `env` entries in
its cleared launch environment, so it can run the same test commands. Without
an acceptance toolchain these entries are absent. Verifier control variables
(`HP_VERIFY_*`) and worker/agent control variables are reserved. Profiles still
cannot carry environment. Paths and resolved dependencies hidden by worker
isolation (owner secrets, the projects root or private scratch locations) refuse
the launch with a path-specific error; move tools into a readable location.
Conflicting environment entries from multiple toolchains are refused.

For hand-authored contracts, obtain the exact policy text before signing:

```sh
herdr-farm result PROJECT toolchain-policy godot -- ./tools/run-tests.sh --headless
```

Embed that JSON as the acceptance policy's `text`. The signed digest is required
for a toolchain policy; a bare name cannot attest what tools existed at signing.
Only the owner config supplies mounts, environment, network permissions and
budgets. Undeclared toolchains and changed identities are refused at contract
install; changes after install reject verification with
`toolchain_identity_mismatch`. Re-sign a new contract revision after an intended
toolchain update. Run metadata records the toolchain name, digest, timeout,
network setting and every path's identity; environment values are not recorded.
No new storage table is needed: this uses the existing signed policy and bounded
verification metadata evidence.

The owner uid mapping requires util-linux >= 2.38 for `--map-user`. It is a
compatibility measure for tools that refuse uid 0, not a hardening measure:
the agent can still create another user namespace and map itself to root.

Verification and integration rechecks use the attempt's frozen `worker_uid`
policy from the owner safety configuration, never a fresh project setting.
Absent/`"root"` preserves the existing verifier command. With `"owner"`, the
outer namespace retains root for its mounts and root switch; each check,
stress load and rerun executes through an inner
user namespace using the sealed host uid/gid, equivalent to
`unshare --user --map-user=<uid> --map-group=<gid>`. Since its `/proc` is
read-only, the gated child performs `unshare(CLONE_NEWUSER)` and the map writes
before exec using a private descriptor to the setup proc mount. That descriptor
closes before check exec; cross-user-namespace ptrace checks prevent checks from
reopening it through `/proc/1/fd`; supervisor dumpability is also disabled.
No proc mount becomes writable to checks.
Recorded argv includes the frozen identity.
The automatic isolation probe exercises this same mapping and verifies both uid
and gid before claiming work. It also requires zero bounding, permitted, effective,
inheritable and ambient capability sets. The mapped check drops its entire
bounding set before clearing its other capabilities, while it still holds
CAP_SETPCAP in the new namespace. Historical imported attempts without sealed launch
inputs retain root behavior. Nothing gets broader: the inner check namespace
loses root capabilities at exec, host identity is unchanged, and hidden host
paths and read-only mounts stay enforced. HOME remains the private `/tmp` for
toolchain checks; legacy HOME behavior is unchanged.

Owner-mode setup retains only CAP_SETFCAP in its permitted and effective sets
so Linux 5.12+ permits mapping parent uid 0. After mapping, the gated child
clears all capabilities before check exec. The root-mode privilege drop is
unchanged. The availability probe uses the same privilege drop and mapping hook.

Commands beginning with `./` resolve within the copied checkout and may not
traverse outside it. Checks can write only inside that disposable copy and a
private `/tmp`; the rest of the root and owner tools are read-only. Mount
capabilities are dropped and privilege escalation is disabled before checks.
Gitignored output is allowed; tracked changes, index/HEAD changes and unignored
output reject with `tampered_tree`. Nothing is written into the live repository.

Toolchain checks have one timeout for all repetitions: default 600 seconds,
maximum 3600. This overrides the caller's legacy verification timeout. Integration
rechecks each policy with its own toolchain timeout (ordinary policies retain
30 seconds), at most six policies, with a lease covering the sum plus margin.
Automatic verification reserves a 3720-second lease; integration's extended
lease covers up to six maximum-length policies. Exact-key replay remains a
historical result and does not rerun or certify the current toolchain.

Candidate host inspection uses `herdr-farm result PROJECT checkout --submission ID --into DIR`
(or `--attempt ID` when it has exactly one submission). DIR must be absent or empty,
with an existing parent, outside the project and source repository. This creates a
fresh detached checkout of the exact retained candidate and prints JSON with
candidate/base OIDs, attempt, task, path and declared outputs. It grants no acceptance evidence.
Run host checks from that directory and record them with `result PROJECT host-check`.
Read worker reports with `result PROJECT show --attempt ID --report` (UTF-8,
1 MiB maximum; larger reports are refused). `show --attempt ID` exposes declared
review document paths in `artifact_manifest`; read those documents in the candidate checkout.
Never read `.git-quarantine` or `.state/worker-output` directly.

Checkout reuses verification's digest-checked submission objects in
`.state/factory-objects/staging` and imports only the trusted base from the source
repository. Quarantine import happens before submission staging. No source refs
or source checkout are changed. Git uses a scrubbed environment, empty hooks,
an empty template, no global/system configuration, and no submodule recursion.
Source repository configuration is not copied; no LFS or other filter drivers
are configured, so their execution is avoided and LFS pointers stay pointers.
Candidate files remain untrusted: running their scripts on the host uses operator
privileges. This command does not change integration policy or responsibility.

The ticker checks the identity of its executable each pass. Deletion or replacement logs `herdr-farm binary was deleted or replaced since this process started; restart the ticker` and files one deduplicated operator inbox notice per project/process. Automatic verification and integration remain pending/paused for this host condition, and a failure detected after claim remains retryable without a worker rejection. Re-exec pins the running image from `/proc/self/exe`: namespace setup inherits a sealed executable descriptor (closed before checks), and ordinary child helpers use `/proc/<pid>/exe`; delayed launches and filesystem layouts require an unchanged real path. Restart the ticker after reinstalling the binary.
