# Getting started: open your first project

Install the plugin, create a project, and let a coordinator agent start threads for you.

## 1. Check the prerequisites

- macOS or Linux, and [Herdr](https://herdr.dev) 0.9.1 or newer. Check with `herdr status`: both the client **and the running server** must be 0.9.1. After `herdr update`, a server that was already running stays on the old version until you restart it, and `herdr plugin link` or `install` then fails with `plugin_requires_newer_herdr`.
- Rust/Cargo 1.89 or newer and a C compiler. Herdr builds the executable during installation. On macOS, `xcode-select --install` installs Apple's command-line build tools if missing. Install Rust using [rustup](https://rustup.rs).
- Git and the public repository [`hilather/herdr-farm`](https://github.com/hilather/herdr-farm).
- An agent CLI Herdr can start, on `PATH`. Canonical worker launch supports live-certified Codex and Claude versions; see [canonical worker launch](canonical-worker-launch.md) for login, sandboxing and admission requirements. `PROJECT.md` selects the coordinator kind.
- Optional: `gh`, logged in, for pull request follow-up; `ssh` and `rsync` for threads on other machines.

No hosted service or separate API key is required by the plugin. Your agent CLI has its own prerequisites and account.

## 2. Install the plugin

```bash
herdr plugin install hilather/herdr-farm
```

Review the install preview. Herdr clones the repository, runs its locked Cargo release build, and registers `herdr-farm` with project and fleet actions and popups. Its startup command starts a background ticker only when you have at least one project; until then it creates nothing.

To run the binary from a terminal, create a symlink yourself. `herdr plugin list` prints the plugin's folder:

```bash
ln -s <plugin root>/target/release/herdr-farm ~/.local/bin/herdr-farm
herdr-farm doctor
```

`doctor` prints the binary's absolute path, its version, the projects root and the config directory, so you can see exactly what is running.

## 3. Create and open a project

From Herdr's action menu, run **Farm: new project**. It asks for a name and a goal, creates the project, and opens it. Or from a terminal inside Herdr:

```bash
herdr-farm new "Billing" --goal "Ship the new billing page" --repo ~/dev/app
herdr-farm open billing
```

`new` creates `~/.herdr-farm/billing/`. `open` creates a Herdr workspace in that folder with a `coordinator` tab, starts your agent there, and sends it one priming line that tells it to print and follow the coordinator skill. The first time, your agent asks whether you trust the folder: answer it in the coordinator's pane. The ticker sends the priming line as soon as the agent is ready.

Edit `PROJECT.md` in the project folder to write your standing instructions and to change the agent kind, `max_parallel_threads`, or the listed repos.

## 4. Tell the coordinator what you want

Type in the coordinator's pane, for example: "Add a billing page: API endpoint, the page itself, and end-to-end tests."

You can also ask it to add, assign, delegate and show tasks. It keeps them in `TASKS.md`.

By default it lists the threads it suggests and waits. Reply with a go-ahead that names them ("start all three"). Your agent then asks permission to run `thread start` for each one, unless you've allow-listed it (see [Operations](operations.md#the-allow-list-for-your-coordinator)).

## 5. Confirm the threads appear

Each code thread opens as its own workspace on a branch named `hp/<project>/<id>-<title>`; a task with no repository opens as a tab in the project's workspace. Expand Herdr's agent sidebar to see `project`, `thread` and `review` beside each one, or run **Farm: overview**.

Codex workers start with launch-only trust for their worktree and repository root,
a workspace-write sandbox and on-request approval. Network is off; set
`thread_network = true` in the project's safety settings when workers need
downloads. Claude workers use `--permission-mode acceptEdits` and a built-in
`--allowedTools` list for local Git workflows and shell reads. To pre-approve a
project command, add `thread_allowed_commands = ["godot --headless:*",
"tools/run_tests.sh:*"]` under `[safety."<canonical project path>"]` in the
config file printed by `safety show`. These validated prefixes extend Claude's
defaults; Codex workers use their sandbox instead. Legacy Claude workers run
with your permissions: prefix approvals do not enforce worktree confinement.
`safety show` and `doctor` display the effective rules. A blocked command may
be outside the allow-list; inspect its pane before approving or adding a prefix.
The first thread in a new repository may still need you to answer Claude's folder-trust
dialog in its pane. `thread list` preserves the `blocked` state and shows a
separate hint column; `doctor` flags possible trust or permission prompts.
An explicit `thread_agent_args` replaces the defaults,
including an empty array. See [Operations](operations.md#safety-settings).

When a thread finishes it writes a report. The report is copied to `threads/<id>.md` in the project folder and the thread moves to Ready for review. Tell the coordinator you've looked (it runs `thread ack`), or resolve the thread:

```bash
herdr-farm thread resolve billing t-0001                     # keep the worktree
herdr-farm thread resolve billing t-0001 --remove-worktree   # remove it; the branch is kept
```

## Check your setup

```bash
herdr-farm doctor
herdr-farm ticker status
```

- **`open` says the session is not reachable**: run it inside Herdr, or pass `--session <name>`. A project belongs to the session it was first opened in; opening it from another one is refused.
- **A thread stays at "no agent"**: the ticker launches agents, one per project per tick (about 15 seconds). `ticker status` shows whether it runs and which `herdr`, `git`, `gh`, `ssh` and `rsync` it resolves from its own environment, which may differ from your shell. After three failed launches the thread is marked failed with the reason; `thread restart` tries again.
- **Herdr was restarted**: panes are gone but records, reports and branches are not. Run `open <project>` for a new coordinator and `thread restart <project> <id>` for each thread you want back. The coordinator starts with no chat history; it works from memory, thread records and the inbox. To keep your agent's own history, set `coordinator_agent_args = ["--continue"]` together with `coordinator_agent_args_kind = "claude"` (for Claude Code).
- **The coordinator forgot how to behave** after a long conversation: `herdr-farm open <project> --reprime`.

## Optional configuration

User-level settings live in `~/.config/herdr-farm/config.toml`, which you edit by hand. No agent works in that folder.

```toml
# Where projects live (default ~/.herdr-farm). HERDR_FARM_ROOT and --root win over this.
root = "~/projects"

# Only needed when `herdr machine list --json` shows no SSH target for a machine.
[machines.buildbox]
ssh = "me@buildbox.local"
```

Safety settings are per project, in the same file. `herdr-farm safety show <project>` prints the effective values and the exact table header to add.

## Upgrade

```bash
herdr plugin install hilather/herdr-farm
herdr-farm ticker start
```

A rebuilt binary has a new build identifier. `ticker start` (also run by `open` and `thread start`) stops a ticker of another version and starts the new one; it never replaces a healthy ticker of the same version.

## Remove

```bash
herdr-farm ticker stop
herdr plugin uninstall herdr-farm      # or: herdr plugin unlink herdr-farm
```

Your projects stay in `~/.herdr-farm/` and your settings in `~/.config/herdr-farm/`; delete them yourself if you no longer want them. Worktrees and branches that threads created are yours: nothing removes them for you.

## Troubleshooting

- **`plugin_requires_newer_herdr` although `herdr --version` says 0.9.1**: the running server is older than the CLI. Restart it (`herdr status` shows `server_binary_stale`).
- **`thread restart` says a half-made worktree needs a human look**: a failed `git worktree add` can leave the branch behind. Run `thread resolve`, delete or reuse the branch yourself, and start a new thread.
- **`thread resolve --remove-worktree` refuses**: either the worktree has uncommitted changes (Herdr's refusal is shown unchanged; nothing is ever forced), or part of the thread's files could not be copied home first. The message lists what was not copied; `--discard-uncopied` accepts that loss.
- **No nudge reaches the coordinator**: that is the default. See [Operations](operations.md#nudges-and-notifications).

Compatibility: legacy environment variables and existing data/config locations remain supported; see [renaming](renaming.md).

## Open the coordinator after migration

After migration, open the coordinator with `herdr-farm open PROJECT` in the
owner's Herdr session, or pass `--socket /absolute/session.sock`. The canonical
coordinator uses the configured coordinator agent and safety arguments, is bound
with live ownership/observation, and is primed with the canonical skill and state.
Run `herdr-farm context PROJECT` for task/attempt/inbox/result state and the exact
commands. Dispatch uses `task PROJECT add` and owner-signed `launch PROJECT run`;
review uses `result PROJECT show` and verification/integration jobs. `thread`
commands refuse with canonical replacements. If the coordinator pane was closed,
run `herdr-farm open PROJECT --reprime`. See the [operator runbook](operator-runbook.md#canonical-coordinator-after-migration-w-coord-2)
for signing configuration, safety semantics and interrupted-effect recovery.
