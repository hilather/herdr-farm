# Operator runbook: canonical workers on a real project

For a project already migrated to the canonical store with control active, this
runs a task on a Codex or a Claude Code worker from the CLI alone: no SQL, no
hand-edited agent configuration. Herdr Farm signs launches automatically within
the owner policy through gated
`ssh-keygen -Y sign`. The application never opens private key files.

## 0. Once per machine

Run `herdr-farm new PROJECT` at the final project location. The default build
includes SQLite support. First creation generates the owner approval key and
appends authority settings and available starter profiles automatically; existing
tables are preserved. The profile examples below are optional customization.

* The owner is logged in with the agent CLIs (`codex login`). A Codex worker reuses
  that login (see [Shared login](profiles.md#shared-login)); nothing is copied. A
  Claude worker uses a long-lived setup token instead: run `claude setup-token`, save
  the token in a 0600 file outside the project and `~/.claude`, and add
  `[worker_isolation.login] claude_token_file = "/abs/path"` to the owner
  configuration **before** preparing and verifying the Claude profile. Without it
  `verify-interaction` and `launch run` refuse the Claude profile.
* A pinned owner configuration (`~/.config/herdr-farm/config.toml`) with the
  `[authority]` key and one profile per worker setup, for example:

```toml
[profiles.codex-sol]
kind = "codex"
permission_policy = "interactive"
model = "gpt-6.1-sol"
reasoning_effort = "low"
[profiles.codex-sol.budget]
max_wall_seconds = 3600
unknown_usage = "allow_with_warning"

[profiles.claude-sonnet]
kind = "claude"
permission_policy = "interactive"
model = "claude-sonnet-5-5"
reasoning_effort = "low"
[profiles.claude-sonnet.budget]
max_wall_seconds = 3600
unknown_usage = "allow_with_warning"
```

  Changing this file changes its digest. `launch run` refreshes profile evidence
  and re-acknowledges owner edits automatically; coordinators never edit it.
* An existing branch of the repository, not checked out, for results to integrate
  into (for example `git branch integration`).

## 1. Profile evidence is automatic

`launch run` revalidates retained evidence and refreshes it when missing or stale
(config digest, agent or Herdr binary, or model pins changed). It uses the same
probe as `profile verify-interaction --retain` and reports `profile_evidence:
refreshed` with the reason. Herdr is selected from `HERDR_BIN_PATH`, then the
latest profile evidence, then an absolute real file resolved from PATH. The
agent uses the recorded executable if present, otherwise its kind from PATH
following symlinks; script shims require `mise which <kind>` or explicit
`profile verify-interaction`. The execution home uses the recorded home or
`~/.herdr-farm-homes/<profile>` created with mode 0700. Explicit verification
remains available for unusual installations.

Run launch in the background or with a timeout of at least five minutes: profile
refresh alone can take 120 seconds. Progress appears on stderr as each step
starts. Rerun the same command after interruption; finished steps are skipped.

## 2. Run a planning task per worker

New tasks can be added while other workers are reserved or running. Parallel
workers need disjoint write scopes (each planning task writes only its own
`docs/...md`) and available capacity. An explicit owner pause must be resumed
after reconciliation before launching. Write the task text (`prompt.md`), then:

```sh
common="--repository /path/to/repo --prompt-file prompt.md --integration-ref refs/heads/integration \
  --max-active-workers 2"
# Reserve each task; --prepare-only optionally stops before drafting and reservation.
herdr-farm launch PROJECT run --task plan-codex  --profile codex-sol      --plan-output docs/plan-codex.md  $common
herdr-farm launch PROJECT run --task plan-claude --profile claude-sonnet  --plan-output docs/plan-claude.md $common
```

Per run it: revalidates and, when needed, refreshes the profile evidence; makes
the project
active (an owner configuration acknowledged by an active project is what lets a signed
contract be installed); adds the task; builds and signs (namespace
`contract@herdr-projects`) the planning contract (the single deliverable `docs/...md`,
write scope exactly that file, acceptance: the file exists and has content, route
`verify_then_integrate` with `--integration-ref`); installs it; queues the task; sets
scheduler capacity; configures the integration target and turns verify+integrate
automation on; starts a dedicated Herdr server for the task under
`PROJECTS_ROOT/.herdr-run/PROJECT-TASK/` (or uses `--herdr-socket` for a server you run);
binds the task to it, reconciles and activates. Without `--prepare-only` it then
retains the knowledge snapshot (PROJECT.md followed by your prompt and the
deliverable instruction), drafts the launch, signs the approval
(`approval@herdr-projects`), imports it and reserves the attempt. The JSON report lists
every step as `done`, `already_done`, or `refreshed`, the attempt id and its worktree.

It is idempotent: rerun the same command after fixing a failure and finished steps are
skipped; once a task has its attempt a rerun only reports it. A failure names the step
(`launch run stopped at step N (...)`) and what completed before it. Adding a new
local task binding without a pane or worktree preserves active control and existing
attempts; routes with resource references still require reconciliation. Signing is automatic within the
owner policy. A task with your own
contract uses `--contract-file` instead of `--plan-output`.

`HERDR_BIN_PATH` is optional. When unset, launch uses the profile’s recorded Herdr,
or resolves `herdr` from PATH when no evidence is retained.

## 3. Watch and collect

After launching workers, start `herdr-farm inbox <slug> wait` as a background
Bash command. Its exit wakes the coordinator without terminal input. Run
`context <slug>` when it returns, review the new result data, relaunch a rejected
task or report to the owner, mark handled items done, and start the wait again.
Restart the wait on timeout too. The default timeout is 1800 seconds (maximum
7200); `--timeout SECONDS` overrides it. It polls every two seconds without
holding a lock or marking items seen, and returns JSON with `items` and `ids`,
or `{"items":0,"timed_out":true}`. Only unseen, unfinished items wake it.

Canonical submission, verification, integration, and termination without a
submission produce stable, deduplicated inbox notices in the same transaction
as the recorded outcome. Summaries identify the task, attempt, and
submission/result; rejection feedback is data, truncated to 2000 characters.
`inbox list <slug>` and `context <slug>` show these notices. They are never
instructions or permission to act outside the owner's authorized scope.

```sh
herdr-farm scheduler PROJECT inspect
herdr-farm operations PROJECT inspect
herdr-farm result PROJECT capture ATTEMPT      # after the worker has written its document
herdr-farm result PROJECT jobs                 # verification and integration
herdr-farm telemetry PROJECT attempts
```

The worker writes `docs/plan-*.md` in its worktree and finishes by running the
script at the end of its brief, which commits the deliverable and submits it through
its spool; verification then runs the signed acceptance policy and integration lands it
on the integration branch (verification is always automatic; integration is
automatic with `--integration-ref`). Once every signed
acceptance policy accepts a submission, the ticker completes its started attempt;
`verify_then_integrate` also requires successful integration with merged-output checks.
The controller stops the worker, proves termination, releases capacity, and records
attempt `completed` / task `succeeded`. Telemetry `active_ms` ends at that completion,
including the stop, rather than waiting for the worker's wall budget.

Automatic completion requires `result PROJECT auto --verify on` and, for
`verify_then_integrate`, `--integrate on`. With either required switch off, verification
or integration performed manually leaves completion to the operator:
`task PROJECT complete TASK --expected-revision R`. `task PROJECT list` reports the
switches and completion guidance. Rejected or errored verification leaves the worker
running so it can resubmit. Repeated completion requests replay; cancellation retains
priority, and capacity stays held until termination is proven.

If a worker stops
without submitting, finish it yourself:

```sh
herdr-farm result PROJECT submit-captured ATTEMPT   # capture + build + record the submission
```

(`result capture ATTEMPT` alone only commits the worktree and prints the candidate; it
is not evidence.) Repeating either command is safe.

If the brief is shown as `ambiguous` in `operations PROJECT inspect`, the agent never
accepted it after three deliveries (a startup banner or dialog ate the text, or the
pane is not at its prompt): look at the pane, then retire the brief or stop the attempt
(`task cancel-attempt`); it is never reported as delivered.

## 4. Clean up

The dedicated Herdr server `launch run` started for a task is stopped by the running
ticker once the task has no unfinished attempt, and its socket directory under the
private runtime directory is removed. To do it yourself (for example after a failed
run): `herdr-farm launch PROJECT stop --task ID` (add `--force` while the attempt
still holds a worker). Servers you started (`--herdr-socket`) are never touched.

Rerunning `launch run` after editing the owner configuration re-acknowledges it
(`project_control` is reported as `done`); profiles must be prepared and verified
against the edited bytes first, as their evidence is pinned to the configuration digest.

### Submitting work against a large base

`result PROJECT submit-captured ATTEMPT` and the canonical brief's POSIX sh
script submit only objects new since the frozen base, plus both commit anchors.
A base with hundreds of unchanged files is supported. Keep the signed contract's
owner repository available with that exact base commit: verification imports its
base into a fresh private repository, checks retained object hashes and complete
candidate connectivity, and checks out the exact candidate before acceptance.
It never imports a candidate from a worker worktree or quarantine.

Submission bounds are 1024 delta/anchor objects, 16 MiB per compressed object,
64 MiB aggregate retained bytes and 256 KiB of JSON. An object-count refusal
prints the count and limit; byte-limit refusals identify their bound. Split a
change exceeding these derived limits into smaller tasks. The unchanged trusted
base does not count against them. Retry unchanged submissions under the same key
for idempotent replay. A missing base reports `owner repository base is
unavailable or invalid`; a corrupt or incomplete candidate reports `candidate
object hash mismatch or missing referenced object`. Restore the owner base or
correct the submitted closure; never supply an alternate pointing at a worker.

Validation: `cargo test --locked --offline -j 3 --test
canonical_worker --test factory_harness` covers large-base automatic integration,
replay, wrong object bytes, and an unstaged candidate blob. The canonical worker
suite needs Unix socket permissions; compile it with `--no-run` in a restricted
sandbox and run it on the steward's host.

Compatibility: legacy environment variables and existing data/config locations remain supported; see [renaming](renaming.md).


## Canonical coordinator after migration (W-COORD-2)

Coordinator `open` retains operator intent and project ownership across Herdr
calls and priming waits. Root exclusivity covers only conflict validation and
binding publication, with no Herdr I/O. Replacement relinquishes the old claim
under the existing project guard rather than reacquiring a shared root lock.
An absent or uncertain pane returns to the owner for `open --reprime`; the
canonical ticker does not replay coordinator startup or priming each pass.
The E2E replacement workflow requires foreground reprime to finish within five
seconds while the accelerated ticker is running.


After migration, open the coordinator with `herdr-farm open PROJECT` in the
owner's Herdr session (or pass `--socket /absolute/session.sock`). It uses
`PROJECT.md`'s `coordinator_agent` and the project's effective
`coordinator_agent_args`. It creates or reuses the coordinator pane, binds it as
canonical `coordinator`, and records live observation and ownership. It never
starts a replacement server or imports old thread records as live state.

After observation and adoption, `open` resumes automatic reconciliation pauses
through the same activation path as `launch run`. A retained owner control event
setting paused/archived prevents automatic resume even after later invalidation.
Admission blockers print individually and leave control paused; open succeeds.
Focus-only opens perform the same check. Owner config edits are re-adopted using
the current digest. `context` identifies automatic pauses with `open PROJECT`
re-activation guidance and lists profiles with retained launchable evidence.

Claude Code starts receive `--settings <project>/.state/coordinator/claude-settings.json`
in addition to owner arguments. The generated directory/file modes are 0700/0600;
owner `--settings` arguments are refused. Allow rules cover coordinator verbs,
ask rules cover owner decisions, and deny rules protect configuration, SSH,
project `.claude` files and the generated file. Owner settings are never edited.
A running agent picks up permissions only when open starts it; focus does not
reload them. `context` records startup provenance and `doctor` explains that
non-Claude kinds have no generated permissions. Retain the generated settings
and journal in project backups; telemetry never prunes them.

Coordinator priming accepts bundled manifests and updated `remote:<path>` manifests
whose resolved file is inside the owner's Herdr state directory
(`$XDG_STATE_HOME/herdr`, or `~/.local/state/herdr`). Symlinks escaping that
directory and relative paths are refused. Herdr must report the configured agent,
an idle matched rule and visible idle prompt, no working/blocker signals, no
warning or fallback, and a non-empty manifest version of at most 96 bytes.
Dedicated canonical workers continue to require bundled manifests.

Local manifests and local overrides shadowing remote manifests are refused by
default. Inspect the override first; remove it to use Herdr's updated detection,
or explicitly authorize it in the owner `config.toml`:

```toml
[coordinator]
allow_local_manifest_override = true
```

After changing config for a refused priming attempt, inspect the pane and use
`open PROJECT --reprime`. The opt-in does not relax visible readiness checks.
The coordinator journal `.state/canonical-coordinator.json` retains the manifest
source, version and override flag used before submitting each priming prompt.
`doctor` reports this priming evidence and the live remote/override manifest
policy as information, including refusal reasons and recovery actions. A busy
coordinator can have an accepted manifest policy while priming waits for idle.

Priming runs the coordinator skill and canonical context. A prompt acknowledgement
is insufficient: the adapter waits for working/blocked status or visible working
evidence, using the worker brief confirmation window. Startup screens that
swallow a prompt cause up to three deliveries after fresh verified visible idle
checks. Repeated `open` focuses the accepted coordinator without launching or
priming again. Replacement requires fresh Herdr evidence that both the recorded
pane and agent are absent on the same socket incarnation. Open audits withdrawal
of the current ownership revision before creating or binding the replacement;
unavailable or ambiguous observations retain ownership and refuse replacement.
If its pane was closed, run `herdr-farm open PROJECT --reprime` to
recreate and prime it. `--rebind` permits moving sessions only after the previous
socket is gone. An interrupted start/prompt remains a durable pending effect;
inspect the pane before explicitly requesting `--reprime`. An interrupted
workspace creation is never automatically repeated: inspect Herdr, record the
observed route with `runtime PROJECT rebind coordinator` (see `--help` for revision
and head arguments), then remove the inspected coordinator journal before opening
again. Do not remove an unresolved journal to blindly retry creation.

`herdr-farm context PROJECT` prints canonical state without requiring a planner
profile: tasks by state, attempts, inbox, recent submissions and accepted
verification/integration receipts, safety settings and concrete CLI templates.
A configured `profiles.planner` keeps automatic checkpoint context; `--profile NAME` also selects checkpoint/session behavior. Dispatch through
`task PROJECT add ID --title TITLE --expected-head HEAD` and `launch PROJECT run`;
review through `result PROJECT show`, `result PROJECT jobs` and the verification
and integration commands printed by context. See `skill/COORDINATOR.md` for the
complete command forms and approval semantics.

Signer resolution is `--sign-with`, then `[coordinator] signing_key`, then a
matching owner key in the directory of the configuration pinned at migration
and `ctx.config_dir` if different. No configuration edit or key handling is
needed. Discovery reads only public `.pub` siblings: the private path must be a
regular non-symlink file owned by the current user with no group or other mode
bits, and its public half must match `[authority] approval_public_key` by type
and base64. Zero or multiple matches refuse. A fixed probe is signed and verified
before any project state change. Passphrase keys must be loaded into ssh-agent.

Every signer source has the same policy: an execution home for the product
sandbox, a canonical local repository listed in PROJECT.md, a frozen profile
name in the current owner configuration, and unfinished reserved, launching, or
running attempts plus this launch within `[launch] max_workers` (default 4,
range 1..64). `--max-active-workers` defaults to this cap and cannot exceed it.
A refusal solely for capacity or an unlisted repository creates an owner request
for an in-session decision; see “Approve an owner request inside coordinator chat”
below. Capacity approval exempts one exact reservation without changing the cap.
The coordinator never passes `--sign-with`, reads or searches for keys, or edits
config.toml. It reports the exact refused rule to the owner. The coordinator obeys
`start_threads=propose/auto`, `resolve_threads=propose/auto` and
`cleanup_resolved=keep/auto`; signed approvals, verified results, explicit
integration targets and proven termination remain canonical enforcement.

All `thread` commands clearly refuse on a canonical store and name canonical
replacements. These are deliberately not aliases: legacy thread prompts and
resolution cannot preserve sealed contracts, immutable attempts and accepted
result evidence. Old thread state remains migration provenance.

The version 2 `.state/canonical-coordinator.json` journal stores frozen route,
socket incarnation, terminal, settings/config digests and layout/start/prime
phases and the generated permission file supplied at start. Version 1 journals
upgrade through `migrations/canonical-coordinator/0002.json` on open. It is effect intent and a receipt, not a second runtime owner. Retain it
with project backups and reconcile restored pending effects before replay.
No canonical or telemetry SQLite schema changes or new tables are introduced.
The maintenance inventory classifies it as canonical and never prunes it.

### One-command code tasks

```sh
herdr-farm launch PROJECT run --task CODE --title 'Implement the change' --profile codex-sol --repository /absolute/repo --write src/ --write tests/ --output src/lib.rs --prompt-file /absolute/brief.md
```

Repeat `--write` for 1–64 repository-relative files or directory prefixes ending
in `/` (no globs), and `--output` for 1–8 exact files the result must contain.
Each output must fall inside a write scope. An existing file the task changes or
`<dir>/NOTES.md` can be an output. Each gets a file-exists-and-has-content policy.
`--deliverable` overrides the title-based description. `--base` (default `HEAD`)
is resolved once to a commit for generated contracts and their worker worktrees.
The owner checkout stays on its current branch. An installed or advanced
`--contract-file` contract supplies its own `base_oid`; that base must exist
locally before reservation.
`--integration-ref` selects verify-then-integrate rather than verify-only.
Planning (`--plan-output`), code (`--write`/`--output`) and advanced
(`--contract-file`) forms are mutually exclusive.

Advanced contract JSON contains the decisions. The product replaces
`project_store`, `expected_head`, `contract_revision` (latest task revision + 1),
and `authority` before signing; these fields may be omitted. `task_id` must match
`--task` and `profile_kind` must match the retained profile. Other values are
preserved through JSON reserialization. Signing keys and signature namespaces
are unchanged.

Interactive `launch run`/`launch stop`, `context`, `inbox`, `task`, `result`,
and `memory` commands wait up to 30 seconds per acquisition for busy project
locks. They retry only lock contention, release partial ownership before waiting,
and print one waiting notice after about two seconds. Set
`HERDR_FARM_LOCK_WAIT_SECS` to an integer number of seconds to override the bound
(`0` restores immediate failure). Ticker effects and jobs keep their existing
non-blocking acquisition policy. Coordinator shell retry loops for busy locks
are no longer needed. A lock busy beyond the bound stops launch with the usual
resumable “rerun the same command” message. Pure reads need no lock wait.

Launch retries store head conflicts up to eight attempts with fresh heads and a short
50–250 ms backoff (accelerated only in test labs), rebuilding and signing documents
that embed the head. Fences remain enforced. Exhaustion asks you to rerun the same
command. The report includes the contract digest, write paths and outputs.
The brief stages all changes in the write paths, refuses missing outputs and
names changes outside scope before submission. Restore those paths: verification
cancels attempts whose results change anything outside their scope.

## Watching canonical workers

Workers appear as `worker: <task>` tabs beside the coordinator in the owner's
Herdr workspace. `herdr-farm launch PROJECT view --task T` reopens a viewer, or
focuses its existing tab. The dedicated worker server keeps its own shell and
bundled manifests; the viewer uses a private product config allowing nesting,
without changing the owner's Herdr config. If the coordinator session is
unavailable, launch still succeeds and reports the viewer as unavailable.

The profile's `max_wall_seconds` ends a worker that runs out of time. After an
`attempt.ended_without_submission` inbox notice, run
`herdr-farm result PROJECT submit-captured ATTEMPT` to submit what it produced,
then review it. Tell the owner before launching a task that looks longer than
the budget.

Stopping a dedicated server with `launch PROJECT stop --task T`, or the ticker
sweep after worker termination, closes only its recorded viewer tab if its ID
and label still match. A closed viewer or unreachable owner session is harmless.

The `state-store` feature is enabled by default; build recipes above use that default.

New projects use the canonical SQLite store by default: `herdr-farm new demo`
creates the project at its final path, paused, even while the ticker runs.
`open demo` or `launch demo run` activates it. Use `new --legacy demo` for
legacy Markdown behavior. Migration is only needed for existing legacy projects.
Canonical projects pin their absolute path; choose the final name and location
before creation rather than moving the directory afterward.

On the first canonical `new`, absent authority settings are appended to the owner
config without rewriting existing content. An Ed25519 approval key is generated
at `owner-approval` (0600), with `owner-approval.pub` (0644), beside config.toml.
If no profiles are configured, executable `codex` and `claude` commands on PATH
receive interactive starter profiles with a one-hour budget and no model pin.
Claude workers need `claude setup-token` once; setup never runs that command.
Existing authority and profile tables are preserved.

An interrupted creation retains a `.creating` marker. `list` reports `creating`,
and ticker passes, doctor scans and open leave it inert. A second `new` explains
that, after confirming no creation command is running, you can remove that
project directory and run `new` again. Do not move a partially created store.


## Approve an owner request inside coordinator chat

When launch policy refuses for capacity or an unlisted repository, the
coordinator receives a canonical owner-request and runs the exact printed
`owner <slug> approve REQUEST --summary '…'` command. Read the summary in Claude
Code's confirmation dialog and press the approval key if you agree. You do not
need a terminal or configuration edit. Decline if you disagree; the coordinator
can mark the request rejected with the same exact summary. Rejection grants
nothing. A wrong summary, expired request or already decided request refuses.

A capacity approval allows one reservation of the named task and contract
within 24 hours, leaving the configured cap unchanged for other tasks. A
repository approval adds that exact repository to PROJECT.md's repos. The
coordinator then repeats the original launch. Separate requests require separate
decisions when both policies refuse. Signer/profile failures and an explicit
owner pause require their ordinary repair; owner requests cannot waive them.

Legacy PERM-1 requests can use the same in-session owner command and the exact
`Owner permission requested: …` inbox summary. Terminal `safety approve` remains
available. Existing coordinators pick up regenerated settings when `open` next starts
the coordinator agent. Ask rules cover normal command forms, not hostile owner processes;
see operations.md for wrapper rules and limits. Retain owner-request history in
whole-project canonical backups; do not replay approved authority from telemetry.

For real project tests, declare `[verification.toolchains.NAME]` once in the
owner's external `config.toml`, with absolute `paths`, validated `env`, optional
`network` (default false) and `timeout_seconds` (default 600, maximum 3600).
A code launch can then add repeatable
`--accept 'NAME:./tools/run-tests.sh --headless'` alongside `--write` and `--output`.
The generated signed policy pins the toolchain and its dependencies. Tests run
in the private checkout with read-only tools, private `/tmp` and loopback-only networking by
default. Set `env = ["GODOT=/absolute/path/to/engine"]` and include the engine
and required tools in `paths` to give the worker the same test environment.
The worker receives those validated entries when `--accept` selects the
toolchain; a launch without it receives none. Tool paths must be readable in
the worker sandbox, outside hidden owner secrets and the projects root.
Codex commands and acceptance checks can listen on their own private loopback;
this grants no outbound access or access to host loopback services. Ignored caches are allowed; tracked or unignored changes fail acceptance.
After an intended toolchain update, sign a new contract revision. See
[acceptance toolchains](factory/verified-results.md#owner-declared-acceptance-toolchains)
for manual policy preparation, evidence and timeout details.


Reviews launched by the coordinator should use
`launch PROJECT run --task REVIEW --profile PROFILE --repository REPO --review-of TASK --output docs/reviews/R.md --prompt-file BRIEF`.
Skeptic challenges add `--review-kind skeptical`; use `--review-scope tree`
for a whole-gate challenge. The launch records the opportunity, operator
assignment and session automatically and retains the project context and
review instructions. The worker submits a review receipt (including
`findings: []` for an empty review) through the spool in addition to its normal
report result. Receipts remain proposals: triage with
`telemetry PROJECT review findings validate|reject|duplicate` at the owner CLI.
`telemetry PROJECT review report` shows M20 completion and M28 skeptical yield;
pending pass claims appear as `pending_triage` until decided. A
`attempt.review_receipt_without_result` notice means the receipt is recorded
but the report still needs submission. Fix rounds use
`launch PROJECT run --task FIX --profile PROFILE --repository REPO --fixes-review REVIEW --write ... --output ... --prompt-file FIX_BRIEF`,
or repeatable `--fixes finding:<token>` for a subset. Selected pending claims
become canonical findings and assigned repairs; submissions record proposals,
accepted exact-candidate verification closes them `fixed`, and integration links
them. Re-review with `--review-of FIX` retains the repair findings as priors.
The owner's shepherd delegated validation on fix launch, repair decisions after
verification, rejection with reasons and duplicate triage to the coordinator on
2026-10-04 under its overnight authority. That is the shepherd's decision, not
the owner's own words; the owner's session records both as `operator:cli` with
`operator_owner.v1`, without distinguishing who acted. Review M22 and M25 as
well as M28 after triage; merge/reset and broader review decisions stay the owner's. See
[review launch contracts](telemetry/contracts-review.md) §13.

Launch lineage: reuse one `--work-item WORK` for every build, review, fix and
recheck of the same piece of work. Reviews and fixes inherit the target's work
item unless explicitly overridden. Use `--role recheck` for a recheck; normal
launches derive `build`, `plan`, `review`, `skeptic` or `fix` from their flags.
When recreating a task under a new id, pass `--work-item WORK --supersedes OLD_TASK`;
the old task must have no active attempt. Lineage is frozen before the first
attempt and retained outside the signed execution contract. Inspect it with
`telemetry PROJECT attempts --json` and
`telemetry PROJECT accounting work-items --json`.
