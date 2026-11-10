<div align="center">

<img src="docs/brand/256x256.png" alt="Lore" width="112" height="112" />

# Lore — Project Memory for Coding Agents

**Keep the decisions. Remember the reasons.**

Turn Claude Code and Codex conversations into source-linked project knowledge.<br />
Keep it in Markdown, find it with qmd, and carry it between PCs with Git.

[Get started](#getting-started) · [Sync across PCs](#sync-across-pcs) · [Documentation](#documentation) · [한국어](README.ko.md)

![Status](https://img.shields.io/badge/status-in%20development-c0851a)
![Tauri 2](https://img.shields.io/badge/Tauri-2-24C8DB?logo=tauri&logoColor=white)
![Rust](https://img.shields.io/badge/Rust-000000?logo=rust&logoColor=white)
![React](https://img.shields.io/badge/React-18-61DAFB?logo=react&logoColor=black)
![TypeScript](https://img.shields.io/badge/TypeScript-3178C6?logo=typescript&logoColor=white)

<img src="docs/screenshots/main.png" alt="Lore main tray window in dark mode: recording status, queue counts, and recent memory notes" width="400" />
<img src="docs/screenshots/main-light.png" alt="Lore main tray window in light mode" width="400" />

<sub>The main tray window in dark and light themes. Screenshots use synthetic example data; the UI is in English.</sub>

</div>

## Why Lore?

“Why did we choose this?” often takes longer to answer than “What does the code do?” A new coding session may have the files but miss the rejected approach, the constraint behind a decision, or the reason yesterday's plan changed.

Lore preserves that context as topic-based notes: decisions and their reasons, problems worth remembering, and failures and solutions. Each note links back to the conversation that supports it. When a decision changes, the aim is to retain its history and explain the change.

## Features

- **Capture as you work** — Claude Code and Codex hooks enqueue conversations at session end and before compaction. The tray app processes them in the background, resuming from the last recorded position.
- **Notes organized by topic** — A configured coding model updates project and shared knowledge instead of creating a report for every session.
- **Checkable sources** — Relative links point to saved conversation copies and stable turn anchors, so evidence can travel with the notes.
- **Local Markdown, searchable context** — Notes live in a folder you choose. [qmd](https://github.com/tobi/qmd) indexes them for search and retrieval through its MCP server.
- **Review before committing** — Compare local changes side by side and choose which notes to commit. Conversation recording itself writes notes automatically; this review controls the Git commit.
- **Git sharing with conflict review** — Compare diverged histories, inspect the supporting conversations, choose a proposed resolution, and preview the resulting documents before applying and sharing.
- **Recoverable background work** — View pending and failed jobs, retry them, or skip a failed recording segment. Synchronization also retains its state after indexing or push failures.
- **Tray controls** — Pause recording, configure the model and hooks, and switch between light and dark themes.

<details>
<summary>Settings and session queue</summary>

<img src="docs/screenshots/settings.png" alt="Recording model and hook connection settings" width="400" />
<img src="docs/screenshots/queue.png" alt="Session queue with recording and pending groups" width="400" />

</details>

## How it works

```text
Claude Code / Codex session ends or reaches compaction
  → Hook adds the transcript to a local queue
  → Model proposes changes to project and shared notes
  → Lore validates paths, quotations, source links, and edit locations
  → Notes and conversation copies are saved; qmd refreshes its index
  → A later session can find relevant context through qmd MCP
```

The model decides what is worth recording. Program checks catch malformed output and invalid edits; they do not prove that a memory is correct. Future agents also need qmd MCP and instructions to consult project memory—recording alone does not make every session use it.

## Getting started

Lore is in development. Build from source using the instructions below.

### Requirements

- An authenticated Codex or Claude CLI for the recording model.
- [qmd](https://github.com/tobi/qmd) installed and usable on this PC for indexing and search.
- Git 2.38+ for the synchronization workflow, plus a remote and branch upstream when sharing between PCs.
- For building: stable Rust, Node.js 24, pnpm, and the [Tauri 2 prerequisites](https://v2.tauri.app/start/prerequisites/) for your OS.

Model calls use the selected CLI's authentication and usage limits. They may incur provider charges. Conversations and relevant notes are sent to that provider for processing.

### Build and run

From a checkout of this repository:

```sh
pnpm install --frozen-lockfile
pnpm tauri dev                 # Development app with frontend hot reload
pnpm tauri build               # Desktop bundle for the current platform
```

The build includes the hook CLI as a sidecar. The product name is now **Lore**; the executable, CLI, and data identifiers still use **`qmd-memory`**. Commands below use those existing names. The CLI is not automatically added to your shell's PATH; use its full path when needed. In a macOS bundle it is `Lore.app/Contents/MacOS/qmd-memory-cli`; on Windows it is `qmd-memory-cli.exe` beside the app.

### Set up your memory

1. Open the app's settings. Choose a memory folder—the default is `~/Documents/knowledge`—and select the recording CLI and model.
2. Quit the tray app, then run the bundled CLI to check dependencies and initialize this PC's index:

   ```sh
   qmd-memory-cli doctor
   qmd-memory-cli index
   ```

3. Configure qmd MCP in the coding tools that should read the memory. Match `QMD_CONFIG_DIR`, `XDG_CACHE_HOME`, and the index name to the paths and values reported by `doctor`. Give those agents instructions to search the memory when relevant.
4. Reopen the app and explicitly connect Claude Code and/or Codex in **Settings → Hook connections**. Trust the new hooks in Codex's `/hooks` interface when required.
5. Work in a Git project. When a connected session ends or compacts, check the tray app for the recording result.

Hooks can enqueue work while the app is closed. The app processes that queue when it starts again. Sessions whose hooks never ran are not automatically scanned; `qmd-memory-cli scan` is available for manual recovery.

For an existing Windows installation or older absolute source links, follow the [migration guide](docs/multiplatform.md#기존-windows-기억-가져오기) before recording on another PC.

## Sync across PCs

Clone the same **memory repository** on each PC and set each app's memory folder to its local checkout. Give source projects a stable [project ID](docs/multiplatform.md#고정-프로젝트-id) when checkout names differ.

1. Review and commit your local notes.
2. Open **Sync** and check the configured upstream.
3. If only one side has new commits, Lore synchronizes that history and refreshes this PC's index.
4. If the histories diverge, inspect the comparison. Changes within the same project, or involving shared notes, require a model comparison even when Git reports no line conflict.
5. Read the evidence, choose a resolution for each issue, inspect the full preview, and apply it. Lore creates a merge commit, refreshes the index, and pushes without force.

<img src="docs/screenshots/sync-review.png" alt="English sync review showing source evidence and resolution options" width="900" />

You can resolve an issue by choosing an option; editing a document by hand is optional. **Sync every five minutes is off by default.** When enabled, it checks committed history and pauses new memory recording when review is needed. Model comparison and final approval remain manual. Uncommitted files are not automatically committed or stashed.

<details>
<summary>See the final document preview</summary>

<img src="docs/screenshots/sync-preview.png" alt="Full document preview showing the chosen policy, preserved history, and source links before applying" width="900" />

</details>

See the [sync review guide](docs/sync-review.md) for retrying, deferring, and handling unsupported conflicts. The model can miss semantic contradictions; review its evidence and proposed wording.

## Data and privacy

```text
knowledge/
├── common/                  # Shared notes
├── projects/<project-id>/   # Project notes
└── sources/<provider>/      # Conversation copies referenced by notes
```

The queue, settings, model execution records, and qmd indexes stay in per-PC app data. They are separate from the memory folder. New evidence links are relative to the note and use source-turn anchors.

Git sharing sends **all committed files in the memory repository**. Conversation copies can contain more than the quoted sentences. Choose the repository's visibility and contents accordingly. Lore's model processing is not fully offline: the configured coding CLI receives the conversations and context used for each request.

## Platform status

| Platform | Build configuration | Verification on this branch |
| --- | --- | --- |
| macOS | App / DMG | Apple Silicon app build and isolated startup checked; signing and notarization not checked |
| Windows | NSIS installer | Platform code and CI configured; not run on Windows during this branch's validation |
| Linux | DEB / AppImage | Platform code and CI configured; desktop runtime not checked |

Windows was the original development platform. Current Mac validation does not establish that every hook, notification, or desktop integration works on every OS. Full results and remaining limits are in [VALIDATION.md](VALIDATION.md).

## Development

```sh
pnpm build:cli                    # Required before Rust tests on a fresh checkout
cargo test --workspace --locked
pnpm test
pnpm typecheck
```

Automated tests use fixtures and temporary repositories without calling real models or qmd. Separate opt-in evaluations call the configured model and incur usage:

```sh
node scripts/evaluate-memory.mjs --backend codex
node scripts/evaluate-sync.mjs --backend codex
```

Both evaluations substitute an indexing fixture for qmd. They test model output and application behavior, not real search quality or long-term usefulness.

| Path | Responsibility |
| --- | --- |
| `crates/core/` | Transcript parsing, recording, validation, queue, qmd integration, Git synchronization |
| `crates/cli/` | Hook receiver, diagnostics, manual processing and recovery |
| `src-tauri/` | Desktop runtime, tray, windows, notifications, background processing |
| `src/` | React and TypeScript UI |
| `scripts/` | Build helpers and model evaluations |

## Documentation

- [한국어 소개와 시작하기](README.ko.md)
- [Detailed usage and CLI reference](docs/usage.md) — Korean
- [Multiple PCs, platform paths, and migration](docs/multiplatform.md) — Korean
- [Synchronization review and recovery](docs/sync-review.md) — Korean
- [Validation results](VALIDATION.md) — Korean
- [Project history and original scope](docs/origin-and-scope.md) — historical notes

## License

A distribution license has not been selected. The Rust workspace is currently marked `UNLICENSED`.
