# Recording through candidates

Lore does not write memory notes on its own judgment. A session and a model propose what to
remember, the user decides, and only what the user approved reaches the notes.

## Why

A model that reads a conversation afterwards can copy what the user said, but it cannot know
which of it will matter later, how far an instruction applies, or whether it is already in the
code. Those judgments differ per user and per project, so no fixed prompt gets them right for
everyone. The user can answer them at a glance for a session they just finished. The app
therefore asks the user the question they can answer ("keep this?") and never asks the one
they cannot ("is this edit to a long note correct?").

## The flow

1. **Mark.** While a session runs, a `UserPromptSubmit` hook adds one instruction to each
   prompt: when the user decides something, corrects how the agent works, or the work
   establishes a lesson, run `lore-cli mark <kind> "<one sentence>"`. The command writes nothing.
   It prints one line, `LORE-MARK {"kind":…,"text":…}`, which lands in the session file as a
   tool result. Marks are a hint. Everything below works without them. The hook runs on every
   prompt, so it reads no settings and opens no database: it parses the event and prints the
   instruction.
2. **Prepare.** `SessionEnd` and `PreCompact` put the session in the queue. For the
   new part of the conversation the worker makes one model call. Its prompt carries the turns
   (filtered and labelled by the program, see "What the model is given"), the marks, the
   candidates of the project that already wait, and where the memory folder is. The model reads
   and searches the notes of `common/` and the project's folder itself, and lists candidates:
   - each has a kind (`decision`, `correction`, `lesson`), one to three self-contained
     sentences, and quotations of the turns it rests on. The program checks every quotation
     against the turn it names; one shorter than ten characters must be the whole turn, so that
     a few letters lifted from a long turn do not pass for evidence. A decision or a correction
     must quote the user; a lesson may quote the assistant and is then labelled as the agent's
     finding.
   - each says whether a note already says this (`same`), says something this changes
     (`differs`), is about something this ends (`obsolete`), or none (`new`), and quotes the
     passages of that note. The program checks that the note is one of `common/` or the
     project's notes and that the passages are in it word for word.
   - nothing a waiting candidate already says is listed again.
   Candidates that are `same` are kept only as a count ("already in your notes"). The rest wait.
3. **Review.** The Candidates window lists what waits, grouped by session. The user goes
   through it one candidate at a time: rewords the sentence if needed, chooses project or all
   projects for new content, and decides approve, discard or later. Each decision moves on to
   the next undecided candidate, and keys do the same as the buttons. The window shows the
   user's own words as evidence and says first what approving does: adds to the notes, edits
   a note or deletes a note, with the note's title and the quoted passages for the last two.
   It never shows a diff of a note.
   Decisions stay in the window until the user applies them, and each can be changed until
   then; applying sends the approvals and the discards at once, and what is later or undecided
   keeps waiting. A candidate starts undecided. Approving the undecided ones together leaves
   out a candidate that arrived while the window was open until the user has looked at it, so
   nothing the user has not seen is approved. A discarded candidate does not come back.
4. **Apply.** Approved candidates of one project form an apply batch. One model call turns
   them into note changes: new notes, targeted edits of existing notes, removals. The program
   accepts the answer only if
   - every change and removal names the approved candidates it rests on, and every approved
     candidate is used by at least one change,
   - a statement goes only into notes of the folder its scope names: one about this project
     never enters a `common/` note, where every other project would read it, and one for all
     projects never ends up only in the project's folder,
   - a removal names a note that an approved `obsolete` candidate showed the user, and the note
     still reads as it did then (its digest is kept with the candidate). A note that was added
     to since is edited instead: the user has not seen what was added,
   - an edit targets an existing note of those folders, which the model reads itself, with the
     same edit operations and checks as a cleanup (`replace` text occurs once, and so on),
   - paths stay in `common/` and `projects/<project>/`.
   Notes the user was not shown may be edited so that they agree with an approved candidate,
   and the result lists them. They are never removed.
   The notes are written, committed and indexed, and the result (which candidate went into
   which note) is stored for the window. A note changed by a manual approval counts as
   reviewed, unless it already waited for review: what a cleanup or an automatic approval
   changed in it has not been looked at, so it keeps waiting.
5. **Unreviewed candidates wait.** With "Approve automatically" on, a candidate that waited the
   set number of days is approved as it stands. The days count from the later of two times:
   when the candidate entered the list, and when automatic approval was turned on. Turning the
   setting on therefore approves nothing at once, and a candidate that was put back in the list
   after a failed write gets its full time again.

The user is told of candidates when a session has ended and all of it is read: one
notification that counts every candidate of the session that still waits, whichever step found
it. A compaction offers candidates without a notification, since the session goes on.

## Interruptions and failures

A session job is only read. Its states are pending, done, failed and skipped; it never holds a
half-written note. A hook that arrives while a step of the same session is with the model is
counted, and the step leaves the job open when it finishes, so what was added to the session
file in the meantime is read by a further step.

Writing is the work of an apply batch, which goes through these states:

| State | Meaning | On failure |
|---|---|---|
| `pending` | Approved, no plan yet | Retried; gives up as `failed` |
| `applying` | The plan is stored and its notes are being written | Finished from the stored plan; when it gives up, what it wrote is taken back first |
| `index_pending` | Every note is written; candidates are `applied` and the result is shown | Only the commit and the index are left. Retried without end, never `failed` |
| `done` | Committed and indexed | |
| `failed` | Gave up | |

Two rules keep a batch from being written twice. A `failed` batch has left nothing in the
notes, so "Try again" plans from clean notes and "Put back in the list" returns the candidates
as they were. And a batch whose notes are written cannot fail any more, so nothing offers to
write it again. While a batch is `applying`, neither a sync nor a cleanup runs: both would
commit or rewrite notes that are half written. A plan checks only the notes it touches before
it writes, so a note that a sync brought in elsewhere does not invalidate it.

## What is stored where

- Candidates, apply batches and their results live in the app's database on this PC. They are
  never synced: a candidate is approved on the PC whose session produced it.
- A candidate the user has dealt with, and a written (`done`) batch, are kept for 30 days and
  then deleted; the window's Written tab lists the batches in that time, and its Clear deletes
  the written ones at once. A batch that is `pending`, `applying`, `index_pending` or `failed`
  is never deleted this way, since it still needs the program or the user. The notes and their
  commits are the lasting record of what was written.
- Only notes are in the memory folder, and only the memory folder is synced.
- A note's `Sources` section, which the program owns, gets one line per quotation of the
  candidates it was built from: `- 2026-11-17, user: "…"` or `- 2026-11-17, agent: "…"`. The
  evidence is in the note itself. No copy of a conversation is written into the memory folder,
  and nothing in the app reads one.
- New notes carry the project in their front matter. `kind` and `authority` are computed from
  the candidates (who was quoted), not chosen by the model.
- There is no code for data of an earlier version: no schema migration, no conversion of older
  notes. The app starts from an empty memory folder and a new database.

## Modules

| Module | Role |
|---|---|
| `mark` | The mark line, reading marks out of session files, the hook's instruction |
| `transcript` | Turns and marks of a session file |
| `session` | What a queued job has to offer: project, new turns, context, marks |
| `candidate` | Candidate, evidence, relation; their storage in the app database |
| `prepare` | The one call that lists candidates and their relations: request, schema, checks |
| `integrate` | apply: request, schema, checks, the notes to write |
| `worker` | One step: an apply batch if one is pending, else a queued session |
| `proposal` | Note text: front matter, sections, edit operations, source list (shared with cleanup) |


## Limits that remain

- A session is read from where the last step stopped. One step takes up to `maxChars`
  characters of new conversation (default 200,000) and up to 40,000 characters of what came
  before it, and a single turn is cut to 12,000 characters. These bound one model request; a
  longer stretch is prepared in further steps.
- A model call returns JSON and changes nothing; the program writes the notes.

## What the model is given

- **The session is in the prompt.** The program filters the session file (the user's and the
  assistant's words; no tool calls, tool output, reasoning or injected reminders), labels who
  said each turn, and puts the turns in the prompt. A model given a long file to read was seen
  to read the start and search the rest, which misses a decision that carries no keyword, and
  with Codex the program cannot tell what was read. In the prompt, every turn is read.
- **The notes are read by the model.** Prepare and Apply name the memory folder, the folders
  the project may change, and the qmd collections of those folders; the model searches and
  reads the notes it needs. No search by the program decides which notes the model sees. Both
  CLIs get one MCP server, the qmd server over this PC's index of the memory folder, defined as
  setup registers it for the CLIs (`lore_qmd`). qmd finds notes by meaning; the file tools find
  exact terms and read the notes. Claude runs with only its read and search tools
  (`--tools Read,Grep,Glob`), the qmd tools allowed, and the memory folder added
  (`--add-dir`); reading outside it is refused. Codex runs in its read-only sandbox from the
  memory folder (`-C`); it can read elsewhere but cannot write. What each run read and searched
  is kept in `events.jsonl` next to its prompt and answer.
- **The CLI runs with the user's setup, less what acts on its own.** Every model call (Prepare,
  Apply, cleanup and sync comparisons) reads the user's own configuration: login, model
  provider, the Codex `config.toml` with its sandbox and shell environment, and the user's
  global instructions (`CLAUDE.md`, `AGENTS.md`). What would act on its own, or adds tools and
  instructions meant for interactive work, is turned off, the same on every PC:
  - Claude: `--disable-slash-commands` (skills and commands), `disableAllHooks`,
    `--strict-mcp-config` with only the qmd server, `--no-session-persistence`.
  - Codex: hooks, `notify`, plugins, apps, bundled skills and the skill list, sub-agents,
    memories, computer and browser use, image generation, goals and web search, the AGENTS.md
    of the working folder, and every MCP server of the user's `config.toml` (each by name;
    `mcp_servers={}` does not clear them); `--ephemeral`.
  A run therefore needs nothing Lore copies from the user's setup, such as a Windows sandbox
  value, and works the way the user's CLI works on that PC. The recording process also gets
  `LORE_RECORDER=1`, so that Lore's own hook would ignore it.
- Cleanup and sync comparisons still get their notes in the prompt and have no tools.

## Beside this flow

Sync, cleanup and the search index work on the notes and do not know candidates. The Review
window lists the changes the user did not decide one by one: automatic approvals and cleanups.
Notes received from another PC were decided there and do not wait for review here. A sync
comparison of notes that two PCs both changed reads the notes alone; what was said, by whom
and when is in their Sources lines.
