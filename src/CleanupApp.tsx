import { useCallback, useEffect, useRef, useState } from "react";
import { ArrowRight, Eraser, FileText, FileX, LoaderCircle, TriangleAlert } from "lucide-react";

import * as api from "@/ipc/api";
import type { CleanupFolder, CleanupItem, CleanupLast, CleanupSnapshot } from "@/ipc/contract";
import { fileName, relativePhrase } from "@/format";
import { useTheme } from "@/theme";
import { ConfirmCard } from "@/ui/ConfirmCard";
import { useNow } from "@/useNow";

const COMMON = "common";

const RUN_STATE: Record<CleanupItem["state"], string> = { waiting: "Waiting", running: "Cleaning up…", done: "Done", failed: "Failed" };

const plural = (count: number, word: string) => `${count} ${word}${count === 1 ? "" : "s"}`;

const dueNames = (folders: CleanupFolder[]) => new Set(folders.filter((folder) => folder.due).map((folder) => folder.name));

/** How a folder stands since its last cleanup, in a few words for the list. */
function standing(folder: CleanupFolder): string {
  if (folder.failedAt !== null) return "the last cleanup failed";
  if (!folder.last && folder.checkedAt === null) return "never cleaned up";
  if (folder.fresh === 0) return "nothing changed since the last cleanup";
  return `${folder.fresh} changed since the last cleanup`;
}

/** A note of the cleaned folder by its file name; a note of another folder, such as a shared one the cleanup added, with its folder. */
function noteLabel(path: string, folder: string): string {
  const own = folder === COMMON ? `${COMMON}/` : `projects/${folder}/`;
  return path.startsWith(own) ? fileName(path) : path.replace(/^projects\//, "");
}

function FolderRow({ folder, item, viewing, checked, locked, onView, onCheck }: { folder: CleanupFolder; item: CleanupItem | undefined; viewing: boolean; checked: boolean; locked: boolean; onView: () => void; onCheck: (checked: boolean) => void }) {
  return (
    <li className={`review-item cleanup-row ${viewing ? "active" : ""}`} aria-label={folder.name}>
      <input type="checkbox" className="check" aria-label={`Clean up ${folder.name}`} checked={checked} disabled={locked} onChange={(event) => onCheck(event.target.checked)} />
      <button type="button" className="review-item-pick" aria-current={viewing} aria-label={folder.name} onClick={onView}>
        <div className="review-item-top">
          <span className="review-item-name" title={folder.name}>
            {folder.name}
          </span>
          {item && (
            <span className={`badge cleanup-${item.state}`}>
              {item.state === "running" && <LoaderCircle size={11} strokeWidth={2} className="spin" />}
              {RUN_STATE[item.state]}
            </span>
          )}
        </div>
        <div className={`cleanup-standing ${folder.failedAt !== null ? "danger" : folder.due ? "due" : ""}`}>
          {plural(folder.notes, "note")} · {standing(folder)}
        </div>
      </button>
    </li>
  );
}

function LastCleanup({ folder, last, now, busy, onUndo }: { folder: CleanupFolder; last: CleanupLast; now: number; busy: boolean; onUndo: () => void }) {
  const undo = useRef<HTMLButtonElement>(null);
  const [confirming, setConfirming] = useState(false);
  if (last.kind === "undone") return <p className="cleanup-line">The last cleanup was undone {relativePhrase(last.at, now)}. The notes are as they were before it.</p>;
  return (
    <>
      <div className="cleanup-last-head">
        <span className="cleanup-line strong">Cleaned up {relativePhrase(last.at, now)}</span>
        {last.commit && (
          <button ref={undo} type="button" className="pill ghost" aria-expanded={confirming} disabled={busy} onClick={() => setConfirming(!confirming)}>
            Undo
          </button>
        )}
      </div>
      {confirming && (
        <ConfirmCard
          anchor={undo}
          label="Undo cleanup confirmation"
          message={`Undo the cleanup of ${folder.name}? The notes it removed come back, and the notes it changed go back to what they said before.`}
          action="Undo cleanup"
          onConfirm={onUndo}
          onClose={() => setConfirming(false)}
        />
      )}
      {last.summary && <p className="cleanup-summary">{last.summary}</p>}
      {last.removed.length > 0 && (
        <section aria-label="Removed notes">
          <div className="cleanup-eyebrow">Removed · {last.removed.length}</div>
          {last.removed.map((gone) => (
            <div key={gone.path} className="cleanup-note-row gone" title={gone.path}>
              <FileX size={14} strokeWidth={1.9} />
              <div className="cleanup-note-main">
                <span className="cleanup-note-name">{noteLabel(gone.path, folder.name)}</span>
                {gone.reason && <span className="cleanup-note-why">{gone.reason}</span>}
              </div>
            </div>
          ))}
        </section>
      )}
      {last.moved.length > 0 && (
        <section aria-label="Moved notes">
          <div className="cleanup-eyebrow">Renamed or moved · {last.moved.length}</div>
          {last.moved.map((moved) => (
            <div key={moved.from} className="cleanup-note-row" title={`${moved.from} → ${moved.to}`}>
              <FileText size={14} strokeWidth={1.9} />
              <div className="cleanup-note-main moved">
                <span className="cleanup-note-name old">{noteLabel(moved.from, folder.name)}</span>
                <ArrowRight size={12} strokeWidth={2} />
                <span className="cleanup-note-name">{noteLabel(moved.to, folder.name)}</span>
              </div>
            </div>
          ))}
        </section>
      )}
      {last.changed.length > 0 && (
        <section aria-label="Changed notes">
          <div className="cleanup-eyebrow">Changed or added · {last.changed.length}</div>
          {last.changed.map((path) => (
            <button key={path} type="button" className="cleanup-note-row open" title={path} aria-label={`Open ${fileName(path)}`} onClick={() => void api.openNote(path)}>
              <FileText size={14} strokeWidth={1.9} />
              <span className="cleanup-note-name">{noteLabel(path, folder.name)}</span>
            </button>
          ))}
        </section>
      )}
    </>
  );
}

function Merge({ folder, others, busy, onMerge }: { folder: CleanupFolder; others: CleanupFolder[]; busy: boolean; onMerge: (into: string) => void }) {
  const [into, setInto] = useState("");
  useEffect(() => setInto(""), [folder.name]);
  return (
    <fieldset className="cleanup-merge" aria-label="Merge into another folder">
      <div className="cleanup-eyebrow">Merge into another folder</div>
      <p className="cleanup-hint">For a project that was renamed or whose notes ended up in two folders. Every note of {folder.name} is moved into the other folder or removed, and that folder is cleaned up.</p>
      <div className="cleanup-merge-row">
        <select className="input set-select" aria-label="Folder to merge into" value={into} onChange={(event) => setInto(event.target.value)}>
          <option value="">Choose a folder</option>
          {others.map((other) => (
            <option key={other.name} value={other.name}>
              {other.name}
            </option>
          ))}
        </select>
        <button type="button" className="pill ghost" disabled={busy || !into} onClick={() => onMerge(into)}>
          Merge and clean up
        </button>
      </div>
    </fieldset>
  );
}

/** The folders in the order they are worth looking at: those of the run, or those that have grown, first. */
function grouped(snapshot: CleanupSnapshot): { title: string; folders: CleanupFolder[] }[] {
  const { folders, run } = snapshot;
  const first = run
    ? run.items.map((item) => folders.find((folder) => folder.name === item.folder)).filter((folder): folder is CleanupFolder => folder !== undefined)
    : folders.filter((folder) => folder.due).sort((a, b) => b.fresh - a.fresh);
  const rest = folders.filter((folder) => !first.includes(folder));
  if (first.length === 0) return [{ title: "Folders", folders: rest }];
  return [{ title: run ? "This cleanup" : "Many changed notes", folders: first }, { title: "Other folders", folders: rest }].filter((group) => group.folders.length > 0);
}

function Detail({ snapshot, folder, now, busy, onUndo, onMerge }: { snapshot: CleanupSnapshot; folder: CleanupFolder; now: number; busy: boolean; onUndo: () => void; onMerge: (into: string) => void }) {
  // Merging is rare, so its form stays closed until it is asked for.
  const [merging, setMerging] = useState(false);
  const projects = snapshot.folders.filter((other) => other.name !== COMMON && other.name !== folder.name);
  return (
    <section className="cleanup-detail" aria-label={folder.name}>
      <div className="cleanup-detail-head">
        <div className="head-title">{folder.name}</div>
        <div className="head-sub">
          {plural(folder.notes, "note")} · {standing(folder)}
        </div>
      </div>
      {folder.failure && (
        <div className="banner" role="note">
          <TriangleAlert size={15} strokeWidth={1.9} style={{ flex: "none", marginTop: 1 }} />
          <span>
            {folder.failedAt !== null ? `The last cleanup failed ${relativePhrase(folder.failedAt, now)}: ` : "The last cleanup stopped early: "}
            {folder.failure}
          </span>
        </div>
      )}
      {folder.checkedAt !== null && <p className="cleanup-line">Looked at {relativePhrase(folder.checkedAt, now)}: nothing needed changing.</p>}
      {folder.last ? (
        <LastCleanup key={folder.name} folder={folder} last={folder.last} now={now} busy={busy} onUndo={onUndo} />
      ) : (
        folder.checkedAt === null && (
          <p className="cleanup-line">
            This folder has not been cleaned up yet. A cleanup reads the {plural(folder.notes, "note")} of this folder{folder.name === COMMON ? "" : " and the shared notes"}, and changes them without asking. It is saved as one commit, and you can
            undo it here.
          </p>
        )
      )}
      {folder.name !== COMMON &&
        projects.length > 0 &&
        (merging ? (
          <Merge folder={folder} others={projects} busy={busy} onMerge={onMerge} />
        ) : (
          <button type="button" className="cleanup-more" onClick={() => setMerging(true)}>
            Merge into another folder…
          </button>
        ))}
    </section>
  );
}

export function CleanupApp() {
  useTheme();
  const now = useNow(15_000);
  const [snapshot, setSnapshot] = useState<CleanupSnapshot | null>(null);
  const [viewing, setViewing] = useState<string | null>(null);
  const [checked, setChecked] = useState<ReadonlySet<string>>(new Set());
  const [notice, setNotice] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [undoing, setUndoing] = useState(false);
  // The folders that are due are checked when the window opens and again after each run.
  const settled = useRef(false);
  const loading = useRef(false);
  const again = useRef(false);

  const load = useCallback(async () => {
    if (loading.current) {
      again.current = true;
      return;
    }
    loading.current = true;
    try {
      const next = await api.getCleanup();
      setSnapshot(next);
      const names = new Set(next.folders.map((folder) => folder.name));
      // Shown first: the folder being cleaned, or else the one at the top of the list.
      const top = next.run?.items.find((item) => item.state === "running")?.folder ?? grouped(next)[0]?.folders[0]?.name ?? null;
      setViewing((current) => (current && names.has(current) ? current : top));
      if (next.run) {
        settled.current = false;
      } else if (!settled.current) {
        settled.current = true;
        setChecked(dueNames(next.folders));
      } else {
        setChecked((current) => new Set([...current].filter((name) => names.has(name))));
      }
    } catch (failure) {
      setError(String(failure));
    } finally {
      loading.current = false;
      if (again.current) {
        again.current = false;
        void load();
      }
    }
  }, []);

  useEffect(() => {
    void load();
    const unlisten = api.onStateChanged(() => void load());
    return () => void unlisten.then((stop) => stop());
  }, [load]);

  const act = async (action: () => Promise<unknown>, done?: string) => {
    setError(null);
    setNotice(null);
    try {
      await action();
      if (done) setNotice(done);
      await load();
    } catch (failure) {
      setError(String(failure));
    }
  };

  const folders = snapshot?.folders ?? [];
  const run = snapshot?.run ?? null;
  const busy = run !== null || undoing;
  const picked = folders.filter((folder) => checked.has(folder.name)).map((folder) => folder.name);
  const viewed = folders.find((folder) => folder.name === viewing);
  const itemOf = (name: string) => run?.items.find((item) => item.folder === name);
  const waiting = run?.items.some((item) => item.state === "waiting") ?? false;

  const check = (names: string[], on: boolean) =>
    setChecked((current) => {
      const next = new Set(current);
      names.forEach((name) => (on ? next.add(name) : next.delete(name)));
      return next;
    });

  const undo = async (name: string) => {
    setUndoing(true);
    setError(null);
    setNotice(null);
    try {
      const stale = await api.undoCleanup(name);
      setNotice(stale ?? `The cleanup of ${name} was undone.`);
      await load();
    } catch (failure) {
      setError(String(failure));
    } finally {
      setUndoing(false);
    }
  };

  const status = run
    ? (snapshot?.blocked ?? (waiting ? "Stop leaves the folders that still wait as they are. The folder being cleaned is finished first." : "Finishing the last folder."))
    : (notice ??
      (snapshot?.auto
        ? `Automatic cleanup is on: a folder is cleaned up once ${snapshot.dueNotes} or more of its notes have changed.`
        : "Each cleanup is applied at once and saved as one commit per folder. The changed notes then wait in the review window."));

  return (
    <main className="review cleanup-window">
      <header className="review-head">
        <div className="head-text">
          <div className="head-title">Clean up notes</div>
          <div className="head-sub">The recording model reads a folder, merges notes that repeat each other and removes the ones that are out of date.</div>
        </div>
        {run ? (
          <button type="button" className="btn ghost" disabled={!waiting} onClick={() => void act(() => api.stopCleanup())}>
            Stop
          </button>
        ) : (
          snapshot && (
            <button type="button" className="btn primary" disabled={undoing || picked.length === 0} onClick={() => void act(() => api.startCleanup(picked.map((name) => ({ folder: name, absorb: [] }))))}>
              <Eraser size={15} strokeWidth={1.9} />
              {picked.length === 0 ? "Clean up" : `Clean up ${plural(picked.length, "folder")}`}
            </button>
          )
        )}
      </header>

      {error && (
        <div className="banner" role="alert">
          <TriangleAlert size={15} strokeWidth={1.9} style={{ flex: "none", marginTop: 1 }} />
          <span>{error}</span>
        </div>
      )}

      {snapshot === null ? (
        <div className="review-center">Loading folders…</div>
      ) : folders.length === 0 ? (
        <div className="review-center">No notes have been recorded yet.</div>
      ) : (
        <>
          <div className="review-body">
            <aside className="review-side">
              <label className="review-all">
                <input type="checkbox" className="check" aria-label="Select all" checked={picked.length === folders.length} disabled={busy} onChange={(event) => check(folders.map((folder) => folder.name), event.target.checked)} />
                Select all
              </label>
              <ul className="review-groups" aria-label="Folders of notes">
                {grouped(snapshot).map((group) => (
                  <li key={group.title} className="review-group">
                    <div className="review-group-head">
                      <span className="review-group-name">{group.title}</span>
                      <span className="review-group-count">{group.folders.length}</span>
                    </div>
                    <ul className="review-groups cleanup-list" aria-label={group.title}>
                      {group.folders.map((folder) => (
                        <FolderRow
                          key={folder.name}
                          folder={folder}
                          item={itemOf(folder.name)}
                          viewing={folder.name === viewing}
                          checked={checked.has(folder.name)}
                          locked={busy}
                          onView={() => setViewing(folder.name)}
                          onCheck={(on) => check([folder.name], on)}
                        />
                      ))}
                    </ul>
                  </li>
                ))}
              </ul>
            </aside>
            {viewed ? (
              <Detail key={viewed.name} snapshot={snapshot} folder={viewed} now={now} busy={busy} onUndo={() => void undo(viewed.name)} onMerge={(into) => void act(() => api.startCleanup([{ folder: into, absorb: [viewed.name] }]))} />
            ) : (
              <div className="review-note review-loading" />
            )}
          </div>
          <footer className="review-foot">
            <div className="review-actions">
              <span className={`grow ${notice && !run ? "" : "plain"}`}>{status}</span>
              <button type="button" className="btn ghost" onClick={() => void api.openReview()}>
                Open review
              </button>
            </div>
          </footer>
        </>
      )}
    </main>
  );
}
