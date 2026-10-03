# herdr-farm

herdr-farm coordinates coding agents in [Herdr](https://herdr.dev). Keep one coordinator conversation, give workers isolated tasks and worktrees, and inspect results, approvals and usage from a local project store. This repository is maintained as [hilather/herdr-farm](https://github.com/hilather/herdr-farm).

## Features

- Canonical workers through `herdr-farm launch PROJECT run`: Codex and Claude, shared owner login or setup-token login, frozen launch specifications and Linux worker sandboxing.
- Result submission, verification and integration, with automatic completion after accepted results reach the integration ref.
- Telemetry collectors, a usage ledger and published-rate cost accounting; outcomes, views, health, export and comparisons; assignment policies in shadow mode and a workspace pane.
- Live-certified agent versions, with certification evidence and explicit admission requirements.
- Coordinator conversations, shared project memory, parallel threads and an overview of work that needs operator attention.
- An [operator runbook](docs/operator-runbook.md) for launch, recovery, review and ongoing operations.

## Quick start

Install Rust and Herdr 0.9.1 or later, then build the CLI and install the plugin:

```sh
cargo build --release --locked
herdr plugin install /path/to/herdr-farm
/path/to/herdr-farm/target/release/herdr-farm new demo
/path/to/herdr-farm/target/release/herdr-farm doctor
/path/to/herdr-farm/target/release/herdr-farm open demo
```

Add `target/release` to your `PATH` to use `herdr-farm` directly. Start with [getting started](docs/getting-started.md), then [profiles](docs/profiles.md) and [canonical worker launch](docs/canonical-worker-launch.md). Canonical launches require the signed policies and approvals described there; creating a project alone does not authorize workers.

New installations use `~/.herdr-farm` and `~/.config/herdr-farm/config.toml`. Existing installations retain their old locations through automatic selection, without copying or moving data. `HERDR_FARM_*` takes precedence over legacy `HERDR_PROJECTS_*`. See [rename compatibility and manual migration](docs/renaming.md).

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

## Credits

herdr-farm started as a fork of [herdr-projects](https://github.com/eliasstravik/herdr-projects) by Elias Stravik (MIT).

The MIT license and original copyright are retained in [LICENSE](LICENSE).
