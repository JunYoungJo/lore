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
   new part of the conversation the worker makes two model calls:
   - *extract*: from the turns and the marks, list candidates. Each has a kind (`decision`,
     `correction`, `lesson`), one to three self-contained sentences, and quotations of the turns
     it rests on. The program checks every quotation against the turn it names; one shorter
     than ten characters must be the whole turn, so that a few letters lifted from a long turn
     do not pass for evidence. A decision or a correction must quote the user; a lesson may
     quote the assistant and is then labelled as the agent's finding.
   - *relate*: for each candidate the program searches the notes (qmd, the candidate's text as
     the query) and shows the model the notes found. The model says whether a note already says
     this (`same`), says something this changes (`differs`), is about something this ends
     (`obsolete`), or none (`new`), and quotes the passages of that note. The program checks
     that the passages are in the note word for word.
   Candidates that are `same` are kept only as a count ("already in your notes"). The rest wait.
3. **Review.** The Candidates window lists what waits, grouped by session. The user ticks,
   edits the sentence, chooses project or all projects for new content, and approves or
   discards. The window shows the user's own words as evidence and, for `differs` and
   `obsolete`, the note's title and the quoted passages. It never shows a diff of a note.
   The candidates that wait when the window opens are ticked. One that arrives while the window
   is open is not: the approve button never takes in a sentence the user has not seen.
   Discarding several at once asks first; a discarded candidate does not come back.
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
   - an edit targets a note whose text the model was given, with the same edit operations and
     checks as a cleanup (`replace` text occurs once, and so on),
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
| `prepare` | extract and relate: requests, schemas, checks |
| `integrate` | apply: request, schema, checks, the notes to write |
| `worker` | One step: an apply batch if one is pending, else a queued session |
| `proposal` | Note text: front matter, sections, edit operations, source list (shared with cleanup) |


## Limits that remain

- A session is read from where the last step stopped. One step takes up to `maxChars`
  characters of new conversation (default 200,000) and up to 40,000 characters of what came
  before it, and a single turn is cut to 12,000 characters. These bound one model request; a
  longer stretch is prepared in further steps.
- The model calls have no tools. They receive text and return JSON.

## Beside this flow

Sync, cleanup and the search index work on the notes and do not know candidates. The Review
window lists the changes the user did not decide one by one: automatic approvals and cleanups.
Notes received from another PC were decided there and do not wait for review here. A sync
comparison of notes that two PCs both changed reads the notes alone; what was said, by whom
and when is in their Sources lines.
