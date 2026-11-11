# Syncing memory across PCs

## First-time setup

On each PC, clone the same memory Git repository and set the app's memory repository path to that PC's checkout. The current branch must have a remote/upstream. For an existing repository, check with `git remote -v` and `git branch -vv`. The app does not create remotes or credentials. Authenticate first so fetch/push works from a terminal. Git 2.38 or later is required, and `user.name` and `user.email` must be set so commits can be created.

What is shared is **every file in the memory repository**. That includes not only memory notes but also the unquoted parts of the `sources/` session copies. List files you do not want to share in `.gitignore`. The queue, settings, model run data and the qmd index stay in per-PC app data. For stable source project IDs and the qmd MCP connection, follow [Using Lore on multiple PCs](multiplatform.md).

## Sync finishes without user action

When a recording finishes, the app commits the changed files right away. If **Auto-sync** is on in settings, it syncs in the order below right after all pending recordings finish and every 5 minutes. If it is off, the app only commits, and syncs in the same order when you click the sync button in the popover's bottom bar or **Sync now** in settings.

1. Commit any uncommitted changes.
2. Fetch commits from other PCs.
3. If both sides have new commits, merge. Changes to different notes, or to different places in the same note, are combined as is.
4. If two PCs changed the same place in the same note, merge **keeping both sides' lines**, and add that note to "items to check".
5. If notes from other PCs came in, update this PC's search index.
6. Push the result. If another PC pushed first in the meantime, fetch that commit, merge again, then push. No force push is used.

No step asks the user anything, and sync never stops recording. A sync left pending review by an earlier version is also handled in this order on the next sync. The review data from that time is kept in app data as `superseded-*.json`.

If one side deleted a note and the other edited it, the edited note is kept and added to the items to check. Renamed files are handled as a delete plus an add. Sync fails only when Git cannot produce a merge, such as histories with no common base; merge once with an external Git tool, then sync again.

## Items to check

Items to check are **notes that are already merged and shared** that are worth a look. Recording and sync continue while items remain. When there are items, the popover's bottom bar shows something like "2 to check", and they are also listed in **Needs attention**. Click to open the sync window (**Memory sync**).

- **Both PCs' lines kept:** A note where both PCs changed the same place and both sides' lines were kept. Shows this PC's note before the sync side by side with the merged note. If the same content was written twice or conflicting sentences remain next to each other, open the note with **Open note** and fix it. Your fix syncs like any other change. When you are done, remove it from the list with **Mark as checked**.
- **Edited note kept:** A note deleted on one side and edited on the other. If it should be deleted, delete the note yourself.
- **Notes disagree:** Conflicting content found by the model. Shows **evidence and quotes from both sides** (This PC / Other PCs), an explanation, and **options**. Choosing an option shows the current note and the note after applying it side by side; **Apply** edits the note and commits. For an option made of a single edit, you can change the wording with **Edit the selected wording**. To leave it alone, click **Keep the note as it is**.

Closing the window with **Later** or **Close** in the header keeps the items.

## Model comparison

After a sync, if there are **notes changed on both PCs**, the app compares only those notes with the recording model (Codex or Claude). Notes changed on only one side are not compared. The comparison reads only data saved at merge time, so recording and sync continue meanwhile. Model calls use your account's usage.

The model reads the changed parts first, then requests additional source text and surrounding conversation segment by segment as needed. The program checks that quotes actually appear in the provided material and that each passage to edit appears exactly once in the merged note; if not, it sends the reason back to the model and asks it to answer again. When an option is applied, it checks again that existing source links are kept. These checks do not prove whether a statement is true, its scope, or whether it is the latest decision. Contradictions the model misses can remain.

If a comparison cannot finish, the sync window shows the error with **Try again** and **Skip**. The notes are already synced, so skipping loses nothing. When the app restarts, an in-progress comparison resumes from its saved position.

If a note changed again after the comparison and an option's edit passage can no longer be found, applying it is refused. In that case, edit the note yourself and remove the item.

## Relationship to the review window

The review window is not a condition for sync. "Review N changed notes" in the popover is **the number of notes recorded on this PC since the last review**. Checking notes in the review window and clicking **Mark reviewed** removes them from the list. Notes are already committed and shared, so they sync whether or not you review them. Notes received from other PCs are not added to this PC's pending review. If another PC also changed a note that has unreviewed changes on this PC, that note stays pending review.

Review state is stored in the local ref `refs/lore/reviewed` in the memory repository and is not pushed.

## Failures and retries

- **Network or authentication failure:** Recording continues and sync is retried on the next cycle. After 3 consecutive failures, you are notified once and the bottom bar shows "Sync needs attention". Check the Git error in the sync window and click **Try again**. This PC's commits remain in place.
- **Search update failure:** The notes have already been received and the push still proceeds. The next sync retries only the index update. Check the qmd installation and path.
- **External Git operation in progress:** If the memory folder has an unfinished merge or rebase, the app does not sync. Finish it, then sync again.
- **While the review window is open:** The app does not sync. After you close the window, it syncs on the next cycle.

The app protects recording, review and sync with the same run lock. It does not lock Git tools run outside the app or other PCs.

## Storage and CLI

Per-PC app data stores the last sync time, failure count and items to check in `sync/<memory path hash>/state.json`. `merge-<ID>.json` holds the versions of notes changed on both PCs and their source material, and `analysis-<ID>/` holds the progress of the model comparison. They are deleted once the items and comparisons are all done. This data is not synced through Git.

Use the CLI after quitting the tray app.

```sh
lore-cli sync status               # Sync status and items to check
lore-cli sync now                  # Commit, fetch, merge, push
lore-cli sync compare              # Compare notes changed on both PCs with the model
lore-cli sync resolve <item ID> <option number>   # Apply an option and commit (numbers start at 0)
lore-cli sync dismiss <item ID>    # Remove an item from the list
```
