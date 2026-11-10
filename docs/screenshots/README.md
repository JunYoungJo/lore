# README screenshots

All screenshots show the implemented React UI in English, captured with headless Edge against the local Vite app on 2026-11-10. Tauri IPC is replaced by fixture responses for these captures; they are UI illustrations, not an end-to-end native test.

- `main.png` and `main-light.png`: the main 400 × 560 tray popover, dark and light themes, captured at 2× pixel density. Recording state, queue counts, and recent activity use synthetic example data.
- `settings.png` and `queue.png`: the same app's settings and queue screens, reached through its actual buttons.
- `sync-review.png` and `sync-preview.png`: synthetic English conversations and the saved real Codex response from `scripts/evaluate-sync.mjs`, using the `work-sync-evaluation/english-ui` evaluation. The displayed root is a generic example path, `~/Documents/knowledge`.

The captures were checked for browser exceptions, page-width overflow, and unexpected Korean text. Source conversation content can still be in any language during ordinary use.

The real model/Git evaluations, native build checks, and their limits are recorded separately in [VALIDATION.md](../../VALIDATION.md).
