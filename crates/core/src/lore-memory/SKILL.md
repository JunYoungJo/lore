---
name: lore-memory
description: Retrieve prior project decisions and lessons from Lore through the qmd MCP server when work depends on earlier context.
---

Use `qmd` to search the matching project collection and `common` before substantive project work. Read `.lore.json` for the project ID; use `status` when the collection is unclear. Set `collections` explicitly on every query.

Use exact-term searches for identifiers and semantic searches for decisions or lessons. Retrieve relevant document bodies with `get` or `multi_get`; snippets alone do not establish a decision. Check linked sources when approval or chronology is unclear. Distinguish user decisions from agent proposals, respect the recorded scope, and follow current user instructions.

Reuse context already retrieved for the current task. A failed lookup is not evidence that no prior record exists. If MCP is unavailable, report the limitation or use Lore's configured qmd CLI with the same local index and environment.

Lore records memories through session hooks. Do not edit memory notes or write session summaries unless the user requests it.
