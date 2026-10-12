# Canonical worktree preparation

Linux library service `worktree_preparation::prepare` provisions repository
resources for a reserved, owner-approved launch. The native launch adapter now
continues that original claim into resource creation, gate release and start.
It maps the selected source working directory into its approved checkout and
retains the primary worktree incarnation in runtime ownership. `launch draft` and
`launch reserve` expose the reviewed preparation/admission path; enabled canonical
dispatch invokes worktree preparation before native resource creation. See the
[dispatch audit](dispatch-enablement.md) for the validated scope.

The complete budgeted worker brief includes all checkout paths and branches.
Creation and release verify pristine approved bytes; start pins directory and
Git associations while allowing legitimate worker changes after release.
The linked checkout's HEAD must still name its approved branch; switching to
another branch or detaching HEAD invalidates that proof. New commits on the
approved branch remain valid after release.
Neither continuation nor lost-reply recovery consumes a second approval.
Metadata and checkout files are opened without following symlinks and with
nonblocking semantics, then checked as single-link regular files by descriptor.
A named-pipe replacement cannot stall the reader while it holds root ownership.
Working directories absent from the approved tree are refused before claiming.
Termination preserves tracked and untracked non-ignored checkout files and
retained references; comprehensive artifact acceptance and safe cleanup remain separate unfinished work.

The service derives one path and branch per approved repository from the actual
attempt ID. Callers cannot supply a branch or output directory. Plans appear in
the launch draft for review. For a contracted task, draft and reservation resolve
its signed `base_oid` as a commit, check the repository object format, and retain
that commit and its tree. A missing base refuses launch before reservation.
The owner's checked-out branch is never changed. Generated `launch run` contracts
resolve `--base` (default `HEAD`) once; advanced contracts use their own base.
Capture and submission compare against the attempt's frozen contract base, and
capture refuses a worktree whose history does not contain it without rewriting
history. Quarantine import, preservation and verification retain these same
commit identities. Source HEAD movement and dirty source files do not
replace the approved commit/tree; the worker receives a new linked checkout.

Before consuming approval, preparation validates the full retained brief, profile
configuration, executable bytes, execution home, repository objects, empty output
paths and absent branch names. It refuses partial clones, configured filters,
submodules, duplicate common repositories and noncanonical paths. Object reads,
tree/file inventories, subprocess output and deadlines are bounded. Tree paths
must be UTF-8; file bodies can be binary. Checkout transformations that do not
match the approved blob bytes are not accepted as ready worktrees.

Approval consumption, the one-use launch claim and `runtime.worktrees_creation`
commit together before creating directories or running Git. Git hooks, filesystem
monitors, automatic maintenance, replacement refs and lazy fetching are disabled;
no submodule update or network fetch runs. A random creation token is retained
in the intent, local marker and Git worktree lock reason. Git fsync is requested.
The inherited root barrier remains held while supervised Git processes clean up.

Observation checks both directions of the linked-worktree association, common
repository, lock token, branch and approved base. It compares actual regular-file
and symlink bytes against bounded `ls-tree`/`cat-file` output, checks executable
bits, and rejects unexpected files. Git's stat cache and assume-unchanged flags
cannot substitute for those byte checks. Directory descriptors remain pinned
through receipt insertion; filesystem incarnation identities are retained with
`runtime.worktrees_ready`.

A repeat call performs observation only. A lost receipt commit can be recovered
after approval revocation without another `git worktree add`. Missing, partial,
modified or replaced resources retain uncertainty and worker capacity. This
service never removes worktrees, branches, dirty source files or partial results.
The receipts do not certify task completion or artifact preservation.

The bounded root ownership inventory includes every creation intent, even before
Git returns a receipt. It validates content-addressed launch inputs, operation and
claim provenance, consumed approval, deterministic plans and any ready receipts.
Corrupt, orphaned, duplicate or over-budget records refuse inventory collection.
Cancellation, termination or missing filesystem paths do not implicitly release
these references. Adoption includes the staged references in its conflict checks;
new worktree provisioning checks canonical and legacy references before consuming
approval. Equal, parent and child paths conflict, including across projects under
the same root. This inventory is conflict evidence, not permission to remove data.

## Preparation cancellation and expiry

The termination adapter can retire a worktree-only launch after cancellation or
claim expiry. It first reacquires the exclusive root barrier inherited by Git
supervisors, establishing that preparation has stopped. It requires the original
worktree intent, retained approval use and reservation, and refuses this path if
any native launch intent or resource event exists. The controller offers these
attempts as bounded termination jobs without enabling new-launch dispatch.

After those checks, recoverable checkouts are captured with their working files,
approved/current Git history and index state. Capture can recover a partial or
modified checkout without declaring it ready to launch: the creation token,
branch and both Git associations must still match. Existing ready receipts also
require the original directory incarnations. A missing ready checkout, or a
missing path with retained branch/registration, blocks retirement. Plans without
ready receipts can record absence only after checking directory, branch and Git
registration twice. Attempt outputs are captured or recorded absent as well.

Delivery retirement, attempt/task state changes and version-2
`runtime.worktrees_stopped` commit together with ordered snapshot/absence
references and output evidence. Capture or database failure retains capacity and
allows verified retry. Late continuation is fenced; approval is never refunded.
All ownership references remain retained. No checkout or branch is deleted, and
the receipt does not certify artifact acceptance. A live, uncancelled preparation
is left unchanged and produces no snapshots.

Ownership inventory accepts the single epoch advance recorded by lease expiry,
while still requiring the original consumed claim and expiry-event provenance.

## Working-file preservation

`worktree_preservation::capture_stopped_files` explicitly captures checkout files
for an exactly terminated attempt under the exclusive root barrier. It checks
the requested head, retained plans and checkout incarnations before reading.
All repositories share a 50 MiB/10,000-entry source budget, the original deadline,
and cancellation. Reads use pinned directory descriptors and never follow links.
Repository snapshots preserve symlinks as typed entries containing their literal
target bytes, read through pinned link descriptors without following the target.
Manifests containing links use version 3; older regular-file manifests retain
their version. Hard links and special nodes remain refused, leaving the source
in place. Attempt-output snapshots continue to reject symlinks. It records binary bytes, executable bits and empty
directories, including untracked non-ignored files. Git-ignored files and
directories (toolchains, caches and build output) are excluded before traversal.

Two byte-based scans must agree before publication. Files are retained by SHA-256
under `.state/worktree-file-snapshots/<attempt>/<manifest-digest>/`, synced and
read back before `manifest.json` is published as the completion marker. Repeating
capture verifies existing bytes; conflicts are refused rather than overwritten.
Partial directories and interrupted writes remain bounded retained evidence,
never grounds for deleting a source or implicitly reclaiming an older snapshot.

The file-only manifest explicitly declares scope `working_files`. It excludes
the root `.git` association and carries no Git-history/index evidence. Capture
makes no task transition or canonical acceptance event and grants no cleanup
permission. The file-only entry point is separate from the repository-state
capture used by native termination below.

## Repository state preservation

`capture_stopped_repository` adds version-2 `repository_state` manifests in the
same snapshot store. Each includes current HEAD, the approved base, logical index
entries/stages, exact index bytes, any referenced shared-index file, and an object
pack containing history reachable from HEAD/base plus all indexed objects. This
keeps staged-only content distinct from working-file bytes. Git's optional locks,
hooks, automatic maintenance, replacement objects and lazy fetching are disabled;
commands inherit the root execution barrier and original deadline. Index/ref
observations must agree after packing, after the working-file verification pass,
and immediately before manifest publication.

Git data has a separate 50 MiB budget shared across the attempt's repositories;
individual index files and inventories are bounded at 4 MiB and logical entries
at 10,000 per repository. Packs are produced without reuse/deltas and with one
packing thread. Unsupported index objects (including submodules), oversized or
changing sources refuse capture while retaining original resources. Repository
configuration, unrelated refs, reflogs, and unreachable objects outside the
selected roots are not included. This remains preservation evidence, not a
verified task result or authorization to remove the original repository.

Disposable restoration tests import packs into independent repositories after
removing the source object databases. They verify approved/current commits,
staged binary content, split indexes, merge-conflict stages, SHA-256 repositories,
intent-to-add entries and index flags. Restore is tested explicitly; the service
does not automatically replace a user's checkout or Git metadata.

## Preservation before native termination

Staged and started native workers now capture repository state after exact
supervisor/workspace quiescence and before the terminal-state transaction. The
trusted adapter retains the root barrier across capture and commit. Stop receipts
carry the ordered `repository_snapshots` vector; the store requires every approved
plan and either a manifest digest or typed capture-incomplete evidence before
releasing capacity. Missing references, changed sources, unsupported entries,
publication errors, or failed commits retain the
reservation and desired cancellation for retry. Repeated capture verifies existing
bytes, so a lost database commit does not require deleting or replacing a snapshot.

Signalling/exit observation keeps its ten-second cap. The complete termination
job, including capture, uses at most the original 45-second admission budget and
never extends a shorter caller deadline. Current controller jobs use that budget.
The same transaction requires output evidence: a verified snapshot of the attempt
output directory, including partial report/library files, or an explicit absent
source observation. Empty directories inside it are captured distinctly; an
empty output directory itself (gate release pre-creates it) is recorded as
absent. Output capture
failure retains capacity just like unsafe or unpublishable repository evidence.

Worktree-only preparation retirement uses the same repository/output capture
boundary, with explicit absence evidence for plans without created resources.
Unrecoverable partial metadata or missing recorded resources remain blocked.
Report/library finalization can consume verified native-stop snapshots after
output source loss. Repository restoration remains explicitly tested rather than
an automatic mutation path. Cleanup acceptance remains unfinished.

Working-file snapshots exclude Git-ignored files and directories, including
toolchains, caches and build output. Git determines exclusions using the
worktree’s ignore rules; excluded directories are never descended into or read.
Both capture and verification use the same exclusions. The 50 MiB working-file
limit still applies to non-ignored work; tracked byte omissions and identity-only
Git evidence are described below. Version 4 manifests record the ignored exclusion roots returned by
Git in `excluded_ignored_paths`; versions 1–3 remain loadable with an empty list.
This changes no canonical store schema.

Operator launch admission timings and retry budgets are described in
[canonical worker launch](canonical-worker-launch.md). `launch run` no longer
shares draft's preparation deadline with reserve. Worktree creation remains a
later leased operation with its own 45-second cap; copy jobs keep their existing
executor deadlines. A pre-spawn transfer budget refusal never constitutes a
worktree or copy receipt.

Dedicated worker-output capture is fail-soft only for its byte and retained-entry
budgets. Output manifest v2 prioritizes reports and small files, records omitted
paths/sizes/reasons, and leaves source files intact. The termination receipt and
one coordinator notice expose omissions. Untracked working-file capture keeps its fail-hard limits; tracked files can
carry identity-bound byte omissions. Neither kind grants worktree cleanup
authority. See [attempt output preservation](canonical-worker-launch.md#attempt-output-and-preservation-contract).

Worktree capture budget recovery (WORKTREE-CAPTURE-BUDGET-1): the working-file
budget remains 50 MiB. Non-ignored untracked files have first claim on that
budget and exceeding it still refuses explicit capture; there is no silent
untracked-source omission or new owner setting. Tracked regular files are then
selected deterministically across the approved worktrees, source/text before
known binary extensions, smallest first, then path and approved repository order
as tie breakers. Manifest v5 records every omitted
tracked file's path, byte size, raw Git blob identity (`hash-object --no-filters`),
Git mode and `byte_limit` reason. Its `tracked_files` inventory also records
raw Git identities for retained tracked regular files. Hashing streams pinned read-only descriptors through Git in bounded batches; it does
not load omitted binaries into the snapshot. Two independent observations must
agree before publication. Missing tracked files remain recorded by the preserved
Git index versus working-file inventory.

When tracked omissions exist, `git_identity` records HEAD, base, index entries,
object format and retained exact index evidence instead of embedding a complete
history pack. This evidence references objects; it is not an independently
restorable full repository archive. The original worktree and repository must
remain available for recovery. Manifests without omissions keep the existing
portable pack behavior and versions 1–4 retain their meaning.

After native worker/workspace quiescence is proven, a typed capture byte, entry,
depth or metadata-structure limit no longer prevents termination. Each approved worktree reference
carries `capture_incomplete` and an empty digest instead of claiming a completed
snapshot. The termination transaction releases capacity and emits one
`attempt.capture_incomplete` coordinator inbox notice naming retained paths.
It does not pause control or authorize cleanup. Provenance changes, unsafe file
types, publication failures and cancellation/deadline failures still fail closed.
A retry of an already wedged termination uses the same rule without operator
intervention; existing automatic control reactivation applies once its blockers
clear. Owner pauses are preserved.

`result submit-captured` selects only the requested attempt's retained inputs
and worktree provenance, rather than loading the entire project snapshot.
Capture rejects a missing attempt or absent retained launch inputs before reading
that provenance, including historical attempts without a prepared worktree.
Its document references candidate/output Git object identities; worktree binary
size does not enter snapshot read accounting. Submission object-transfer limits
and verification requirements remain unchanged. No canonical schema migration
is needed: manifest files, existing termination event payloads and inbox rows
carry the new evidence.
