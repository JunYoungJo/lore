# Using Lore on multiple PCs

## Platforms and file locations

| Item | Windows | macOS | Linux |
|---|---|---|---|
| App data | `%LOCALAPPDATA%\io.github.junyoungjo.lore` | `~/Library/Application Support/io.github.junyoungjo.lore` | `$XDG_DATA_HOME/io.github.junyoungjo.lore` or `~/.local/share/io.github.junyoungjo.lore` |
| App executable | `lore.exe` | `lore` | `lore` |
| Hook CLI | `lore-cli.exe` | `lore-cli` | `lore-cli` |
| Package format | NSIS | app, dmg | deb, AppImage |
| Popover | Bottom right of the screen | Below the menu bar, on the right | ‘Open’ in the tray menu |

Lore uses Tauri's [platform-specific configuration merging](https://v2.tauri.app/develop/configuration-files/). Linux [does not support tray click events](https://v2.tauri.app/learn/system-tray/), so use ‘Open’ in the menu. The Linux desktop needs tray/AppIndicator support.

`LORE_HOME` is an optional environment variable that replaces the whole app data folder. Use it for isolated testing or nonstandard installs. If only the app gets this variable and Claude and Codex do not, they use different queues, so in real use set the same value for the processes that call the hooks as well.

When launched from the macOS Finder or at login, the terminal's PATH may not be passed on. Besides the default PATH, Lore looks in `~/.local/bin`, `~/.bun/bin`, `~/.cargo/bin`, `~/.npm-global/bin`, and the Node install directories of Homebrew and nvm. For other install methods, specify the executable and arguments in the `commands` array of the local `settings.json`. Lore does not run shell init files to find paths.

## Importing existing Windows memory

1. Move the app source and the memory repository separately. The source in a Git bundle alone does not restore `D:\doc\lore-memory` or the operational data in `%LOCALAPPDATA%`.
2. Bring the memory repository's `common/`, `projects/` and `sources/` together with its Git history. `sources/` holds the source conversations, so bringing only the notes leaves the sources unverifiable.
3. In the new PC's `settings.json`, set the **absolute path** of that PC's memory repository. The default for a new install is `~/Documents/lore-memory`. A Windows path set as is on a Mac is rejected.
4. Use `lore-cli doctor` to check what was found for Codex, Claude and qmd, and where the qmd configuration lives. This command does not change settings, the DB or hooks. If the installed CLI is not on PATH, run the executable in the install folder by its absolute path. The CLI inside the macOS app is `Lore.app/Contents/MacOS/lore-cli`.
5. With the tray app quit, view the migration plan with `lore-cli portable-links`. Add `--apply` to migrate. Even without the original session files or Windows drives, the exact turns are found in the imported session copies. If a copy or the exact line cannot be found, it is reported and all links to that copy are kept.
6. Create the new PC's qmd collections and index with `lore-cli index`. It registers `common` and every valid imported project folder. This step needs a real qmd install and model downloads.
7. Set `QMD_CONFIG_DIR` and `XDG_CACHE_HOME` of the qmd MCP used by Codex and Claude to `doctor`'s `qmdConfigDir` and `qmdCacheDir`. Match the index name to `qmdIndex` as well. Older connections pointing to `.qmd/config` must be updated to search new recordings.
8. Launch the app and explicitly connect the hooks for the providers you want. Also check the trust setting in Codex.

The local queue DB is not shared with other PCs. The queue holds absolute source paths and processing positions, so this version does not convert unprocessed jobs in a Windows queue to Mac paths. Process jobs left on Windows on that PC, or prepare the sources and working repository paths separately.

## Stable project ID

Commit the following file at the root of each source repository.

```json
{"project":"demoengine"}
```

The ID must start with a lowercase letter or digit, use only letters, digits, `-` and `_`, and be at most 100 characters. To keep using existing `projects/demoengine/` memory, set the same ID. Give different projects different IDs. The app does not check that an ID is globally unique.

Without the file, identification by folder name continues as before. An invalid ID file is reported as a recording failure. A worktree uses the ID of its original repository. `lore-cli project` shows the ID the current repository will use. Existing memory folders are not renamed or merged automatically.

## Source links and migration recovery

New links use a path relative to the note and the original turn number.

```markdown
[L42](<../../sources/claude/session-id.md#L42>)
```

`sources/` is not indexed by qmd. When a later agent reads `qmd://project-demoengine/topic.md`, the actual note is `<knowledgeRoot>/projects/demoengine/topic.md`, and the relative source path resolves from that location. `#L42` is the `<a id="L42"></a>` anchor in the copy, not line 42 of the physical file.

Migration covers session-copy links in `common/`, `projects/` and the root `README.md`. If you wrote other files by hand with absolute-path links, check them separately. Very old links pointing directly to the original jsonl are handled by the existing `migrate-links` command. Move them to copies first, then run `portable-links`.

Before applying, the full original and changed content is saved to `migration/portable-links.pending.json` in app data. If it is interrupted, running the same command again continues only with files that match either the original or its own applied result. Files changed externally are not overwritten. After completion, the plan is kept as `portable-links-<timestamp>.backup.json`. After migrating, update note search with `index`.

## Sharing scope and concurrent recording

What is shared through Git is the memory Markdown and the session copies. Settings, the SQLite queue, processing history, model inputs/responses, and qmd configuration and cache stay in per-PC app data. The `.qmd/` folder in an existing memory repository is no longer used for new indexes and is not deleted automatically.

When the memory repository has a remote and upstream, the app commits recordings, merges them with other PCs' commits and pushes. **Auto-sync** is off by default; when on, it syncs right after a recording finishes and every 5 minutes. Even when both sides have new commits, it merges without asking. If two PCs changed the same line, both sides' lines are kept and the note is added to the items to check; notes changed on both PCs are compared with the model after the sync. The sequence of steps and failure handling are in the [sync guide](sync-review.md).

There is no distributed lock. If another PC pushes first during a transfer, the app fetches that commit, merges again and then pushes; it never force-pushes. Review state (`refs/lore/reviewed`) and items to check are kept separately on each PC.

## Verification scope

CI is set up with Rust checks, UI checks and native app builds on Windows, macOS and Linux. Having CI does not mean real runs have been completed on all three platforms. Real hook events, notifications, launch at login and qmd search must be checked in a real install on each OS.
