# Cleaning a folder by reading it

A cleanup tidies one folder of notes: it merges notes that repeat each other, settles notes that
disagree when the notes show which is current, removes notes whose subject is gone, and takes
apart notes built around a session. This document changes how the model gets the notes. What a
cleanup may change, and how the result is written, committed and undone, stays as it is.

## Why

Today the program puts the notes into the prompt. One model call carries at most 80,000
characters of notes (`REQUEST_CHARS`), and in a project cleanup the shared notes go with every
call as long as they take at most half of that. A folder with more notes is split in path order
over several calls, and each call sees the notes of the other calls, and shared notes that did
not fit, only as path and title (`note_catalog`). The model asks for the text of one of those by
naming it (`needs_notes`).

Notes keep accumulating, also between cleanups. Once a folder or the shared notes pass these
limits, a note in one call and a note that repeats or contradicts it in another call are found
only when the title gives it away. Recording and applying already let the model search and read
the notes itself. The user wants the cleanup to work the same way, so that it keeps finding
repeats and contradictions by their content however large the folders grow.

## Goals

- Every note of the folder being cleaned is read in full by the model during the run, and the
  program can tell from the run record that it was.
- Notes that belong together are found by their content (qmd search, grep), not by their title,
  whatever the size of the folder.
- The model reads the notes the way recording and applying do: `memory_folder`, `note_folders`
  and `search_collections` in the data, read-only access, the user's own CLI setup.

## Non-goals

- The comparison after a sync stays as it is. Its material (the base, local and remote versions
  of a note) exists only in Git, not as files the model could read.
- What a cleanup may change stays: the rules under "What to change" in the prompt, the edit
  operations, that Sources lines follow moved content (only how a line is named changes, see
  "Naming Sources lines"), the shared folder rules, one commit per folder, undo, the dry run of
  `lore-cli cleanup`.
- When a folder is due (`DUE_NOTES`) and when an automatic cleanup starts stay as they are.

## Anti-goals

- A run that reports a folder as cleaned while some of its notes were never read.
- Taking a list the model writes in its answer as proof of what it read.
- A note changed, removed or drawn from that the run record does not show the model read in full.

## Approaches

1. **Notes in the prompt up to the limits, search beyond them.** Keep today's limits and put the
   texts in the prompt while they fit; let the model search and read only what does not fit,
   instead of the title list. Behaves as today at today's sizes and uses qmd only beyond the
   limits. Two ways of getting notes to the model remain.
2. **No note text in the prompt; the model reads; the program checks the run record.** Each call
   is given the paths of the notes it is responsible for. The model reads them and the notes
   related to them itself. The program takes from the run record which notes were read in full,
   and the answer may touch only those. One way of reading notes across recording, applying and
   cleaning, independent of folder size. Every cleanup depends on the model's tools.
3. **Order notes by similarity before splitting.** The program would search qmd once per note to
   put similar notes into the same call, so the model could stay without tools. That is one qmd
   call of 2 to 4 seconds per note, or reading qmd's internal tables, which qmd 2.9.0 renamed;
   and related notes can still end up in different calls.

The user chose approach 2 on 2026-11-20, on the condition that what was read is taken from the
run record and not from the model's answer.

## The flow

1. The program loads the notes of the folder, of the folders it takes in (`absorb`) and, in a
   project cleanup, the shared notes, as today.
2. It splits the notes the run is responsible for into assignments, in path order, each at most
   `ASSIGNED_CHARS` characters (counted as today, with numbered Sources). The run is responsible
   for the notes of the folder and of the folders it takes in. In a project cleanup the shared
   notes are read and may be added to, but they are not assigned: the shared folder's own
   cleanup is responsible for them.
3. For each assignment it makes one model call in reading mode, with the memory folder as the
   folder the model may read (the same call recording and applying use). The data names the
   assigned paths; it holds no note text.
4. When the call returns, the program reads the run record the recorder wrote for that call
   (`events.jsonl` in the call's artifacts folder) and takes the set of notes read in full (see
   "What counts as read").
5. It checks the answer as today, with one change: the notes that can be edited, removed, moved
   or named in `sources_from` are the notes read in full in this call, instead of the notes
   whose text was in the prompt. An answer that touches another note is rejected and asked for
   once more with the reason, as any rejected answer is today.
6. The changes are applied as today: snapshot check, write.
7. Assigned notes that still exist and were not read in full are assigned once more, alone, in
   one follow-up call. Its answer is checked and applied the same way.
8. Notes still unread after the follow-up are collected, and the run goes on with the next
   assignment. At the end the run is incomplete: what was applied stays and is committed as
   today, and `incomplete` names the unread notes, for example
   `Not read by the model: common/a.md, common/b.md`. The cleanup window shows it as it shows any
   run that stopped early ("The last cleanup stopped early: ...").

A note assigned to a later call that an earlier call removed or rewrote is handled as today: the
notes are loaded again before each call, and a removed note drops out of its assignment.

## What counts as read

The recorder already keeps the run record of every call in its artifacts folder
(`recorder::notes_read` reads it). A note counts as read in full when the record holds a
successful call that returned all of the note as it is now:

| Model | Tool call in the record | Counts when |
|---|---|---|
| Claude (`stream-json`) | `Read` | `file_path` is a note under the memory folder, and there is no `offset` or `limit`. `Read` takes the file from disk. |
| Claude | `mcp__lore_qmd__get`, `mcp__lore_qmd__multi_get` | a note it returned (`[Resource from lore_qmd at qmd://<collection>/<path>]`) has the note's current text |
| Codex (`--json`) | `item.completed` of type `mcp_tool_call`, server `lore_qmd`, tool `get` or `multi_get` | a resource in its result has the note's current text |

- qmd returns the copy of a note stored in its index, not the file. The index is refreshed after
  a recording and after a cleanup, not between the calls of one cleanup, and it can lag behind
  when a refresh failed. So a note qmd returned counts only when the returned text, without the
  context line and the line numbers qmd adds, equals the note's text on disk. A copy that is
  older, cut short (`maxLines`, a line range) or not found does not count.
- A tool call that failed does not count (Claude: its `tool_result` has `is_error`; Codex: the
  item's `status` is not `completed`).
- Search results, grep output and shell commands do not count: they show parts of notes, and a
  shell command can read part of a file in ways the program cannot tell apart.
- A record that is missing or cannot be read counts as nothing read. The record is deleted
  before each call, so one left by an earlier call does not count.

## Naming Sources lines

The program used to show the model each note's Sources list numbered, and `sources_from` named
lines by those numbers. A note read from disk shows the lines as they are, and the tools number
the lines of the whole file instead. `sources_from` therefore names a Sources line by copying it,
with or without the `- ` that starts it. A line that is not in the note is rejected with the
reason, so a wrong reference is asked for again instead of moving the wrong quotation.

## What the model is given

The prompt keeps everything a cleanup is told today, with these changes:

- The `<notes>` block goes. The data comes before the instructions, as today, and holds
  `folder`, `absorbed_folders`, `shared_folder`, `today`, `previous_failure`, and:
  - `memory_folder`, `note_folders` (the folder, the folders taken in, and the shared folder) and
    `search_collections` (the collection of each of those folders), as in recording and applying,
  - `assigned`: the paths of the notes this call is responsible for.
- A section "Reading the notes", worded like the one applying uses:
  - Read every note in `assigned` in full with `Read` or the qmd `get` or `multi_get` tool,
    naming it by its path, without a line range.
  - For each, find the notes about its subject in the folders `note_folders` names, with the qmd
    `query` tool over `search_collections` first and grep for its names and identifiers, and read
    in full the ones that come up before you change anything.
  - You may change, remove, move or draw from only notes you read in full in this call.
  - Sources lines are named in `sources_from` by copying them.
- `note_catalog` and `needs_notes` go, from the prompt and from the answer schema.
- A follow-up call carries `assigned_again`.

## Size of a call

`ASSIGNED_CHARS` is 40,000 characters, half of today's `REQUEST_CHARS`, because the notes the
model reads to compare come on top of the assigned ones in the same call. Today every folder fits
in one call: the largest are the shared notes (51 notes, about 28,000 characters) and
`projects/lore` (20 notes, about 10,000 characters).

## What changes where

- `crates/core/src/cleanup-prompt.md`: as under "What the model is given".
- `crates/core/src/cleanup.rs`: assignments instead of parts with texts; the payload; the
  follow-up call; incomplete with the unread notes; `editable` from the run record; the schema
  without `needs_notes`.
- Reading the run record: a function that takes the record of one call, the memory folder and
  the search collections and returns the notes read in full, for both record formats. It belongs
  next to the recorder, which writes the record.
- `src-tauri/src/background.rs` and `crates/cli/src/main.rs`: the cleanup calls the model with
  `request_json_reading` and the memory folder, as recording and applying do, instead of
  `request_json`.
- `docs/design/candidates.md` "Beside this flow", if it describes the cleanup's input.

## Limits that remain

- The record shows that a note was returned to the model in full, not that the model weighed it.
- Codex can read notes only through qmd in a way that counts. When the qmd server stops during a
  Codex call, the notes it read through the shell do not count, and the run may end incomplete.
  The same holds for a note an earlier call of the same cleanup changed: until the index is
  refreshed, qmd returns its older copy, which does not count.
- qmd on this Windows PC (2.8.3) exits on the first search of a fresh server in 5 to 10 percent
  of calls and once stopped answering a search for 30 minutes. A cleanup now depends on qmd as
  recording does. Claude can still read with `Read` when qmd fails.
- A run that ended incomplete leaves the unread notes for the next cleanup of the folder. The
  next cleanup assigns every note of the folder again, so they are read then.
- A cleanup takes longer than today, since every note is read through a tool call.

## Decided by the user (2026-11-20)

- Approach 2: no note text in the prompt; the model reads the notes itself.
- What was read is taken from the run record, not from the model's answer.
- Assigned notes left unread get one follow-up call; notes still unread make the run incomplete,
  and the cleanup window names them.

## Calls made by the designer

- `ASSIGNED_CHARS` is 40,000 characters (the user agreed on 2026-11-20).
- After an assignment ends with unread notes, the run goes on with the next assignments and
  names all unread notes at the end, rather than stopping at the first.
- Only full reads of the current text count: `Read` without a range, and qmd `get` and
  `multi_get` results equal to the note on disk. Search results, grep and shell commands do not.
- In a project cleanup the shared notes are not assigned; the shared folder's own cleanup reads
  them.
- `sources_from` names Sources lines by their text instead of their number.
- A move counts as touching the note it moves, so the note must have been read in full too.
