# Renaming to herdr-farm

The product is now herdr-farm, maintained in the new public repository `hilather/herdr-farm`. Historical reviews, acceptance records, ADRs, progress logs and certification evidence retain their original wording.

| Item | Old | New |
|---|---|---|
| Package and binary | `herdr-projects` | `herdr-farm` |
| Product environment prefix | `HERDR_PROJECTS_*` | `HERDR_FARM_*` |
| Default root | `~/.herdr-projects` | `~/.herdr-farm` |
| Config | `~/.config/herdr-projects/config.toml` | `~/.config/herdr-farm/config.toml` |
| Herdr plugin id | `herdr-projects` | `herdr-farm` (display name: Farm) |

Root selection is `--root`, then `HERDR_FARM_ROOT`, then `HERDR_PROJECTS_ROOT`, then the selected config's `root`, then an existing `~/.herdr-farm`, then an existing `~/.herdr-projects`, otherwise a new `~/.herdr-farm`. The new config file wins when present; otherwise the legacy config file is used. All product environment reads try the new prefix first and honor the old prefix when the new variable is unset. `doctor` reports the selected root and config directory and warns when both old and new locations exist. No data is moved or copied automatically.

Re-install the plugin from this checkout: the id change creates a distinct Herdr plugin registration. Remove the old registration through Herdr's plugin management before enabling the replacement, so two tickers are not started. Old plugin state is not automatically migrated. Existing scripts that invoke the old binary need their command updated or an operator-managed compatibility symlink.

## Existing installations

No root move is needed: the old location keeps working through the root and config fallback described above. Keep existing canonical projects at their recorded location.

Moving a root that contains canonical projects is unsupported. Their migration journals, worktrees and bindings record absolute paths; a symlink at the old location does not relocate those identities. If a root was moved and commands report that a project was recorded at a different path, move the root back and run `herdr-farm doctor`.

A root with no canonical projects (including one containing only legacy projects) can be moved. Stop its workers and tickers first, back up the selected data, and ensure the destination does not exist. Update explicit `root` settings, scripts, service definitions and exported environment variables after the move. Inspect `herdr-farm doctor` and the projects before restarting workers. Do not overwrite or merge existing stores. Retained launch evidence and pinned configurations can still require the old absolute paths; moving a legacy root does not rewrite them.

The legacy config directory also remains usable through the fallback; renaming it is optional. Keep any configured root pointing at the existing canonical projects.

Worker submission environments set `HERDR_FARM_SUBMISSION_SPOOL` and the legacy `HERDR_PROJECTS_SUBMISSION_SPOOL` to the same directory, so retained briefs and scripts continue to submit. New briefs invoke `herdr-farm`. New popup bindings use `HERDR_FARM_ROOT` and `HERDR_FARM_HANDOFF`; reads accept the legacy names.

Signature namespaces ending in `@herdr-projects`, internal persisted job ids, worktree intent tags, capture Git identity, memory projection markers, host identity and export cursor hash salts deliberately retain their old spelling. SQLite schemas, migrations, event kinds, telemetry streams, signed documents and certification registry identifiers are unchanged.
