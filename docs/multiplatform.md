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

## Using the memory on another PC

1. Clone the memory repository on the new PC. It holds `common/` and `projects/` with their Git history; that is everything the notes need.
2. In the new PC's `settings.json`, or in the setup guide, set the **absolute path** of that PC's clone. The default for a new install is `~/Documents/lore-memory`. A Windows path set as is on a Mac is rejected.
3. Use `lore-cli doctor` to check what was found for Codex, Claude and qmd, and where the qmd configuration lives. This command does not change settings, the DB or hooks. If the installed CLI is not on PATH, run the executable in the install folder by its absolute path. The CLI inside the macOS app is `Lore.app/Contents/MacOS/lore-cli`.
4. Create the new PC's qmd collections and index with `lore-cli index`. It registers `common` and every valid project folder. This step needs a real qmd install and model downloads.
5. Set `QMD_CONFIG_DIR` and `XDG_CACHE_HOME` of the qmd MCP used by Codex and Claude to `doctor`'s `qmdConfigDir` and `qmdCacheDir`. Match the index name to `qmdIndex` as well.
6. Launch the app and explicitly connect the hooks for the providers you want. Also check the trust setting in Codex.

The local database is not shared with other PCs. It holds the queue of sessions, with their absolute paths and reading positions, and the candidates that wait for review. A session is read, and its candidates are approved, on the PC where it took place.

## Stable project ID

Commit the following file at the root of each source repository.

```json
{"project":"demoengine"}
```

The ID must start with a lowercase letter or digit, use only letters, digits, `-` and `_`, and be at most 100 characters. To keep using existing `projects/demoengine/` memory, set the same ID. Give different projects different IDs. The app does not check that an ID is globally unique.

Without the file, identification by folder name continues as before. An invalid ID file is reported as a recording failure. A worktree uses the ID of its original repository. `lore-cli project` shows the ID the current repository will use. Existing memory folders are not renamed or merged automatically.

## Where the evidence is

A note ends with a `## Sources` section that the program writes. Each line is something that was said in a session and that an approved candidate rests on: the day, who said it, and the words.

```markdown
## Sources
- 2026-11-17, user: "Finished jobs get no desktop alert."
- 2026-11-17, agent: "The export timed out because it loaded every row at once."
```

No copy of a conversation is written into the memory folder, so the evidence of a note is in the note and moves with it to every PC.

## Sharing scope and concurrent recording

What is shared through Git is the memory Markdown. Settings, the SQLite queue, the candidates waiting for review, processing history, model inputs/responses, and qmd configuration and cache stay in per-PC app data. A candidate is approved on the PC whose session produced it.

When the memory repository has a remote and upstream, the app commits the notes it writes, merges them with other PCs' commits and pushes. **Auto-sync** is off by default; when on, it syncs right after notes are written and every 5 minutes. Even when both sides have new commits, it merges without asking. If two PCs changed the same line, both sides' lines are kept and the note is added to the items to check; notes changed on both PCs are compared with the model after the sync. The sequence of steps and failure handling are in the [sync guide](sync-review.md).

There is no distributed lock. If another PC pushes first during a transfer, the app fetches that commit, merges again and then pushes; it never force-pushes. Review state (`refs/lore/reviewed`) and items to check are kept separately on each PC.

## Verification scope

CI is set up with Rust checks, UI checks and native app builds on Windows, macOS and Linux. Having CI does not mean real runs have been completed on all three platforms. Real hook events, notifications, launch at login and qmd search must be checked in a real install on each OS.
