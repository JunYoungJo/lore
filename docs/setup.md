# Setup and integrations

Lore opens an introduction on a fresh installation. Existing installations can open it through **Settings → Advanced → Run setup guide again**. Ongoing repairs and advanced controls have a separate **Manage integrations** window, opened from **Settings → Connections → Manage connections**. **Settings → Memory & sync → Change…** opens it on the memory folder with the change form open. Both interfaces are in English.

## First-run experience

1. **Meet Lore:** an interactive, clearly labeled example shows a project decision becoming a source-linked note and being recalled later. This illustration does not record a real conversation or claim that search has been tested.
2. **Your tools:** select Codex and/or Claude Code. Installed CLIs are suggested; no client settings change from selecting a tool.
3. **Your memory:** use the native folder picker, create a new Git repository in an empty folder, or prepare a Git clone. The saved folder contains `common/`, `projects/`, and `sources/`. This does not create a GitHub repository or change the app-wide default path.
4. **Get connected:** check runtime compatibility and CLI sign-in, then show only the next missing prerequisite or sign-in. The terminal appears when an installation or sign-in is requested. After prerequisites are ready, the user can inspect **What will change?** and select **Connect & prepare memory**. Lore builds the index, applies the selected client connections, and saves setup completion after checking their configuration.
5. **Start remembering:** explain client restart, Codex hook trust, and the first recording/recall exercise. **Start using Lore** is available without waiting for an external coding session. The screen explicitly says the first recording and AI recall have not been verified.

A fresh index uses Qwen3-Embedding-0.6B; an existing index retains its model until it is changed in **Manage Lore → Search index**. Recording and sync share an operation gate with indexing. The recording CLI remains the saved choice when it is among the selected tools; otherwise a selected tool is used. The recording model and effort are chosen in **Settings → Recording**.

The current position and client choices are saved in `setup.json` in local app data. **Set up later** defers the introduction; reopen it from Settings to resume. Existing tools are reused. Setup checks run only when preparation is reached, not while browsing the welcome example.

Closing the setup window retains a running terminal command. **Stop** terminates its process tree. Terminal output is bounded and kept in memory, not saved as a transcript or sent to the recording model. Terminal setup sessions have `LORE_RECORDER=1` to avoid recording them as project work. Commands are executed as argument arrays; a review token prevents running a different command after configuration changes.

## Ongoing management

**Manage integrations** (titled **Manage Lore**) is separate from the introduction. Its left list shows each item with its state: Codex and Claude Code under **AI tools**, and **Memory folder**, **Search index** and **Tools** under **This PC**. The header counts the items that need attention. It opens on the first such item.

- **AI tool pages** list each part Lore installs (qmd MCP, memory guidance, memory search skill, session hooks) with its file and state, and say why a part needs repair. For hooks that run another copy of Lore, the page names both locations. The account row shows the CLI sign-in state from the same check the introduction uses, and offers **Sign in** when needed. **Connect** or **Repair connection** applies the reviewed plan; **Review changes** shows the content Lore writes. **Disconnect** is a link at the end of the row. The Codex page explains hook trust and can open Codex in the terminal.
- **Memory folder** shows the folder, whether it is a Git repository, and its branch and upstream. **Change…** opens the existing/new/clone form; after a change, the list shows that the index must be built and the connections repaired for the new folder.
- **Search index** shows the embedding model, collections and local paths, and can refresh or build the index. **Change…** switches between Qwen3-Embedding-0.6B and embeddinggemma-300M, the two model families qmd 2.8.3 formats queries for. Lore writes `models.embed` in the index config and runs `qmd update` and `qmd embed -f`. The whole index is rebuilt on this PC; semantic search returns nothing until it finishes, and open AI sessions should be restarted afterwards. Another PC is not affected.
- **Tools** shows Node.js, Git, qmd (and Homebrew SQLite on macOS) with their paths and offers an installer only for what is missing.

The embedded terminal appears at the bottom only after an install, sign-in, clone or Codex command is prepared, and can be closed when no command is running. A configuration conflict routes the introduction here for review. It does not force the user to repeat the introduction.

## Recording model and effort

**Settings → Recording** lists the models each CLI reports for the signed-in account. Lore reads the lists once at launch, and again after a command in the setup terminal finishes or **Check again** is pressed: Codex with `codex debug models` (models marked for listing, in Codex's order), Claude Code with the `initialize` response of `claude --print --input-format stream-json` in safe mode, without sending a message. Neither is a documented, stable interface; when a list cannot be read, the field falls back to free text and recording continues with the saved value.

- **Default** for Codex is the first listed model (`gpt-6.1-sol` when no list has been read); for Claude Code it is the `haiku` alias. Choosing a Claude alias keeps following the CLI's newest model of that name.
- **Other…** accepts a model name that is not listed.
- **Effort** lists only the levels the chosen model reports. **Default** keeps Lore's previous behavior: `low` for Codex, and no `--effort` for Claude Code. A level the next model does not support is reset to Default. Higher levels make each recording slower; raise **Recording timeout** if recordings time out.
- When the saved model is not in the CLI's list, the tray shows it under **Needs attention**.

## Client files

| Resource | Codex | Claude Code |
| --- | --- | --- |
| qmd MCP entry | `$CODEX_HOME/config.toml`, otherwise `~/.codex/config.toml` | `~/.claude.json` |
| Short memory guidance | `$CODEX_HOME/AGENTS.md`, otherwise `~/.codex/AGENTS.md` | `~/.claude/CLAUDE.md` |
| Memory search skill | `~/.agents/skills/lore-memory/SKILL.md` | `~/.claude/skills/lore-memory/SKILL.md` |
| Session hooks | existing Lore hook configuration | existing Lore hook configuration |

MCP is named `qmd` and uses the same command, index name, `QMD_CONFIG_DIR`, and `XDG_CACHE_HOME` as this PC's Lore settings. It does not copy another computer's paths. Hook registration uses the CLI bundled beside the running app; apply the connection again after moving from a development executable to an installed app.

Restart the AI client after installation. In Codex, the user reviews/trusts the hooks through `/hooks`; use `/mcp` to check the server. Lore does not approve trust or inspect another AI session's recall. Configuration readiness and actual memory use are separate states.

## Repair and removal

Client settings are changed only by an explicit connection or disconnection action. **Check again** refreshes Lore's executable detection when a tool was installed or moved. **Apply connection** in management changes only the reviewed resources and registers missing Lore hooks. If a file or index path changed after preview, the user must review it again.

Lore records ownership of the MCP entry, its guidance section, and skill in `integrations/<client>.json` in app data. It can update these resources when paths change. An equivalent preexisting entry is reused without claiming ownership. A preexisting Lore guidance section is kept as written, including custom wording. Other instruction sections and installer markers, such as `codebase-memory-mcp`, are kept.

If an unowned qmd entry or skill conflicts, or an owned resource was edited outside Lore, automatic replacement is blocked. The preview identifies the affected file. After resolving it, check and review again. A connection failure may leave some resources installed; reopening the guide shows their current state so it can be retried.

**Disconnect** removes only the still-matching resources Lore owns and Lore's session hooks. It preserves user edits and reused entries, and reports retained edited files. It does not restore an entire old client configuration, uninstall dependencies, delete memories, remove model caches, or revoke client trust. The older **Hook connections** controls and CLI `connect`/`disconnect` still manage hooks only. `lore-cli integrations disconnect <claude|codex>` also removes the unchanged owned MCP, skill, and guidance. The Windows uninstall hook uses this cleanup command on ordinary uninstall; its existing update-mode guard is retained.

## Platforms and limits

- macOS: prepared Node/Git/SQLite installation uses an existing Homebrew installation. If Homebrew is missing, follow its official instructions and check again. Node 24's Homebrew bin directory is included in executable discovery.
- Windows: prepared Node/Git installation uses an existing `winget`; npm installs qmd and optional coding CLIs. Supported npm wrapper scripts are called through PowerShell with individually quoted arguments. Standard Node, Git, and user npm directories are included in discovery.
- Linux: use the official Node/Git links or your distribution's package manager for those base tools. The guide offers npm installation commands once that runtime is ready.
- Node must be 22 or newer. Preparation runs a version check and reads the selected CLI login statuses. These probes never invoke a recording model; the returned UI state contains readiness, not credentials.
- The first embedding-model download is about 640 MB and indexing can take several minutes. Errors remain visible and the step is retryable. Index commands use the saved qmd timeout; adjust it in Settings if needed.
- Git authentication remains with Git and its credential helpers. Cloning does not configure an upstream for an unrelated folder, enable automatic sync, or set commit author identity.
- The suggested first-memory exercise uses a real recording account and its allowance. Perform it in a normal project session, not the setup terminal. It is not required to finish setup; no synthetic memory is written automatically.

References: [qmd](https://github.com/tobi/qmd), [Codex skills](https://learn.chatgpt.com/docs/build-skills), [Claude Code MCP](https://code.claude.com/docs/en/mcp), [portable-pty](https://docs.rs/portable-pty/0.9.0/portable_pty/), [xterm.js](https://xtermjs.org/docs/).
