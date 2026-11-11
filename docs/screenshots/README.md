# README screenshots

All screenshots show the implemented React UI in English, captured with headless Edge against the local Vite app on 2026-11-11. Tauri IPC, conversation excerpts, and model responses are synthetic fixtures for these captures; they are UI illustrations, not an end-to-end native test.

- `main.png` and `main-light.png`: the main 400 × 560 tray popover, dark and light themes, captured at 2× pixel density. Recording state, queue counts, and recent activity use synthetic example data.
- `settings.png`, `queue.png`, and `notes.png`: the same app's app-update/recording settings, session queue, and updated-note card, reached through its actual buttons.
- `integrations.png` and `search-index.png`: Manage Lore's connection resources and local search-index settings, at 1040 × 760 and 1.5× pixel density.
- `onboarding.png` and `onboarding-tools.png`: the first-run introduction and AI-tool choice, at 760 × 640 and 1.5× pixel density.
- `sync-review.png` and `sync-preview.png`: the decision sidebar, source quotations, and final side-by-side result, at 1120 × 800 and 1.5× pixel density. The sample decision concerns email sign-in versus passkeys for a fictional app. The displayed root is a generic example path, `~/Documents/lore-memory`.

The capture script checks browser exceptions, visible error banners, page-width overflow, and unexpected Korean text. It selects a resolution and opens the final check, but never applies or shares it. Source conversation content can still be in any language during ordinary use.

To reproduce, start `pnpm exec vite --host 127.0.0.1 --port 1462`, then run `node scripts/capture-readme.mjs` in another terminal. The script uses a temporary browser profile; set `LORE_SCREENSHOT_BROWSER` to a Chromium-based executable on another system, or `LORE_SCREENSHOT_URL` to use a different Vite URL. No authenticated CLI, model, qmd, or real Git repository is used by the fixtures.
