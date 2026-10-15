# Canonical worker launch implementation

Canonical prepared-worker dispatch is enabled after the authorized live controller
workflow and production-default controller regression tests. This page tracks
runtime work, not packaging or general release acceptance. See the current
[dispatch audit](dispatch-enablement.md) for evidence and unsupported capabilities.
Later dated sections preserve the implementation history; earlier disabled-gate
statements describe their original validation point.

## CI launch diagnostics

The native pane process PID identifies the host-side `unshare` supervisor,
not the agent's PID inside the worker namespace. Initial resource observation
opens that PID, then discovers the namespace init's host PID through the supervisor's
`/proc/<pid>/task/<pid>/children`. Observation errors include the PID and whether
opening the outer supervisor or namespace init failed. Naming/start observation
reopens those retained PIDs after gate release; by then the filesystem sandbox
and agent may already have exited. Reaching naming means the initial gated
supervisor was previously observed, so post-release mounts, the nested user
namespace and agent startup deserve particular attention. The exit log identifies
the outer supervisor. ESRCH can mean a namespace/setup failure, an exited agent
and supervisor, or a stale/invisible
native process inventory; it alone does not identify a sandbox primitive.

The canonical-worker, operator-launch and replay CLI labs retain sandbox/agent
stderr in `agent.log` and append host supervisor PIDs and exit statuses to
`agent-exits.jsonl` (negative statuses identify signals). Canonical/replay labs
keep these beside the native socket; operator labs use their fixture home so
evidence survives disposal of verification server socket directories.
An independent waiter records exits even if no further native API requests
arrive. Failed tests print these files before fixture cleanup, alongside retained
ticker diagnostics. Multiple launches append evidence rather than overwriting it.

Before the eight test partitions run, the workflow prints kernel, namespace
limits, AppArmor settings and util-linux versions, then probes user/mount/PID
namespaces, mount-proc, tmpfs, overlayfs, recursive read-only bind mounts and the
nested user namespace. Each primitive reports its status and kernel messages are
printed afterward. Probe failures leave the full test gate enabled. The existing
runner-only AppArmor user-namespace setting remains in place; further setup
changes require evidence identifying the failing primitive. The canonical
schema is unchanged.

Read-only anchors no longer depend on util-linux interpreting `ro=recursive`.
After each recursive bind, the sandbox enumerates `/proc/self/mountinfo`,
remounts every descendant mount with `remount,bind,ro,nosuid,nodev`, and checks
its kernel VFS options with `findmnt --direction backward --mountpoint`
(covered mounts can share the same pathname; the last mount is visible).
Root-cover bind-back entries receive this protection individually; remounting only the covering tmpfs leaves child
mounts writable. Hidden tmpfs and file masks are also checked. File masks bind `/dev/null`
with `ro,nosuid,noexec`, omitting `nodev` so reads return empty bytes rather
than EACCES; the read-only VFS check still applies. A missing mount
or writable VFS result exits 125 before the worker starts. These checks happen
before explicitly allowed writable exposures are mounted on top; protected
anchors are checked again after setup.

CI diagnostics additionally mirror the descriptor bind-back under a covering
root tmpfs and attempt an actual write from the nested worker user namespace.
The owner-environment E2E probe now poisons `HERDR_DIR` and an unrelated owner
variable and verifies neither reaches the worker. Both sandbox setup and final
agent exec use `env -i`; retry commands use the same retained command builder.
The previously intermittent filesystem sentinel leak still requires an
outside-sandbox reproduction to identify its cause; environment scrubbing
alone does not establish that filesystem hiding is sound.

Writable exposures use `mount --rbind "$p" "$p"` followed by
`mount -o remount,bind,rw "$p"`. The explicit bind remount clears the inherited
read-only attribute on util-linux 2.39 (Ubuntu 24.04) as well as newer versions,
without remounting the enclosing owner-home anchor or filesystem superblock.
Inherited submounts remain read-only unless separately declared writable.
Login-file binds already use the same explicit remount pattern; private keep
binds preserve their source attributes, and exposed executables and hidden
files remain read-only. CI scratch stays under `$HOME/.cache` so the diagnostic
probe and login-sharing workflow exercise an execution home beneath the owner
anchor. Any bind or remount failure refuses the launch before the agent runs.

## Implemented safeguards

- Reservation checks the exact stored approval, its validity interval, permission
  policy, revocation/use state and current on-disk configuration before committing
  capacity, immutable task/profile/configuration/memory inputs, the task pointer,
  and launch intent atomically. Approval consumption remains a claim-time write;
  admission does not spend the grant. New reservations require schema 13 or newer.
- Launch claims consume exact owner approval. Claim history independently prevents
  replay after any previous claim, including a claimed operation subsequently
  reported as having no effect.
- Generic completion and observation reject caller-written launch confirmations.
  The typed lifecycle receipt service atomically supplies worker identity,
  publishes ownership and changes the attempt to `launching`. Its prepared receipt
  cannot be constructed by deserializing JSON. Generic attempt mutation cannot rewrite a sealed launch's state,
  knowledge binding, reservation, or termination flag.
- Cancellation frees capacity only when the launch was demonstrably never claimed.
  Uncertainty, missing panes, and cancellation requests do not prove termination.
- SIGKILL fixtures cover the uncommitted reservation, committed reservation,
  committed claim, and a simulated external start with a lost acknowledgment.
  Reopening SQLite preserves atomicity, consumed approval, retained capacity and
  refusal to replay. The simulated start is a filesystem marker; this is not live
  Herdr crash certification.
- Receipt commit has separate SIGKILL coverage before and after commit. A lost
  start response can be reconciled using a freshly verified exact worker receipt
  without invoking start again. Recovery preserves pause/revocation, task state
  and capacity; it does not renew execution authority. This receipt version covers
  local non-repository starts; repository/worktree receipts remain separate work.
- A sealed target must be recorded before submission. Receipts must match its
  exact terminal and socket incarnation; a replacement socket at the same path
  cannot be accepted during recovery. Target replay is read-only, target changes
  are rejected, and retained attempts cannot share a staged pane in one store.
- `memory PROJECT attempt-brief --attempt ATTEMPT` renders a complete versioned
  worker prompt without sending it. It uses the attempt's retained instructions,
  task and snapshot, verifies memory bytes, and reports the exact prompt digest
  and character count. It cannot accept replacement knowledge or a larger budget.
- `MemoryStore::create_worker_snapshot` reserves protocol, identity and output-path
  framing before selecting optional memory. Its estimator is
  `char-count-worker-brief-v2`, included in snapshot identity. Older snapshots
  remain readable, but the final worker renderer refuses their obsolete output
  contract and requires a new worker snapshot and approved attempt. The complete
  prompt must also fit its captured envelope.

## Gated resource creation

`canonical_worker::create_resource` now creates a new tab through Herdr 0.9.1
`layout.apply` from an existing sealed reservation for a local, prepared workspace.
It validates the retained full brief, exact configuration/profile/argument digests,
executable identity, route and approval before taking the one-use claim and making
one creation request. It never replaces an existing tab. Unsupported repository,
model, reasoning and environment mappings are rejected before creation. Profile
inspection and resource preparation share definition validation. Preparation also
requires a supported wall deadline, checks the entire retained prompt against the
profile's character-estimated input envelope, and refuses profiles that block
unavailable provider usage. An allowed usage limitation is explicitly retained in
the creation intent. These failures occur before approval consumption or a native
creation request.

The created process is a literal-argv gate inside the bounded Linux supervisor.
The approved executable cannot run until the exact release line arrives; wrong
input or EOF exits. The wall deadline includes time waiting at the gate. Native
pane/process observations establish the exact socket incarnation, terminal and
supervisor identity before recording a version-2 launch target. No start receipt,
ownership or brief delivery is inferred from resource creation. A failed or lost
creation reply retains the original claim and cannot trigger another creation.

Before the request, a durable creation intent records the socket incarnation,
prepared route and command digest. `canonical_worker::reconcile_resource` scans
bounded native pane/process inventories and accepts only one exact live supervised
command in the original socket and workspace. It records a target without changing
the delivery, approval, attempt or ownership. This observation works after claim
expiry, approval revocation or a configuration change; it does not release the gate
or renew authority. Duplicate candidates, changed routes/commands and replacement
sockets are refused. A failed target commit is recoverable through observation.
Creation and recovery compare the stable pane/terminal identity before and after
process observation, refusing replacement during that interval. Once an exact
supervisor has been observed, its pidfds remain held through target commit; an
exit during that interval no longer discards the identity. The termination service
can then confirm quiescence and release capacity without asserting agent start.
Processes that exited before any identity was observed remain uncertain; older
attempts without a pre-effect creation intent cannot use this recovery path.

Version-2 selected targets also support cancellation and process-exit recovery
before a start receipt exists. Confirmed quiescence atomically retires the launch,
releases capacity and clears the task pointer; a failed commit retains capacity
and is recoverable. Selected resources remain in the cross-project inventory even
after this staged termination, until a future explicit cleanup service resolves
them. Termination never silently forgets a pane or certifies artifact preservation.

This service is not yet wired into new-launch controller dispatch. Trusted
profile/preparation ingress, gate-release/start production and workspace creation
now exist. Worktree creation and recovery when no live matching process remains
are still open. Resource recovery
runs through the bounded controller queue and synchronous observation path; neither
path enables launch creation or gate release.

## Gate release boundary

The sealed `record_launch_release` transaction records one input-submission
opportunity for the exact version-2 target, original consumed launch claim and
creation intent. It rechecks current approval, lease, control, configuration,
knowledge, budget and reservation before recording. It refuses any prior release
record, including identical replay after an uncertain input result. A failed commit
records nothing. Release recording alone does not change delivery, attempt or
ownership and does not establish that input arrived or an agent started.

Version-2 gated start receipts now require that exact retained release intent,
with observation times in order. Version-1 historical target handling and exact
replay of previously confirmed receipts remain unchanged.

`SupervisorObservation::waiting_gate` pins and checks the exact canonical shell
child of namespace PID 1. Its live proof fails after the shell execs the agent,
even if the outer supervisor and namespace remain alive. This is process evidence,
not launch authority or a sender.

`canonical_worker::release_gate` now supplies the native sender. It requires an
explicit `execution_home` in the frozen profile and a command that clears inherited
variables before agent execution. The fixed environment contains only HOME, PATH,
LANG, LC_ALL and TERM; credentials stay in the selected owner's credential store.
The home must be a canonical owner-controlled directory outside the project.
The home reference is part of the profile and therefore the signed launch identity;
older profiles remain readable but cannot use the sender without explicit environment
preparation. Arbitrary environment values are not stored in the profile or ledger.

The sender checks retained brief/budgets, exact executable, live waiting gate,
terminal identity and cross-project references, then repeats terminal/process and
approval checks immediately before committing the one-use release record and making
one `pane.send_input` request. A lost reply retains that record and prevents replay.
An OK acknowledgment does not publish ownership or confirm a start. The sender is
not yet wired into new-launch controller admission; trusted profile preparation and
production reservation ingress still need implementation.

`canonical_worker::reconcile_start` now produces a typed start receipt through
observation only. It requires a durable matching release intent, the retained live
supervisor, an exact direct executable and argument digest under namespace PID 1,
and one matching Herdr agent with the deterministic attempt name, exact terminal,
route and no pending launch. It checks the process again after native observation
and cross-project inventory. Script/interpreter indirection and agent argv changes
are deliberately unsupported by this direct-executable observer. It neither renames
agents nor supplies a readiness or prompt-delivery receipt.

The controller's bounded launch-observation path now rotates released unconfirmed
workers alongside uncertain resource creation and termination. Start observation
can confirm an existing process after approval revocation, configuration changes or
claim expiry without renewing authority. Failed receipt commits retain the original
state and can be reobserved; confirmed receipt replay is read-only.

## Concrete brief and termination services

`memory::enqueue_attempt_brief` renders the retained complete prompt and queues only
its digest, character count and immutable identities. `canonical_worker::deliver_brief`
rechecks configuration, approval, knowledge, budget, executable hashes, socket
incarnation, exact supervised worker and readiness before claiming and sending.
The final claim/configuration/knowledge check runs after executable hashing and
request serialization, immediately before native command execution; it is not
reused from before those potentially expensive steps. Only a matching native
acknowledgment moves the attempt to `running`. A lost or
foreign acknowledgment retains the claim and cannot cause a second send. Typed
store recovery can acknowledge an existing delivery without renewing authority;
a native producer for an uncertain prompt receipt is still absent.

`canonical_worker::reconcile_termination` observes or stops the exact recorded Linux
supervisor. Cancellation remains effective while paused or after approval revocation.
Persistent identities bind both pidfs inodes to the boot and observer PID namespace;
recovery refuses unsupported kernels, inaccessible observations and changed views.
New version-2 supervisor identities also retain a domain-separated SHA-256 of
the local `/etc/machine-id`. Reconnect, stop and exit recovery require that host
identity to match, including when the recorded boot differs. Version-1 identities
remain readable and usable in their original boot but cannot prove termination
after a boot change. Missing or changed machine identity retains uncertainty.
This assumes machine IDs are unique to each installation; cloning a machine ID
does not establish a distinct trusted host. It is not remote-host attestation.
Stop recovery reopens each recorded process independently, so an exited namespace
init does not prevent stopping its surviving outer supervisor. Signals use pidfds,
never an unverified numeric PID. Graceful stop uses at most half the remaining
budget (capped at two seconds), leaving time for forced exit observation even when
the original deadline is short. This requires 64-bit Linux with
pidfs and user/PID namespaces, not merely the presence of the helper executables.

Verified quiescence atomically retires outstanding brief obligations, releases
capacity and clears the task's active attempt. Cancellation marks the task cancelled;
unexpected process exit blocks it. Neither establishes task success. Files, panes,
runtime bindings and ownership remain recorded in place; immutable artifact
finalization remains a separate obligation. Failed commits retain capacity and can
be recovered from the persistent process identities after controller restart.

Both synchronous and queued controller paths rotate brief preparation, delivery and
termination work. Confirmed supervised starts without any initial brief operation
produce a preparation hint. A restart re-renders the retained snapshot and queues
exactly one obligation with head/revision and authority checks. Existing operations,
including ambiguous or permanently failed deliveries, prevent replacement hints.
Preparation checks cancellation and the original deadline before publication.
Termination hints are scheduling data, not outbox entries or signal authority.
Root conflict checks now include selected launch targets before their start receipt,
including canonicalized aliases of sockets referenced by legacy threads. Malformed
legacy routing fields block submission rather than becoming empty identities;
missing or invalid thread IDs return errors instead of panicking. Selected-target
retention follows the stored operation-to-attempt relationship before validating
event JSON, so a corrupt payload cannot hide a live resource behind a terminated
attempt. The bounded reader also verifies content-derived attempt/operation IDs,
full retained-input validation and the matching launch operation's task, target,
revision, payload, hash, payload version and idempotency key. Rehashing edited
inputs alone cannot make an inconsistent ownership record pass this scan.

## Work required before enabling dispatch

New resource creation now commits the operation claim, one-use approval consumption
and creation intent in one transaction. A failed intent insert cannot leave a
consumed approval and claimed launch without its recovery identity. The native
request runs only after that transaction returns successfully. SIGKILL coverage
checks both uncommitted boundaries and the committed boundary; trigger failures
verify that no native request escapes rollback. Historical split claims are not
retroactively treated as proof that no resource was created. Exits before live
process observation still require additional evidence and retain capacity.

1. Trusted named-profile preparation: installed executable identities, explicit
   arguments/environment, permission policy and version-bound adapter capabilities.
   Installation version probes alone must not assert launch/readiness/stop support.
   Preparation, native verification, project-bound retention and bounded current
   revalidation now exist. Authenticated live interaction/workflow acceptance is open.
2. Production preparation ingress: repository commit/tree selection, canonical route,
   complete worker snapshot, budget and exact approval scope. No public constructor
   may turn deserialized `FrozenProfile` or `LaunchInputs` into authority.
   `launch draft` and `launch reserve` now provide this ingress and share transactional
   admission checks. Signed fixture acceptance passes; controller connection remains open.
3. Supervised native resource creation and launch: durable effect boundaries for
   worktree/pane creation and agent start; exact typed acknowledgments; persisted
   resource ownership; no blind replay after a lost response. The atomic start
   receipt/recovery store service and gated creation in an existing local workspace
   now exist, with live-target recovery after uncertain creation and direct-executable
   start observation. Native gate submission now exists with an explicit clean
   environment. New workspace creation now has separate retained ownership and
   layout boundaries. Worktree creation, workspace recovery after bootstrap exit, and
   recovery without a previously observed matching process remain open.
4. Add native evidence production for uncertain prompt recovery; never resend it.
   The controller now recovers missing brief preparation after a confirmed start.
5. Extend termination/resource recovery to creation without a recorded target and
   repository worktrees. Recorded version-2 gated targets and confirmed starts now
   have concrete stop/recovery services; retained resources are not finalized artifacts.
6. Integrate preparation/start with bounded controller admission, then certify the
   complete disposable Herdr agent workflow and crash boundaries. Current tests
   establish separate contracts, not the full launch-to-result workflow.

Verified predecessor evidence remains a separate W07 requirement. Tasks with such
dependencies must remain blocked until that producer exists. Nothing in this work
authorizes fallback to legacy thread records or launches in a user's live project.

## Native contract investigation

Inspected upstream v0.9.1, commit `065ef9d6a531c49fb8bee7e818ef837065b21ee9`,
downloaded to `/tmp/herdr-canonical-source`. No upstream source was changed.

- `src/app/agents.rs::start_agent` builds argv from the kind's canonical executable
  name and submits a shell command. It does not take an absolute executable override.
  Agent names are limited to 32 characters; `worker_agent_name` now maps full
  content-addressed attempt IDs to bounded deterministic native names.
- `src/cli/pane.rs::pane_run` joins arguments into shell source and returns a silent
  submission result. It is not an argv execution API or typed start receipt.
  The new POSIX command encoder quotes every argument, including metacharacters.
  The viewer adapter accepts that silent zero exit specifically for `pane run`;
  JSON-returning commands still require their reply. Submit without `--`, which
  Herdr 0.9.1 preserves as shell source. The first encoded word is the quoted
  absolute `/usr/bin/env` path, so it cannot be mistaken for an option.
  Three best-effort visible pane reads share a two-second startup deadline,
  with 64 KiB capture per read. Recognized shell execution errors remove the
  newly created viewer and report it unavailable without failing the worker.
  Dedicated servers best-effort select the retained worker tab after creation
  (including creation after viewer attachment); reopening a viewer selects the
  live retained target on the same socket incarnation. Focus failures never fail
  worker launch. Failed dedicated focus checks or API calls record their reason
  in `.ticker.log` without failing launch. Operator-supplied sessions retain
  owner-selected focus.
  Unsupported reads retain the viewer; this bounded check is not a typed attach
  receipt. The operator-launch CLI lifecycle lab checks exact submitted source,
  persisted ownership, reopen/focus, submission failure, and silent submission
  followed by a shell error. It requires Unix socket permissions.
- `src/app/api/layouts.rs::handle_layout_apply` accepts literal command vectors through
  `layout.apply`. Supplying a workspace and omitting a tab creates a new tab;
  supplying a tab replaces it and is unsuitable for unowned resources. An isolated
  live test verified exact argv, shell metacharacter preservation, supervisor
  observation, persistent-identity reconnect and exit recovery. This removes the
  need to use `pane.run` for exact-executable launch, but does not supply durable
  pre-start target selection or a launch receipt producer by itself.
- `src/pane.rs::shutdown_pane_processes` snapshots session PIDs and signals them
  with bounded grace periods. It does not produce durable quiescence evidence and
  must not by itself release a canonical reservation.

`worker_supervision` constructs a Linux user/PID-namespace command with a required
wall deadline and a five-second forced-stop grace. Detached descendants remain
inside that namespace. This is a command builder, not an execution authority or
an automatic capability certificate. Native admission must still verify helpers,
the installed agent, the terminal's shell/environment and the exact route.

`SupervisorObservation` checks the generated command, executable inode identities,
parent/child relationship and nested namespace PID 1. It retains pidfds for both
processes and an open namespace descriptor through exit and reaping. The live
closure test now exercises this implementation, as well as checking the namespace
has no remaining processes and its detached writer stops. The observer does not
grant stop authority or satisfy artifact preservation, and its handles alone are not
restart-recovery evidence. The new version-2 start receipt additionally retains
boot-, namespace- and pidfs-bound identities used by the concrete termination service.
The OS contract is described in [pid_namespaces(7)](https://man7.org/linux/man-pages/man7/pid_namespaces.7.html)
and [pidfd_open(2)](https://man7.org/linux/man-pages/man2/pidfd_open.2.html).

The native socket transport now applies one absolute deadline to nonblocking
connect, write and read, and bounds both request and reply sizes. It rejects
incomplete or invalid UTF-8 replies and never retries a potentially submitted
request. Tests cover stalled submission, trickling replies, oversized output,
invalid framing and a real temporary socket exchange.

Three opt-in tests passed against the installed Herdr 0.9.1 in temporary homes and
isolated servers: exact process observation/submission/closure, and closure of a
supervised worker with a `setsid` descendant. The latter checks the namespace is
empty and its detached writer has stopped. A library fixture also verifies wall
deadline cleanup independently of controller stop. These establish these specific
contracts, not a completed agent workflow or canonical dispatch certification.


Latest validation (2026-09-20): 329 library tests, 12 memory acceptance tests,
2 contract tests and 9 focused controller tests passed. Three live native contract
tests passed on the installed Herdr 0.9.1 in disposable homes/servers. Native brief
adapter tests use a fixture executable plus real Linux supervisors; they do not
certify an authenticated coding-agent conversation. Logs:
`/tmp/herdr-lifecycle-{library,memory,contracts,controller,live}.log`.

Broader regression run: 504 of 508 binary tests passed. Three cleanup/preservation
tests refused to establish writer quiescence because this host denies inspection
of `/proc/32390/cwd`; the safety check was not weakened. One remote controller
timing scenario failed in the parallel suite and passed when rerun alone. Two
compile-fail doctests also passed. Details are in
`/tmp/herdr-lifecycle-binary.log`, `/tmp/herdr-lifecycle-remote-recheck.log` and
`/tmp/herdr-lifecycle-doctests.log`. The full binary suite is not claimed green.

### Native naming boundary

`canonical_worker::name_started_agent` assigns the deterministic attempt name to
an unnamed native agent after the gate has executed the exact retained executable
and arguments. The request requires the original live claim and current authority,
a matching native route/terminal/kind, an exact supervised process, and a committed
`runtime.launch_name` one-use event. Existing foreign names are refused. The event
does not prove the rename or process start; fresh native and process observations
are required before recording the start receipt.

Naming fences only the target pane's own agent entry, so other workers on the
same Herdr server may change status meanwhile; no other agent may hold the
target's name. Within that entry, only the state a starting agent moves itself
may change: its terminal title (Codex animates a spinner there for several
seconds), `agent_status`, `focused`, and the `state_change_seq`/`revision`
counters, which may only increase. Every other field, including pane, terminal,
cwd, foreground cwd, kind, name and any field Herdr adds later, must be equal. A lost acknowledgment leaves the naming intent intact.
`reconcile_start` observes the expected name without renewed authority. If the
intent is recorded but the exact target agent is still unnamed, recovery sends the
same rename once more, which is idempotent; a different name is refused. If the
claim expired before any naming intent was recorded, no rename was sent and none
may be: recovery closes the launch delivery as a permanent failure with reason
`start_unnamed` and stops retrying, leaving the running worker and its capacity
for the operator to cancel with `herdr-farm task <slug> cancel-attempt <id>
--expected-revision R --expected-head H --reason TEXT` (current attempt revision
and store head, and an operator reason). The explicit naming service stays one-use. An already-confirmed launch reconciles its
receipt directly instead of re-entering creation recovery. Fixture tests cover
naming success, lost reply, foreign-name refusal, failed event commit before
submission, two workers on one server, and recovery of an unapplied rename.

### Bounded launch advancement

`canonical_worker::advance_launch` connects the concrete creation, gate release,
naming and start observation services for an explicitly selected retained launch
operation and delivery revision. It validates the explicit execution home before
creating a resource, then revalidates it before release. It shares one deadline and cancellation signal
across stages. It creates only a never-claimed delivery, expects precisely the
revision produced by that claim, and recovers later stages from durable events.
Uncertain creation is discovered rather than repeated; unknown absence retains
capacity. Recorded release requests are never repeated; a recorded rename is
repeated only while the exact target agent is still unnamed. Expired claims
and uncertain naming take the observation-only start path. Each effect service
revalidates authority and identity immediately before its own durable boundary.

A complete fixture workflow now exercises this service, initial brief preparation
and delivery, cancellation, confirmed process termination, capacity release and
artifact preservation. Lost acknowledgments at creation, release and naming all
recover with exactly one request per boundary. This does not enable automatic
launch admission or replace the pending live vendor-agent acceptance tests.

### Direct-launch readiness

Herdr 0.9.1's `interactive_ready` describes its managed-launch phase and is not
set for the canonical literal-command path. Initial brief delivery instead
requires a positive visible idle rule from the pinned binary's bundled detector
through `agent.explain`, together with exact native agent identity and idle state.
Fallback idle, blocked/busy screens, skipped detection, external detector sources
and warnings refuse delivery. The adapter checks identity again after detection
and repeats readiness immediately before submission. A failure after claiming
retains the one-use claim rather than opening a retry opportunity.

See [the live readiness review](reviews/2026-09-21-native-readiness.md) for the
verified Codex startup/naming/termination contract and remaining certification gap.

### Creating a worker workspace

New launches without a workspace use `workspace.create_command` when the
connected server advertises it (the optional patch in `patches/herdr/`). The
first pane itself runs the bounded, gated PID-namespace supervisor. Stock Herdr
(0.9.1 and upstream HEAD) lacks that method, so the adapter instead uses
`workspace.create` and types one line into the new default shell:
`exec <herdr-farm> launch-exec <spec>`, submitted with an `Enter` key
(stock Herdr does not run a trailing newline in typed text). Both paths are absolute and must
match `[A-Za-z0-9/._-]`, so no shell (POSIX, fish, ...) needs quoting. The
literal supervisor argv and working directory are written first to a private
single-use spec (`.state/launch-specs/<command-digest>.spec`, directory 0700,
file 0600). The launcher requires that private file, its digest-bound name, the
matching argv digest and the canonical supervisor vector, deletes it, changes to
the recorded directory and execs the argv, replacing the shell in place. The
launch counts only once bounded `pane.process_info` polling observes the exact
supervisor as the root pane's process, leaving six seconds of the original
deadline for cleanup. Otherwise (hanging rc files, swallowed input, refused spec)
the adapter deletes the spec and closes the workspace; no workspace or target is
recorded and the retained claim recovers nothing. Version 3 creation intents mark
this transport and recover exactly like version 2; recovery deletes an unconsumed
spec. Upstream support is tracked in
<https://github.com/herdrdev/herdr/discussions/3345>. Production capability
certification remains required before admission.

Version 2 `runtime.launch_creation` intents distinguish this path from historical
bootstrap launches. The exact command digest and server session are retained
before submission. A native root-pane response plus exact live supervisor
observation establishes the target. `runtime.launch_workspace` (target version 2)
and `runtime.launch_target` commit atomically with the same payload; no separate
layout request is made. Lost replies or commit failures are recovered by bounded
workspace/pane/process discovery, exact argv digest and OS process identity, and
terminal incarnation checks. Labels only narrow discovery. Duplicate or changed
layouts fail closed. Observation may recover ownership after revocation/expiry,
but cannot renew authority or release the gate. Missing process evidence remains
uncertain and retains capacity.

The existing-workspace route still creates a gated tab with `layout.apply`.
Repository/worktree provisioning and resource cleanup remain open. Workspace
ownership survives worker termination and does not authorize deletion.

### Historical bootstrap workspace recovery

Historical version 1 creation used
one explicit `workspace.create` request before creating the gated worker tab.
The native response and a fresh pane observation establish the workspace ID and
bootstrap terminal identity. `runtime.launch_workspace` retains that resource
receipt. A separate one-use `runtime.launch_layout` event precedes `layout.apply`;
the layout always specifies the exact workspace and never defaults to the active
workspace or replaces an existing tab.

If the workspace receipt commits but the layout boundary does not,
`continue_workspace_layout` (also used by `advance_launch`) can continue under the
original live claim after rechecking configuration, command, socket, and bootstrap
identity. A lost layout reply uses exact command/process discovery within the
recorded workspace. Neither workspace nor layout requests are blindly replayed.
Loss of the workspace reply or failure to commit its receipt retains uncertainty
and capacity until the exact live bootstrap marker can be observed. A missing,
exited or unverifiable bootstrap remains uncertain; labels alone do not suffice.

Workspace bootstrap receipts participate in bounded cross-project resource
inventory and remain after worker termination. They do not assert that the
bootstrap shell stopped, authorize workspace deletion, or release artifact
preservation obligations. Cleanup must separately prove bootstrap quiescence.
Both staged-stop and started-worker termination transactions refuse to release
capacity when historical root-creation or bootstrap quiescence is unresolved. A
cancellation may stop the exactly identified worker, but its stop alone cannot
mark the attempt terminated or free its reservation. Repeating reconciliation
keeps canonical state unchanged and reports unresolved bootstrap quiescence.
This restriction does not apply to current supervised-root creation or launches
in an existing workspace that did not create a bootstrap shell for the attempt.

For version-2 host-bound supervisor records, local observation of a different
boot on the same recorded host can resolve historical bootstrap quiescence.
The termination receipt retains `host_reboot` evidence with the host hash and
both boot IDs. The store verifies this evidence against the exact retained
supervisor before atomically releasing capacity. Same-boot worker exit, missing
host identity, a different host, or historical version-1 supervisor records do
not establish bootstrap termination. A bootstrap-only creation without a worker
supervisor receipt also remains unresolved. Recovery retains files and resource
references; it neither deletes a workspace nor certifies artifact preservation.
Repository/worktree provisioning is still unsupported by this creation service.

### Recovering a historical bootstrap acknowledgment

Historical marked workspace creation intents contain a random 256-bit marker, committed before
submission and passed as `HP_WORKSPACE_CREATION` only to the bootstrap terminal.
The actual worker still executes with its isolated environment. Recovery examines
a bounded native workspace/pane/process inventory on the pinned server session.
The expected label narrows discovery; a live process must carry the exact marker,
have the expected owner and working directory, and remain pinned by a pidfd while
the native terminal identity is rechecked. Environment contents are not returned,
logged or stored by the observer. Duplicate or altered workspace layouts refuse.

The observation-only transaction retains the bootstrap resource under the original
consumed launch claim, including after expiry or approval revocation. It neither
renews authority nor creates a layout. Continuing the acknowledged workspace still
requires the original live claim and current policy/configuration checks. Missing
markers (including historical intents), bootstrap exit before observation, or
unavailable process evidence continue to retain uncertainty and capacity.

The disposable real-Herdr `live_workspace_bootstrap_marker_contract` verifies marker
propagation, wrong-marker refusal and invalidation after workspace closure. Fixture
regressions additionally cover expired/revoked claims, label-only refusal, duplicate
inventory and receipt-commit rollback without any workspace recreation.

### Native adapter integration evidence

`live_canonical_supervised_root_creation_release_and_termination` runs the actual
canonical services against a disposable patched Herdr server: new root creation,
atomic SQLite workspace/target ownership, exact gate release, process observation
and staged cancellation/termination. It uses a sleep process and fixture approval
and capability records; it is not vendor workflow or production admission evidence.
This test found and fixed the adapter's incorrect `pane.info` call: the Herdr JSON
method is `pane.get`. Fixture dispatch now uses that same method.


### Connected-server creation capability

Before a fresh direct-root launch consumes approval, the adapter sends a bounded
read-only `ping` to the exact socket incarnation. It requires the expected Herdr
version. `capabilities.workspace_create_command: true` selects the patched
method; a missing or false advertisement selects the stock exec-into-shell
launcher, whose typed paths are validated at this point. Repository launches
perform this admission before the worktree claim and repeat it before native
creation. Malformed or version-mismatched advertisements, or launcher paths
outside the safe character set, refuse creation without consuming the approval
or creating checkout paths.

The optional compatibility patch advertises the capability.
Recovery of already-recorded native resources still uses their retained identities;
this check does not retroactively remove resources or replay requests. Existing
workspace launches retain their `layout.apply` path. The advertisement establishes
API availability, not agent workflow certification.


### Prepared-launch controller wiring (production enabled)

`read_controller_dispatch_hint` optionally includes already-reserved launch
operations in the bounded controller rotation. Due, unclaimed launches and live
original claims can advance; cancelled attempts, expired claims, paused projects
and projects requiring reconciliation cannot receive launch-effect hints.
Recovery-only hints continue independently. A launch qualifying for both recovery
and advancement appears once in the rotation. Selection never reserves a task,
consumes an approval, reconstructs a profile or changes canonical state.

The foreground controller routes advancement to `advance_launch`; its executor
uses a separate typed launch action with the original queue deadline and
cancellation token. Worker ingress still validates current signatures, profile,
repository/memory inputs, capacity and every one-use boundary. Launch job failures
back off one second rather than the generic thirty seconds, allowing lost-reply
observation within the original claim lease. This does not renew that lease or
permit effect replay. Other projects retain their admission turns.

`PREPARED_LAUNCH_DISPATCH_ENABLED` is true following the authorized live controller
workflow. Ordinary production polling offers prepared launches alongside
observation, brief and termination work. Current capability evidence, exact signed
approval, capacity and retained inputs remain mandatory. See the current
[enablement audit](dispatch-enablement.md); historical entries below describe the
evidence as it accumulated and do not certify optional agent protocols.


### Creation replies after authority changes

After native creation, storing an exact observed supervisor uses observation
admission rather than requiring a still-live effect claim. The adapter obtains
the current delivery revision/head and retains the typed observation against the
original creation intent and consumed approval. Expiry, revocation or cancellation
can therefore block gate release without discarding the identity needed to stop
the already-created namespace. This write does not renew a claim or consume
another approval. A changed binding or released reservation still rejects it.

The case where a supervisor exits before any exact observation remains unresolved;
an absent pane or process list is not accepted as termination evidence.


### Attempt output and preservation contract

Version 2 worker briefs include `output_directory`, deterministically derived as
`<project>/.state/worker-output/<attempt-id>`. Gate release creates that directory
(owner-only) before the sandbox binds it writable, since the rest of `.state` is
read-only to the worker; workers write `report.md` there, and place supporting
files under `library/`.
This keeps reports separate across concurrent attempts and from repository source
files. Start records the same directory in the runtime binding's artifact source.
The preview and retained rendering use the same path and include it in prompt
budget/hash accounting. Worker snapshots now use `char-count-worker-brief-v2`,
reserving exact path/instruction framing before optional memory selection. Older
snapshot contracts require a new snapshot and approved attempt.

After exact worker termination, explicit finalization uses receipt-bound output
snapshots when available, or the recorded local source for historical bindings
without snapshot evidence, through the bounded, verified artifact pipeline. Cancelled tasks
remain cancelled, with no task revision or disposition change; successful capture
adds preservation evidence only. Other eligible tasks still become awaiting-review.
This captures report/library bytes. Repository-state snapshots described below
cover checkout changes; worktrees and branches remain retained until separate
cleanup acceptance requirements have been satisfied.

Native termination now preserves repository state before committing either a
staged stop or started-worker termination. The receipt binds every approved
checkout to its retained manifest. Working-file snapshots preserve tracked and
untracked non-ignored work; Git-ignored toolchains, caches and build output are
excluded before traversal and are not preserved. The 50 MiB limit still protects
non-ignored working files; Git history/index capture is unchanged. See
[canonical worktree preservation](canonical-worktrees.md) for manifest versions.
Capture or receipt-commit failure keeps the attempt nonterminal and capacity reserved, while retry observes the already
stopped worker and reuses verified snapshots. The same boundary now captures the
attempt output directory into `.state/worker-output-snapshots/<attempt>/<digest>`.
The stop receipt records either its manifest digest or an explicit observation
that the source directory was absent. Because gate release pre-creates the
directory, an existing empty directory is recorded as absent too; a directory
with any entry (an empty subdirectory included) has a manifest. Partial reports, binary library files,
ordinary `.git` output data, executable bits and empty directories are retained.

Output manifest v2 records byte/entry budget omissions as `{path, bytes, reason}`.
Capture inventories metadata first, then prioritizes `report.md`, small files by
size and path, and directories. Retained entries stay within 50 MiB / 10,000
entries; excluded files remain in the source directory. Exclusions are separate
from Git-ignore rules. Worktree preservation still fails hard at its limits.
Termination receipts carry the omission count and manifest digest and commit one `attempt.output_omitted`
coordinator inbox notice. Inspection can read paths, sizes and reasons from the
receipt-bound output manifest. Readers accept digest-bound v1 and v2 manifests. Previously
wedged output captures recover on the next termination or cancellation retry;
no operator reset or schema migration is required. Metadata remains bounded to
30,000 entries, 64 nesting levels and a 4 MiB manifest; unsafe paths, unsupported
entries and concurrent source changes still fail closed.

WORKER-OUTPUT-BUDGET-1 sandbox validation: all six `worktree_preservation`
checks passed; clippy completed with no diagnostics on changed lines. The
`canonical_worker` library run had 2 passed, 67 Unix-socket bind failures and
10 ignored. The CLI suite had 10 passed, 51 Unix-socket startup failures,
3 local TCP bind failures and one fixture-setup store deadline; that deadline
case (`acceptance_decision_replays_in_the_ledger_as_of`) passed in isolation.
The extended completion workflow writes 56 MiB of logs before submitting and
asserts a retained report, omission evidence/notice, intact omitted source files,
and active control after termination. The launch/cancel recovery workflow also
covers the 10,000-entry budget. Both those workflows and the unchanged
fail-hard worktree workflow require the steward's run outside this socket-denying
sandbox; their behavior assertions were not reached here. No schema migration
or live-project action was performed.

Output capture uses bounded descriptor-based traversal, two retained-byte scans, durable
content-addressed writes and readback, with the manifest published last. Unsafe
source/destination paths and publication failures keep capacity reserved for
retry. Foreground and queued finalization verify the receipt-bound manifest and every
referenced blob, then publish report/library bytes through the established
artifact receipt pipeline. They work after the original output directory is lost
and do not recreate it. Corrupt or absent recorded outputs fail closed; they do
not fall back to new source bytes. Historical bindings without output receipts
retain their existing local-source path. Neither snapshot format authorizes
source removal or proves task success.


### Enabled controller effect-path acceptance

Run `scripts/test-canonical-dispatch` on Linux to build both test executables and
exercise the controller's normal effect-selection, queue and executor path with
launch selection enabled only inside the test. Cargo runs locked and offline;
its cache must already contain the dependencies. `/usr/bin/cc` compiles the small
fixture process; Git and ssh-keygen are also required. Disposable sockets and Linux
process namespaces must be permitted. The installed CLI and production dispatch
gate are unchanged.

The library fixture generates a disposable Ed25519 owner key, pins its public key
in the project authority configuration, and installs launch grants through the
production signature importer. A tampered grant must be refused without advancing
the project head or installing approval. Installation preparation, retained-report
revalidation, launch drafting and reservation all use their production APIs. A
changed Herdr executable after signing must prevent reservation without changing
state. The fixture retains repository inputs and supervised test processes while
a separate binary-test process drives the controller. Two
projects launch, receive the exact retained brief once, then stop and preserve
repository/output evidence through a fresh queue. A lost naming acknowledgment
must allow another project's admission before retry and must not repeat creation,
gate input or naming. A third launch is cancelled after queue offer but before
admission; it must not consume approval or create native resources. The disabled
production selector is separately checked to leave fresh launches untouched.

The script also runs the real ticker body with its production background services
against three projects sharing one root, using production polling delays. It
requires positive Herdr/Git observations, brief delivery, a full executor restart,
and cancellation with verified preserved outputs. Exclusive worker effects and
shared-root observation batches alternate so maintenance cannot continually block
launch preparation. Active canonical work uses a 250 ms service interval;
successful termination and observation-only recovery checks cool down for 15
seconds, and idle polling retains
its usual interval. These are scheduling hints; workers still enforce ownership
and claim validity at execution ingress.

Both tests still supply synthetic native capability evidence at an explicit,
compile-time test boundary. The retained profile is not vendor-certified, and the
fixture Herdr protocol does not prove real vendor readiness or prompt acceptance.
The production preparation/draft/reservation-to-ticker chain is covered with those
fixtures; the equivalent live vendor workflow remains required before enabling
the production gate.

### Creation without a recorded process identity

`herdr-farm reconcile <slug> --plan` now reports `inspect_launch_identity`
for retained attempts whose native creation intent exists but whose exact target
or started process was never recorded. It also identifies the affected runtime
binding instead of describing it as an unused binding. A still-live original
claim keeps `wait` advice for the operation; once that claim expires, the report
requires inspection of creation evidence rather than giving generic expiry advice.

The report is read-only. An empty process inventory is not proof that all owned
processes stopped, and neither expiry nor cancellation authorizes recreation or
capacity release. Recovery without any durable process identity remains blocked;
this diagnostic does not provide the missing termination evidence.

Observation-only recovery uses a separate volatile queue identity from active
launch advancement and cools down for 15 seconds after success. Repeated empty
inventories therefore do not monopolize exclusive root access or keep the ticker
at its fast interval. A live original claim can still advance using its own queue
identity and one-second failure backoff. Both current supervised-root creation and
historical tab creation are tested for exit before process identity observation.

### Legacy cleanup beside canonical projects

With `state-store` enabled, legacy worktree cleanup now reads the bounded root
identity inventory, including canonical bindings, launch targets and retained
worktree plans. An unrelated canonical project no longer causes blanket refusal.
Any overlapping local worktree, working-directory or output-directory reference
blocks removal, including missing descendants beneath existing aliases. Dangling
aliases, malformed project metadata and incomplete inventory also block removal.
The check runs under the root execution lease and at the cleanup ingress before
Git commands or a removal reservation.

This does not authorize deleting canonical worktrees. Legacy cleanup still
requires its verified preservation snapshot, writer checkpoint, current Git
registration and non-force removal. Retained canonical references continue to
protect resources after an attempt has stopped.

Reopening an intentionally removed legacy worktree uses the same root inventory.
The target may be absent: existing ancestors are resolved before comparing paths.
A canonical reference acquired after removal blocks worktree recreation, including
references through aliases or to not-yet-created descendants. An unrelated
canonical project permits reopening the retained branch. Conflict refusal leaves
the removal record, Git registration and preservation snapshot unchanged.

### Live native preservation after source loss

The two `live_canonical_*` library tests now verify retained evidence after actual
Herdr supervised creation, gate release and namespace termination. They write a
report and binary library output, stop the worker, delete only the disposable
output source, and load the exact bytes from the receipt-bound output snapshot.
The repository variant also removes its disposable source report and checks the
retained working-file blob, repository manifest identity and Git-pack digest.

Run these live contracts with an optimized build, `state-store`, and
`HP_LIVE_HERDR` pointing to the capability-enabled Herdr binary:

```
cargo test --release --locked --offline --lib \
  canonical_worker::tests::live_canonical_ -- --ignored --nocapture --test-threads=1
```

Operation deadlines match the production controller's 45-second limit; the
original 30-second launch claim and bounded termination are unchanged. On this
machine the unoptimized repository fixture exhausted its claim budget; both
optimized cases passed in approximately seven seconds total. These tests use
isolated homes and sleep processes, not authenticated vendor agents, and do not
certify vendor readiness, prompt completion or the complete worker workflow.

The same live-test filter also runs
`live_canonical_lost_creation_and_gate_replies_preserve_one_use`. Its transparent
CLI proxy forwards each request to the real server, then drops one successful
`workspace.create_command` or `pane.send_input` reply. Creation recovery must
observe the existing exact supervisor; creation cannot be replayed. A lost gate
reply retains the one-use release record and a second release attempt must refuse.
Both cases prove exact process execution and safe cancellation, and run the same
receipt-bound source-loss preservation checks. Only method names are logged by
the proxy; no authenticated vendor prompt is involved. This is acknowledgment-loss
acceptance, not a substitute for controller/host-crash or vendor workflow tests.

`live_canonical_caller_crash_after_creation_and_gate_input` runs each effect in a
separate test process. After the real server returns success, the reply proxy holds
the response while the parent sends SIGKILL and reaps that caller. Only then is
the proxy released. The test waits for inherited root-lock cleanup before opening
the durable state for recovery. Both creation and gate-input cases retain exactly
one request and one durable intent, refuse replay, and pass exact-process stop and
source-loss preservation checks. This covers caller crashes at those two native
effect boundaries; it does not claim host reboot, Herdr restart, or authenticated
vendor workflow acceptance.

`live_canonical_server_restart_preserves_old_outputs_and_new_worker` kills and
reaps the disposable Herdr server, then starts a replacement at the same socket
path. A new supervised worker deliberately receives the old pane ID. Its session
birth identity and supervisor identity must differ. Cancelling/reconciling the old
attempt must settle only the old supervisor, preserve its outputs and leave the
replacement session/worker alive. Repeating reconciliation must leave the snapshot
unchanged. The test then explicitly stops its replacement worker for teardown.
This supplies live Herdr-restart evidence for staged canonical attempts; it does
not establish authenticated vendor readiness or started-worker workflow recovery.

### Live exit before recorded launch identity

`live_canonical_unobserved_exit_keeps_capacity_and_refuses_replay` forwards a
real `workspace.create_command` request and drops its successful reply. Only the
disposable harness retains that request/reply to locate and stop the exact native
supervisor. Its identity is never supplied to the production store. Recovery
then sees the exited worker without a recorded launch target.

Repeated creation is refused, observation cannot reconstruct an identity, and
termination refuses the missing staged target. The same checks run after explicit
cancellation. Each pass must leave the full store snapshot unchanged, keep capacity
reserved, retain one consumed approval/creation intent, send no gate input and
preserve the approved checkout bytes. This covers externally induced native exit
before durable identity capture; it does not certify a spontaneous vendor crash
or authorize automatic cleanup of ambiguous resources.

All six optimized live canonical contracts passed together on 2026-09-22
(21.26 seconds), with the same documented capability-enabled Herdr binary and no
credentials. Production dispatch remains disabled pending the remaining audit.

### Live historical bootstrap refusal

`live_canonical_historical_bootstrap_blocks_release_after_worker_stop` uses the
existing test-only historical creation ingress against a real disposable Herdr
server. It recreates the non-repository bootstrap-plus-worker layout, releases
the gate and observes the exact worker executable. After cancellation, production
termination stops the supervised worker but refuses capacity release because the
bootstrap has no durable descendant-quiescence evidence. The test independently
identifies that still-live bootstrap by its creation marker and working directory.

Two termination attempts leave the full store unchanged, capacity retained and
report/library bytes intact. No cleanup or successful preservation receipt is
fabricated. This validates the visible blocked disposition permitted by T03.4;
it does not add automatic historical cleanup. Current production creation uses
one supervised root and cannot create this historical bootstrap layout.

All seven optimized native contracts passed together on 2026-09-22 (25.19 seconds).
No authenticated vendor prompt or real host reboot was involved.

### Authorized interaction diagnostic

On 2026-09-22 the explicitly authorized isolated Codex 0.154.0 diagnostic passed
(16.99 seconds). It copied the login into a disposable home, trusted only the
exact disposable work directory, kept the read-only sandbox, submitted the fixed
no-tools token prompt once, verified the native acknowledgment and supervised
termination, and asserted removal of the temporary home/login copy. It did not
wait for a token reply and does not establish task or memory protocol behavior.

Initial attempts failed before prompt submission: extra temporary-path nesting
caused workspace creation failure; the original shorter layout worked, but the
broad `/tmp` trust entry did not establish trust for the specific working directory.
The fixture then provisioned that exact path before calling the shared verifier.

That workaround is gone (W-COORD-1): `profile verify-interaction` now runs the
agent in a fixed empty directory under the execution home
(`<home>/.hp-verify-work`) and trusts exactly that directory in the home's own
agent configuration (`.codex/config.toml` for Codex; `.claude.json` and
`.claude/settings.json` for Claude Code). Verification still never touches the
owner's real `~/.codex` or `~/.claude`; it writes only inside the dedicated
execution home. The same preparation pins the profile's model and reasoning
effort and the verifier reads them back after readiness (see
[profiles.md](profiles.md#model-and-effort-pinning)); the evidence's
`interaction.pinned` records them. Launch, readiness, prompt and stop semantics,
and the version and digest binding of the evidence, are unchanged. API errors
now identify the failed method without exposing output; readiness timeouts retain
the last validation error. Test-only screen diagnostics emit fixed categories,
not terminal contents or credentials.

### Claude Code as a canonical worker kind and the shared login (W-COORD-1)

`claude` is launchable beside `codex`. Before the gate is released the controller
prepares the execution home for the kind (pins, worker permission defaults, trust
for the attempt's worktree; `permissions.additionalDirectories` carries the
attempt's spool, output and Git directories, the Claude analogue of the
`writable_roots` argument Codex receives). Launch, readiness, prompt, stop and
result submission (`result submit` through the attempt's submission spool, then
`result capture`) are the agent-agnostic native paths; only the kind string,
version format (`2.1.0 (Claude Code)`) and the home configuration differ.

The Codex worker authenticates through the owner's single `~/.codex/auth.json`, bound
read-write into the isolated home (Codex rewrites it in place, so the bound inode stays
current; see [profiles.md](profiles.md#shared-login)). A Claude worker does **not** bind
`~/.claude/.credentials.json`: Claude Code renames a new file over its credentials, which
left the worker's mount with an expired inode ("Login expired" minutes after a passing
verification). Its login is a long-lived setup token read from the file named by
`[worker_isolation.login] claude_token_file` in the pinned owner configuration and passed
as `CLAUDE_CODE_OAUTH_TOKEN` in the agent's environment only (via a descriptor opened
before anything is hidden, never an argument, file or report). A Claude profile without
one is refused by `profile verify-interaction` and `launch run`. No credential is copied
and the rest of the owner's agent directories stays hidden;
`tests/worker_login_share.rs` proves all of it through the real sandbox.

### From "launched" to "landed" (W-COORD-1e)

The first real canonical trial (`tactics-dev`, a Codex and a Claude worker) exposed seven
gaps between a launched worker and a landed result; each is fixed and has an end-to-end
test.

1. **Claude login**: setup-token file instead of a bound credentials file (above).
2. **Codex login**: it persists `auth.json` in place (truncate-and-write), so the bind
   stays valid; an owner refresh is visible to a running worker
   (`tests/worker_login_share.rs`).
3. **Brief delivery is confirmed only when accepted.** After `agent.prompt` the
   controller watches the agent for up to 6 s: its status leaving idle (working, or
   blocked on a dialog of its own) or Herdr's visible-screen detector seeing it working.
   Still idle (a startup banner swallowed the text), it re-sends the same brief, up to
   three deliveries inside the 30 s claim lease, and then leaves the delivery
   **ambiguous** (never confirmed, attempt not running) with an operator message; the
   operator inspects the pane and retires the brief or stops the attempt.
4. **Result submission.** The brief ends with an exact shell script (values from the
   installed contract filled in) that commits the declared outputs on the attempt branch
   and runs `herdr-farm --root ROOT result PROJECT submit --input-file DOC` through
   the attempt's spool; the worker must see `submission_id` before replying DONE. For a
   worker that finishes without submitting, `herdr-farm result PROJECT submit-captured
   ATTEMPT` captures its worktree, builds the submission from the capture (frozen contract
   revision and digest, base and candidate OIDs, artifact manifest = the blob of every
   declared output at the candidate; objects staged from the repository) and records it
   under a key bound to the candidate, so repeating it replays. With
   `--integration-ref` automatic verification and integration then proceed: with
   automatic verification on, `submit-captured` enqueues the verification job itself (as a
   spooled submission does on the ticker's next turn), so the worker need not exit and the
   attempt remains running while verification is pending or rejected. After every
   acceptance policy accepts, the ticker requests completion through the same path as
   `task PROJECT complete TASK --expected-revision R`. A `verify_then_integrate` route
   first requires recorded successful integration and current merged-output checks.
   Automatic completion requires verify automation on, plus integrate automation on
   for that route. With a required switch off, `task PROJECT list` reports that the
   operator must complete it. Verification errors and rejections never complete it.
   Completion stops the worker and proves its termination before releasing capacity
   and marking the attempt `completed` and task `succeeded`; telemetry `active_ms`
   ends there, without charging the remaining wall budget. Requests are idempotent
   across ticker restarts and operator completion races; cancellation takes priority.
   Seeded candidates and unselected candidate-group submissions remain held.
   A canonical worker job (brief, preparation, termination) that lost a lock race or hit a
   transient failure is retried after two seconds, not thirty; project ownership itself is
   still taken without waiting, so status reads and exclusion checks never block. Configuring integration and enabling automation before a reserved
   launch therefore cannot park its preparation behind a lock race; a failure that remains
   is written to the ticker log (`.ticker.log`) with its reason.
5. **`launch run` re-acknowledges an edited owner configuration.** The `project_control`
   step compares the digest control acknowledges with the current configuration and, when
   they differ, reconciles and sets the project active again with the expected revision
   and head (reported as `done` with `owner_configuration_reacknowledged: true`).
6. **Permission mode**: preparation sets the profile's mode and the other keys it owns on
   every launch ([profiles.md](profiles.md#permission-mode-and-the-other-keys-the-product-owns)).
7. **Dedicated servers are stopped.** `launch run` records the Herdr server it starts
   (`.herdr-run/PROJECT-TASK/herdr/server.json`). The ticker stops it, only after proving
   the recorded process is still that server (a `herdr server` process whose environment
   names the recorded socket), once the task has no unfinished attempt, and removes its
   socket directory; `herdr-farm launch PROJECT stop --task ID [--force]` does it
   explicitly (refused while the attempt still holds its worker unless `--force`).

### One operator command

`herdr-farm launch PROJECT run --task ID --profile NAME --repository REPO
--plan-output docs/FILE.md --prompt-file FILE [--sign-with KEY]` performs the whole
operator sequence on a migrated project and stops at the first failing step; a
rerun skips finished steps and never reserves a second attempt. `--prepare-only`
stops before drafting and reservation. New local task bindings without resource
references preserve active control, so tasks can be added while other workers are
reserved or running. Launch records fresh evidence; bindings naming existing
resources still require reconciliation. Explicit owner pauses block launch.
See
[operator-runbook.md](operator-runbook.md).

### Large repository result submissions

The closing POSIX sh script and `result submit-captured` list the candidate's
reachable objects excluding everything reachable from the frozen base, then add
both commit anchors. The object list is sorted/deduplicated; unchanged files do
not consume the submission budget. Replay still binds the exact document and
retained bytes to its idempotency key.

We choose option (b), importing the base from the repository in the signed
contract. A standalone pack (a) would repeatedly transport the unchanged tree;
combining packs and base reuse (c) adds a second ingress format without improving
this workflow. Existing loose-object ingress already snapshots packed objects
into private retained bytes. No schema or retention classification changes are
needed: these remain existing `result_objects` and factory-object artifacts.

The verifier initializes a private repository, fetches only the pinned base
from that owner repository with object checking enabled, and overlays retained
submission bytes. It never uses the attempt worktree or quarantine as an object
source, and leaves no alternate. Full strict fsck checks object hashes and
connectivity before checkout. A wrong hash or a missing referenced tree/blob is
refused with `candidate object hash mismatch or missing referenced object`.
The checkout must resolve to exactly the claimed candidate.

The delta budget is derived from ingress bounds: the 256 KiB JSON budget
divided by a 256-byte object-entry allowance gives 1024 object entries. Each
compressed loose object is limited to 16 MiB; four times that per-object budget
gives a 64 MiB aggregate retained-byte limit. Both producers report the object count when exceeded;
ingress reports byte-bound refusals. The trusted base is independent of that
budget; fetching it uses the verifier's existing 30-second Git command deadline.
Large changes beyond these bounds need smaller task submissions.

Compatibility: legacy environment variables and existing data/config locations remain supported; see [renaming](renaming.md).

### Canonical coordinator launch adapter (W-COORD-2)

Coordinator `open` retains operator intent and project ownership across Herdr
calls and priming waits. Root exclusivity covers only conflict validation and
binding publication, with no Herdr I/O. Replacement relinquishes the old claim
under the existing project guard rather than reacquiring a shared root lock.
An absent or uncertain pane returns to the owner for `open --reprime`; the
canonical ticker does not replay coordinator startup or priming each pass.
The E2E replacement workflow requires foreground reprime to finish within five
seconds while the accelerated ticker is running.


`open PROJECT` now supports a migrated canonical project. It uses a durable
version 1 coordinator effect journal, canonical runtime binding/observation and
operator-authorized adoption; no new schema or tables. Priming shares native
agent identity and visible-readiness validation with worker briefs and confirms
acceptance only after leaving idle, with at most three sends. A closed pane is
recreated by `open PROJECT --reprime`. Canonical context no longer requires a
planner profile for the full state view. Thread commands deliberately refuse
with signed task/launch and result/finalization replacements. The coordinator
obeys the existing project safety settings; signing uses `--sign-with` or the
owner's configured `[coordinator].signing_key`. See the
[operator runbook](operator-runbook.md#canonical-coordinator-after-migration-w-coord-2)
and `skill/COORDINATOR.md` for CLI forms and recovery rules.

The `state-store` feature is enabled by default; build recipes above use that default.


Workers may end their attempt output `report.md` with `## Remember`, containing
short durable project lessons as evidence, never instructions. Explicit result
submissions carry this report in the optional `report` JSON string; operator
`submit-captured` reads the attempt output report. Both create one candidate per
attempt/content digest for coordinator review. See [canonical Remember capture
and delegated decisions](memory-store.md#canonical-remember-capture-and-delegated-decisions)
for the 2026-10-04 owner delegation and mandatory owner inbox/Herdr notification.

## Dedicated server retirement

Each ticker pass scans persisted `.herdr-run/<project>-<task>/herdr/server.json`
records, including after a restart. It stops a private server when no attempt
for that task retains its worker, without depending on the task's active pointer
or the continued existence of its task row. Previously the sweep skipped missing
task rows, stale active pointers and servers without attempt history. Retained
attempts still fence cleanup, including uncertain termination. Stop reads the
socket from the persisted server record rather than recomputing a short socket
path, and verifies the recorded process's server argv and socket environment
before signaling. Retirement holds the project effect guard and an exclusive launch-preparation
file lock, so it cannot race a reservation or a launch that has created its server
but not yet reserved an attempt. Relaunch also reuses the persisted live socket when the
runtime directory changed, and refuses to overwrite a live server record until
the retention fence permits stopping it. Operator-managed servers remain running.

Retirement runs on the next completed ticker pass after those fences clear,
including for pre-deploy records discovered after restart. This covers launch
failure before agent creation, proven dedicated-server loss, cancellation,
relinquishment and queued supersession, including brief authority/readiness
failures. A live attempt also protects a recorded socket through its current
binding when the binding's task identity has moved. Each private retirement
records a typed `runtime.dedicated_server_retired` event (task, PID, socket,
record incarnation and recorded viewer) in the existing journal before removing
`server.json`; retries of that identity do not duplicate the event. No store
migration is needed.

The sweep reads a read-only canonical snapshot without a whole-store integrity
check. Idle monitoring likewise uses persisted pane observations, leaving pane
probes and the durable response to unexplained loss to reconciliation. A missing
pane without proven termination pauses admission and requires reconciliation;
it never makes a capacity-retaining attempt's server eligible for retirement.

### Worker command card and environment

Rendered worker briefs end with a compact command card: the submission script
above it, canonical memory commands with the projects root, project slug and
attempt filled in, and the literal report and library output paths. Workers do
not need to explore `herdr-farm --help`. Submission skips unused declared write
paths that do not exist, while staging tracked deletions and still requiring all
declared outputs.

The isolated worker environment sets `HERDR_FARM_WORKER_OUTPUT` to its writable
attempt output directory and supplies `GIT_AUTHOR_NAME`, `GIT_AUTHOR_EMAIL`,
`GIT_COMMITTER_NAME` and `GIT_COMMITTER_EMAIL`. Repository identity wins;
otherwise the identity is `herdr-farm worker <attempt>` with email
`worker@herdr-farm.invalid`. No global Git configuration is required to commit
or merge. With no explicit root flag or root environment override, worker CLI
commands resolve the projects root from `HERDR_FARM_SUBMISSION_SPOOL` (or its
legacy alias), so memory commands reach the canonical project from an isolated
home. This resolution does not grant any additional write authority.

## Wall-budget termination and contention (schema 72)

The namespace `timeout` includes gate waiting and kills the worker at the approved
profile wall budget. No agent or terminal disappearance alone releases capacity.
Termination reconciliation reopens both retained supervisor incarnations; ESRCH,
a different pidfs incarnation, or a signalled pidfd proves the original process
ended. Inaccessible observations remain unresolved.

The existing attempt-input frozen profile supplies the wall budget through
`frozen_definition` and the launch wrapper’s `validate_gated_preparation`. A proven
unrequested exit observed after the creation time plus this budget records `timed_out`; this is an elapsed-budget classification, not a
captured timeout exit status. Earlier exits remain `process_exit`. Historical
attempts whose frozen definition cannot be read or validated keep `process_exit`.
Cancellation and completion requests retain precedence. No budget is inferred
when the frozen configuration is unavailable or changed.

The ticker derives termination hints from `termination_observed=0` on every pass,
including while paused. Lock contention records no completion or permanent failure;
the queue retains pending work and retries. Pending live-process observations also
remain pending rather than being reported as completed jobs. Restart reconstructs
hints from the store and the exact retained supervisor identities. The successful
transaction records quiescence, preservation, terminal attempt state, capacity
release and the usual coordinator ended notice atomically, making replay idempotent.
The ticker logs project and attempt on contention deferral and successful recording.

The launch claim lease is 180 s (`timing::launch_lease`; labs keep 30 s,
`HERDR_FARM_LAUNCH_LEASE_SECS` overrides within 1..=300 s). Every launch stage
retakes the exclusive root with a two-second wait, and an operator command that
holds the root for its whole preparation (`launch run` refreshing profile evidence
runs an agent probe for up to two minutes) made the historical 30 s lease expire
before the naming intent was recorded, closing the launch as `start_unnamed`
(LAUNCH-STALL-1). A launch whose ticker really died is still recovered at lease
expiry, as before. The server sweep retains servers only while an attempt retains
capacity, launch preparation holds its lock, or a launch delivery is pending,
claimed or ambiguous. Queued status alone does not retain a server; cancelled,
finished, superseded and never-reserved launches are eligible once these fences clear.

A long isolated verification or integration check (minutes for a Godot test run)
holds the shared check lock `.check.lock`, not the root barrier, so termination,
launch and brief stages take the exclusive root beside it instead of deferring
until the check ends (TERM-RECORD-1). Root maintenance and cleanup still refuse to
run beside a check. One ordering rule remains: an exited worker whose own
submission is still being verified or integrated keeps its attempt open until that
verdict is recorded, because the termination moves the task revision the running
check is fenced on; the next observation after the verdict records it, with no
contention and no cancel.

### Relaunch contract scope and overlap refusals

`launch PROJECT run` reuses an installed task contract on reruns. If supplied
`--write`, `--output`, or `--accept` inputs change its scope, outputs, or acceptance
policies, launch refuses before preparation and names the old and requested
values. The same check applies to `--plan-output` and `--contract-file`. Use a new
task id with `--supersedes OLD` for corrected contract decisions; a rerun that
omits contract inputs continues to use the installed contract.

An active attempt retains its write claims. A conflicting launch reports
`launch run stopped at step N (launch failed: refused: ... resource_conflict: ... overlaps ... )`
on one line, including the conflicting paths and holding task/attempt, so filters
for `refused` or `error` retain the cause. Resolve the conflicting work before retrying.

### Conflict inventory bounds

Root-wide ownership checks (including `open --reprime` and legacy brief delivery
across canonical neighbours) and worker pane/worktree
allocation share a ceiling of 16,384 candidate records, 50 MiB of charged input,
and at most ten seconds per scan. Each reader consumes the same root budget;
creating a new project does not reset it. A separate per-project allowance would
multiply the total read budget, so this hotfix retains one bounded root scan.
Bindings and target rows each consume a record; worktree creation rows and each
retained plan consume records separately. Root enumeration remains bounded to
1,024 entries and legacy thread enumeration to 256 entries per project. Errors
identify the reader, project and consumed record/byte count.

`runtime.launch_release` records the start-gate release that lets the agent begin,
not a resource release. Workspace history and bindings remain conflict references;
started, terminated launch targets retain the existing eligibility rules and
pane reads use the retained launch-resource projection. Retiring finished launch
history requires a real close/release signal and is follow-up work (LAUNCH-BUDGET-1).
No canonical schema, backup or retention classification changes are made.

E2E coverage uses two isolated published projects with more than 1,024 terminated
launches that remain counted, checks live pane conflicts and shared-budget errors,
and extends the public coordinator reprime and worktree preparation workflows.
The latter two need local Unix sockets and must run outside sandboxes that forbid
binding them.
The socket-free public inventory regression also seeds 1,040 launches in the
second project and scans worktrees, launch targets and bindings with one shared
16,384-record budget. Historical fixtures retain version-2 profile evidence,
matching approval grants and consumed uses, and confirmed delivery claim history;
the effective-profile insertion guard remains enabled while seeding.

`launch PROJECT run` records `elapsed_ms` for each completed step in its JSON
report and prints the timing beside the step outcome. Refusals name the current
step, its elapsed time, and the completed steps with their timings. The
`--budget-seconds` admission window defaults to 600 seconds (1–3600 accepted),
measured from the beginning of the command, including preflight and refresh.
Draft and reserve each receive a fresh 60-second deadline, capped by that window;
approval signing/import no longer consumes reserve's 60 seconds. Budget-only
failures during draft or reserve retry internally at most three times with fresh
step deadlines, without restarting completed steps. Other failures retain the
existing refusal behavior. The window bounds admission deadlines; filesystem
calls and the existing signing subprocess are not forcibly interrupted by it.

Launch repository observations reserve their full five-second command budget
plus five seconds of supervisor cleanup before starting. Insufficient remaining
budget is a retryable admission error, never evidence of a completed transfer.
Draft creates no reservation, and reserve commits atomically after validating
its live proof, so a budget refusal leaves no partial reservation. The final
refusal includes the chosen budget and retry count. No store schema changes are
needed.

Deadline trace: `--base` is resolved locally by gated Git during contract creation
(no base fetch in `launch run`). Draft/reserve Git observations use the live
profile proof deadline through `supervision::run`. Retained profile revalidation
caps that deadline at 60 seconds; time waiting for its root/check locks counts
against the step budget. Worker worktree preparation is a later operation with its own caller deadline capped
at 45 seconds and subsequently by the worktree claim lease; its supervised Git
commands are capped at 20 seconds. Copy jobs use their executor's absolute
`Control` deadline, capped again at execution entry by the command timeout.
Neither later operation inherits the operator draft/approval deadline. Native
profile refresh uses a separate 120-second deadline capped by the launch window.

### Launch conflicts and vanished dedicated servers

Launch advancement reselects the current delivery revision on a store conflict,
with at most three passes through the durable one-use phase boundaries. Worktree,
workspace, gate and naming intents remain the replay fences. Brief preparation
and delivery also reselect on conflicts; an uncertain prompt is never resent.
Exhausted conflicts are recorded as `launch_conflict` in the existing event
journal and delivery diagnostic. A reservation with no Herdr creation intent releases
capacity, retaining any worktree receipts and checkouts. An uncertain effect retains its evidence and capacity for recovery.
One stable `attempt.launch_failure` inbox item names the attempt, phase, error
and recovery (`relaunch` after release, or `task cancel-attempt` with fresh
revisions). No canonical or sidecar schema change is required.

Resource recovery proves a dedicated server lost when the socket directory is
absent and a `/proc` scan finds no process naming that socket in
`HERDR_SOCKET_PATH`. The route must come from this launch's farm-recorded
`runtime.launch_creation`. The `.herdr-run/<project>-<task>` directory must be
owned by the current UID or itself absent. When its `herdr/server.json` is
present, the owner-controlled record must match the recorded socket and
project/task and contain a valid PID; a missing record permits the same proof.
A present unreadable, invalid, mismatched or operator-managed record fails closed.
Dedicated-server creation and viewer updates write owner-only (0600) records
independently of the host umask, including when rewriting an existing record.
Unreadable process evidence owned by the current UID also fails closed. A missing socket alone remains retryable. Proven disappearance
ends the attempt `lost`, releases capacity and retains worktree receipts and
checkouts. `task cancel-attempt` can use the same proof, including after an earlier
cancellation request that retained capacity.

Reconciliation validates the caller's expected delivery revision before recording
server disappearance. A stale caller revision, a store selection/commit error,
and a failed native start confirmation propagate without launch-failure events.
Only errors observing the socket pinned by a validated creation intent enter
bounded recovery escalation. SQLite trigger-aborted writes are failed commits,
not revision conflicts; brief expiry and aborted commits leave no new events.
Expired or cancelled brief retries also return before conflict notification.

Three identical ENOENT, permission or invalid-route socket observations stop automatic
recovery with a permanent delivery diagnostic and one coordinator notice. Without
proof of resource absence capacity stays held. The `launch_stalled` health rule
warns after five minutes without launch progress and is critical after fifteen
minutes or when recovery stops with capacity retained. Operators should inspect
the recorded effects and cancel the attempt rather than create another worker.

Started launches awaiting a confirmed `runtime.worker_brief` remain subject to
`launch_stalled` (warning after five minutes, critical after fifteen). Later launch
events do not refresh the first start timestamp. Background brief job failures
are retained in the existing event journal and included in health evidence.

`insufficient transfer cleanup budget` remains the typed `BudgetExhausted` path
from LAUNCH-BUDGET-1: draft and reserve have three fresh bounded step budgets and
never retry an uncertain external creation.

Dedicated server retirement records the disappearance proof before removing its
server record. A subsequent `cancel-attempt` on that proven lost attempt is an
idempotent success. Failed attempts with uncertain terminal effects keep their
task pointer and capacity so the existing cancellation and termination observer
can still retire an acknowledged gated worker.

Proven launch-server disappearance also retires every runtime ownership claim for
that attempt and its observations in the launch-failure transaction. The existing
`runtime.relinquished` audit retains the ownership identity and disappearance
proof; runtime bindings, worktrees and external resource references remain intact.
This uses the same retirement code as the normal proven-termination observation
path, so terminal launch history cannot repeatedly invalidate project control.

A sealed brief control epoch or configuration digest mismatch is permanent for
that attempt (`brief_authority_stale`). Preparation checks this fence before
rendering retained knowledge, and delivery checks it before submission. The
attempt fails once with one coordinator launch-failure notice, rather than
retrying unchanged sealed inputs. Expired or changed consumed approvals fail as
`brief_approval_expired`; other permanent authority `Invalid` errors fail as
`brief_authority_invalid`. The supervisor stops a failed started worker, preserves
its outputs, and records proven termination before releasing capacity and the task
pointer. Only the typed `brief_approval_expired` failure also retires runtime
ownership in that same termination transaction; a failed commit rolls back the
retirement and its audit event. Revoked barriers and other authority failures
retain ownership for preservation and review. A coordinator can then launch a
replacement with fresh signed approval using a new task id, `--work-item WORK` and `--supersedes OLD`,
without pausing project control. The superseded task must already have matching
work-item lineage; lineage cannot be added after a task's first attempt.
Same-task reruns require matching retained lineage and the installed contract.
No prompt is sent under invalid
authority. No store schema migration is required.

Previously both the project inventory and root queue rotated launches and briefs
with equal priority, allowing neighbouring exclusive launch stages to displace
ready briefs. Brief preparation and delivery now take priority over new launch
advancement in the project hint inventory and across the root's background queue. An already admitted
root-exclusive operation finishes first; priority applies at the next admission,
with rotation retained among briefs. Approval expiry remains a hard authority
boundary, including while queued. The default signed validity is 900 seconds
from draft issuance, not reservation or worker start. Root jobs retry launch
failures after 1 second and brief lock failures after 2 seconds; persisted delivery
backoff grows from 1 second to 256 seconds. Readiness rejection occurs before
claiming a brief, so its zero-attempt delivery does not accumulate that persisted
backoff; the volatile queue delay and project/root rotation govern those retries.
In the reported 2026-10-11 incident,
launch confirmation around 09:40Z preceded the first observed expiry at 09:59:37Z
by about 19 minutes; the 09:42:34Z preparation contention preceded expiry by
17 minutes 3 seconds. The intervening readiness rejection also shows that contention alone cannot
explain the entire gap. These log bounds do not reveal the draft issuance time or
the duration of each neighbouring exclusive stage. The isolated E2E contention
lab holds the root exclusively for 3 seconds and reports elapsed time to Running;
the expiry lab waits past its actual 120-second signed grant and verifies supervised
retirement and a successful launch of a new task superseding the failed task in
the same work item, with persisted lineage and no operator recovery action.

For stores already affected by #52, observation also recognizes the retained
terminal `attempt.launch_failed` proof with reason `dedicated_server_lost`, a lost
attempt with observed termination, and matching sealed attempt/binding identity.
It retires the leftover claim through the same audited relinquishment without
invalidating control again. No access to a live worker or owner data is needed
for this repair.

Cancellation after proven dedicated-server loss retires the attempt's ownership
and observations in the same transaction and emits `runtime.relinquished` with
terminal disappearance and cancellation evidence. Observation repairs pre-fix
cancelled leftovers only with observed termination, matching sealed attempt/binding
identity, a released cancellation journal entry, and the exact dedicated-server-loss
outcome recorded by that sealed operation's cancellation path. Ordinary cancellation
or endpoint absence alone does not authorize this repair. No schema change is needed.

The LOST-OWNERSHIP-1f observation repair first uses an indexed attempt-ID lookup
for state and termination. Cancelled candidates additionally require a permanently
failed launch delivery. Live attempts and normally terminated operator-cancelled
workers do not decode sealed launch inputs or scan journal JSON on this path.
Only candidates with the exact recorded dedicated-server-loss proof read the
sealed input to match the ownership binding before audited retirement.
Cancellation's disappearance recovery is restricted to capacity-retaining
reserved, launching or failed attempts; a running worker follows normal
cancellation and termination observation even if its server later disappears.
This hotfix adds no schema or dependency changes.

LAUNCH-LEASE-EXPIRED-1: the operation claim is shared by worktree preparation,
workspace creation, readiness, release and naming. Previously its fixed expiry
was set once at worktree creation (or native creation without repositories).
Repeated bounded advancement could spend the remaining lease before creating an
agent. New claims cover the remaining unscaled phase deadline plus five seconds
for receipt persistence, with the configured launch lease as a minimum. Each
live creation continuation, release or naming stage extends that same generation
under its existing root guard only after selection and preflight establish that
external work is needed, immediately before committing its effect intent, recording
`operation.lease_extended` with reason `launch_phase_budget`. It preserves owner,
epoch, revision and attempt count and checks current binding and approval.
Confirmed starts, retained-worktree observation, repeated one-use calls, refusals
and no-op naming do not renew or append lease events. Read-only replays preserve
the delivery row and event head, including the existing stage rotation order.
Renewal for worktree-to-workspace creation, gate release and naming commits in
the same transaction as the effect intent; a refused or failed commit rolls both
back. A selected legacy workspace continuation renews immediately before its
layout effect.
Expired claims are never renewed or replayed: observation recovers an existing
exact resource. An observed target without a release intent ends promptly with
typed `launch_lease_expired` failure and a coordinator/operator diagnostic. The
existing supervised termination path stops the gated worker, proves quiescence
and retires capacity and ownership. The finished-task sweep then stops a recorded
dedicated server. Before a target exists, ambiguous creation observes the
native workspace inventory. An inventory with no workspace for this attempt
ends with a typed failure; an
automatically managed dedicated server is cancelled using a pinned process
handle before capacity is released. A present workspace retains the existing
identity adoption path. Once an unreleased target exists, termination observation takes
priority and records the typed expiry failure exactly once; its next pass stops
the worker using the updated attempt revision. Cancelled staged launches also
select termination alone. Released launches continue start observation.
A claim always commits a worktree or native creation intent atomically; there
is no claimed launch with neither boundary. Naming requires a release intent,
and a committed start moves the attempt out of Reserved into worker maintenance.
Worktree-only ambiguity stays in termination hints, and missing-server recovery
retains the typed `dedicated_server_lost` proof. No schema or dependency changes.

The permitted unreal-t4c ledger records W4-06a5 reserved around 22:2xZ, ambiguity
within roughly five minutes, and server-loss recovery at 22:35:59Z. Its healthy
replacement W4-06a6 reached Running in about 40 seconds. W6-03fixf expired at
16:30:12Z during launch_creation. These are ledger observations, not measured
phase durations; the live ticker/telemetry logs are outside this checkout and
were unavailable under the owner's data-access restrictions. E2E coverage holds
native creation for two seconds with a one-second configured lease and checks
one confirmed launch, no ambiguous outcome, and Running. The existing lost-reply
workflow stops gracefully and restarts while Claimed. A separate killed-launcher
workflow expires through the public store API and checks one creation, no prompt,
one typed failure notice, supervised cleanup, active control and a new attempt
reaching Running for the same task without an identity block.
A socket-free signed-reservation workflow also exercises renewal through the
public store API: the old lease token is fenced, the original expiry does not
expire the renewed generation, and neither elapsed nor ambiguous claims can be
revived or reclaimed.

Repository capture budget exhaustion after exact native termination now records
`capture_incomplete` in the retained worktree references and one coordinator
notice, releasing capacity while preserving the original worktree. Explicit
capture still refuses oversized untracked source. Tracked byte omissions carry
raw Git identities in worktree manifest v5; see [worktree capture budget
recovery](canonical-worktrees.md). This does not establish a verified result or
permit repository deletion.


The shared root queue prioritizes recovery (`canonical-worker:recover:` and
`recover-`), termination, brief preparation and brief delivery before launch
advancement in any project. Pending safety work blocks new launch advancement
even during its retry delay. Failed recovery requests remain executable between
inventory scans. Three failed turns or a wait exceeding the recovery observation
interval promotes the oldest waiting job within its priority class; refreshed
inventory offers do not reset that history. Admission starts a new wait and
spends a promoted job's contention credit; successful completion clears its
failure count. A repeatedly failing safety job cannot monopolize other safety work.
Root acquisition holds `.execution-turn.lock` while draining shared holders,
so new status/start probes cannot overtake an already waiting exclusive effect.
The existing `.execution.lock` remains the inherited execution barrier.

Native creation has a 15-second external request budget inside the unscaled
45-second phase; the lease covers the phase plus a five-second commit reserve.
A creation timeout first observes whether an exact supervisor exists, without
publishing a launch target or acknowledging the lost creation reply. An observed
supervisor retains uncertainty and capacity for ordinary recovery. Otherwise it cancels only the automatically
managed server recorded for that project, task and socket, using a pidfd after
checking the live command, environment and socket incarnation. The existing
server-disappearance proof releases the reservation, with typed
`launch_creation_timed_out` for a call cancelled before lease expiry. The server record stays
available to the ordinary retirement sweep. Operator-managed servers are never
signalled; their uncertain resources retain capacity. Expired no-target launches
whose dedicated server remains blocked are cancelled by the same path after a
bounded workspace observation. Existing tables and retained events carry the
typed failure; there is no schema change or new dependency.

E2E workflows in `tests/canonical_worker.rs` exercise a shared root with a
continuously offered launch in one project and empty-workspace recovery in
another, bounded to 30 completed passes. Projects share the lab owner's key and
configuration, while repositories, executables and execution homes remain
separate. A separate hung native creation exceeds
the configured one-second lease floor, is cancelled within the phase lease,
and finishes with a typed delivery, no target, no ambiguity and released capacity.

Pending recovery E2E first refuses workspace observation for ten passes, checks
that the other project's native creation is still held, then permits observation
and verifies terminal recovery within 30 passes and a new launch reaching Running.
Termination E2E refuses the termination journal commit, verifies bounded retries
and held launch advancement across projects, then permits the commit and verifies
capacity release, one persisted termination and the other launch reaching Running.
The injected SQLite journal refusal surfaces as the store's `Conflict`, so the
workflow matches the queue diagnostic for the exact attempt and checks that the
stop hint carries its post-cancellation revision. The same ticker must finish
termination after the refusal is removed, excluding a permanent stale-fence loop.
These replace the obsolete queue unit policies that allowed launches during
recovery/termination cooldown; coverage remains through public CLI workflows.

Passive liveness polls use `canonical-worker:monitor:terminate-` and keep their
existing observation cadence. They do not hold new launches throughout cooldown.
The controller distinguishes requested cancellation/completion, failed attempts
and expired unreleased launches using existing state and events, promotes those
stops ahead of project-local launch hints, and emits ordinary priority termination
jobs. The worker still validates the current attempt before any signal. This
allows additional projects to launch beside healthy workers without weakening
pending-stop priority.

The pending-stop workflow also starts a second healthy project while the first
worker is still Running, then holds a third project's launch behind the requested
stop. It verifies that passive monitoring permits concurrent projects and that
requested termination blocks advancement until its persisted outcome commits.

A restarted ticker also observes a still-live original creation claim. Its native
observation deadline leaves five seconds for bridge draining and five for server
cancellation/receipt; an unresponsive creation is cancelled before the original
lease expires, without renewal or replay. Expired Claimed rows have the same
terminal handling as Ambiguous rows, so a deferred expiry service cannot leave
an empty launch or gated target parked indefinitely. The hung-creation E2E runs
both with the original ticker and with a killed/restarted ticker, checking that
the typed failure commits before the original lease deadline in both scenarios.

Native short socket namespaces (`r*/s`) also hide their containing private Herdr
directory; shared temporary and owner/project directories remain protected by
the existing shared-directory checks. Socket incarnation checks precede lease
budget decisions, and a short remaining lease alone never proves a hung server.
The pending-stop workflow keeps its ticker alive through persisted termination
and the next launch after removing the journal failure fixture. Completed passes
can precede background commits; stopping then can interrupt the newly admitted launch.
## Exit watch and execution-lock contention

Passive worker observations are named
`canonical-worker:worker-exit-watch:terminate-<attempt>`. Requested stops retain
their existing safety-job identity and priority.
A read-only retained selection and pidfd exit poll precede root ownership: a live
worker without failure, cancellation, completion, wall expiry or launch-lease recovery
returns without `.execution.lock`. Live staged gates also poll without ownership
while their original launch authority remains usable.
An exited worker awaiting its own verification verdict also avoids that lock.
Observations, including contended retries, cool down for 15 seconds (scaled in
isolated labs). The stored `runtime.worker_termination` hint remains compatible.
In labs, the 3.6-second queue retention prunes retained requests between advisory
windows; the lock-free hint re-offers them each pass, so exit observations may
follow the 15.5-second advisory cadence.

An actual exit recording or requested stop waits at most 30 seconds for the
exclusive root, never past the original 45-second job deadline or cancellation.
The existing OS-backed `.execution-turn.lock` writer turn drains current
holders and prevents new phases from overtaking the recorder. Existing
ROOT-LOCK-FAIRNESS safety admission, requested-stop priority and maintenance
batch scheduling remain in place for effects. Retained exit watches may be
admitted while the maintenance observation batch is pending, so a blocked batch
cannot suppress the lock-free exit poll or contention escalation. They retain
the single queue ticket and take the exclusive root fence before recording.
Failed attempts bypass the live-worker early return and reach supervised stop,
including workers whose brief approval expired after launch.
Exit selection is read again after acquisition;
all existing identity, revision, receipt and lock-order fences remain. A conflicted
selection or commit refreshes the attempt revision and retries up to three times
within the job deadline with backoff. The queue retains failed termination
observations, including unchanged-revision SQLite refusals, for later retries
without relying on a fresh maintenance-blocked inventory. Repeated recording
conflicts use the same advisory escalation; its counter clears only after
termination is recorded.

The old card's process-local priority reservation, copy-phase yielding and
service-priority changes were dropped: current main already provides fairness
across processes and schedules safety work before launch advancement. No
additional live-copy hold reduction was needed or measured by this change.

After three exhausted acquisition windows or two minutes since the first
deferral (whichever comes first), the worker writes one stable advisory inbox
notice `telemetry-health-worker-exit-<attempt>`. It names the attempt, count and
holder kind from best-effort Linux lock evidence: same-process or external-process,
shared project effect/observation or exclusive root maintenance/external operation.
Unavailable evidence is explicitly unknown; no PID or unrelated lock data is
persisted. The notice also records the last acquisition wait in milliseconds. Counts and elapsed time are ticker memory and
reset on restart; the notice survives restart and is deduplicated even after
acknowledgement. Advisory intake uses the existing active-store publication
checks and atomic inbox transaction independently of the effect barrier, so a
permanently held execution lock cannot suppress its own warning. Execution state
is untouched. A failure to persist the notice is reported and retried.

Successful exit-watch log lines include total `elapsed_ms` (read, wait and recording).

`telemetry PROJECT health --json` exposes `worker_exit_contention`; health text
prints the notice summary. Each `telemetry PROJECT attempts` row exposes its
matching notice in `worker_exit_contention`. These retained diagnostics do not
by themselves mean recording is still blocked: inspect the current attempt.
No canonical or sidecar schema changed.

`inbox done` uses a monotonic exact-ID acknowledgement: acquire ownership with
the existing bounded foreground wait (30 seconds), then read the current event
head and retry up to eight unrelated head conflicts. It only handles the chosen
IDs; `--all` selects the unfinished IDs shown before the wait. The strict
`runtime::update_inbox`/store API still checks the supplied head. Task edits and
cancel-attempt already use the foreground wait and retain their explicit head
and entity revision fences; a changed task or attempt must still be reviewed.

### Prompt growth and launch size preflight

`launch PROJECT run` checks the prompt's UTF-8 byte envelope before signing
preflight and profile readiness/interaction probes. Previously snapshot creation
ran after profile preparation, so a readiness refusal could hide an oversized
prompt. `prompt_too_large` now identifies the growing part and reports bytes and
limit. The final retained rendering also enforces the byte limit; the legacy
`prompt_chars` and `budget_chars` names remain for compatibility.

`memory PROJECT attempt-brief --attempt ID` includes `prompt_bytes`: framing,
memory, guidance, brief, fixed_overhead, total, and limit. These parts partition
the complete outbound prompt. Framing includes product instructions, identity
and section delimiters; guidance includes protocol, submission instructions,
outputs, worktree guidance and the command card. Brief includes task title and
explicit task/review instructions. `fixed_overhead = total - brief`. The launch
report's draft step contains the same breakdown. These report fields are derived
from retained text and do not enter approval or configuration fingerprints.
Retry preflight measures an existing attempt from sealed bytes without requiring
its old owner configuration to remain current; launch authority still checks
current configuration and fresh worker identity evidence.

Worker memory defaults to an 8192-byte share. Set `[launch]
memory_budget_bytes = 8192` in the pinned owner configuration to change the launch
allowance (1–32000 bytes). Hard rules and pinned requirements are selected first;
optional facts follow applicability/relevance ranking, then promotion recency.
Oversized optional facts use deterministic one-line excerpts and record IDs.
`memory PROJECT attempt-fact --attempt ID --record RECORD` fetches the full
retained revision through the same attempt checks as attempt-input, including
worker spool isolation. An excerpt never rewrites the canonical record.
The launch knowledge step lists `summarized_for_budget` keys separately from
`omitted_for_budget` keys whose excerpts cannot fit.
Hard memory that exceeds its allowance produces a typed `prompt_too_large`
refusal naming the records, rather than silently dropping rules.

Keep owner memory facts short: they are delivered to every worker. No canonical
or telemetry schema migration is required; summary selection is retained in
existing snapshot entries, and full objects keep their existing retention rules.
