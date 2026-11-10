# README screenshots

`sync-review.png` and `sync-preview.png` show the implemented React synchronization window. They were captured with headless Edge against the local Vite app on 2026-11-10.

The UI uses synthetic example conversations and the saved Codex response from the two-repository integration evaluation (`scripts/evaluate-sync.mjs`). Tauri IPC is replaced by fixture responses for these captures. The displayed root is a generic example path, `~/Documents/knowledge`.

These are UI illustrations, not evidence of a complete native UI-to-model-to-Git test. The real model/Git evaluation and its limits are recorded separately in [VALIDATION.md](../../VALIDATION.md).
