import {
  ChevronRight,
  CircleAlert,
  CircleCheck,
  CircleDashed,
  Eraser,
  FileCheck2,
  FileDiff,
  FileText,
  FileX,
  GitCompare,
  Inbox,
  ListChecks,
  LoaderCircle,
  Pause,
  Plug,
  RotateCw,
  Share2,
  Undo2,
  type LucideIcon,
} from "lucide-react";
import { useRef, useState, type RefObject } from "react";

import * as api from "@/ipc/api";
import type { Activity, ActivityKind, Snapshot } from "@/ipc/contract";
import { elapsed, fileName, goneNotes, providerLabel, relativePhrase, relativeTime, summaryLine } from "@/format";
import { unavailableModel } from "@/recordingModel";
import { useAppStore, type QueueGroup } from "@/store/appStore";
import { AnchoredCard } from "@/ui/AnchoredCard";
import { ConfirmCard } from "@/ui/ConfirmCard";
import { IconChip, type Tint } from "@/ui/IconChip";
import { useNow } from "@/useNow";

const KINDS: Partial<Record<ActivityKind, { glyph: LucideIcon; tint: Tint; label: string; tone: string }>> = {
  offered: { glyph: ListChecks, tint: "blue", label: "Candidates", tone: "" },
  recorded: { glyph: FileCheck2, tint: "green", label: "Recorded", tone: "ok" },
  no_change: { glyph: CircleDashed, tint: "neutral", label: "Nothing to remember", tone: "" },
  retry_scheduled: { glyph: RotateCw, tint: "amber", label: "Retry scheduled", tone: "warn" },
  failed: { glyph: CircleAlert, tint: "red", label: "Failed", tone: "danger" },
  cleaned: { glyph: Eraser, tint: "green", label: "Nothing to clean up", tone: "" },
  cleanup_failed: { glyph: CircleAlert, tint: "red", label: "Cleanup failed", tone: "danger" },
  cleanup_undone: { glyph: Undo2, tint: "neutral", label: "Cleanup undone", tone: "" },
};

const plural = (count: number, one: string, many = `${one}s`) => `${count} ${count === 1 ? one : many}`;

function QueueLink({ snapshot }: { snapshot: Snapshot }) {
  const showQueue = useAppStore((store) => store.showQueue);
  const { ready, waiting } = snapshot.counts;
  if (ready + waiting === 0) return null;
  const parts = [ready > 0 && `${plural(ready, "session")} waiting`, waiting > 0 && `${waiting} retrying`].filter(Boolean);
  const focus: QueueGroup = ready > 0 ? "ready" : "waiting";
  return (
    <button type="button" className="now-queue" onClick={() => showQueue(focus)}>
      <span className="grow">{parts.join(" · ")}</span>
      <ChevronRight size={14} strokeWidth={1.9} aria-hidden="true" />
    </button>
  );
}

function NowCard({ snapshot, now }: { snapshot: Snapshot; now: number }) {
  const togglePause = useAppStore((store) => store.togglePause);
  const { current, settings } = snapshot;
  // A run that still waits for its turn shows in the cleanup window, where it was asked for.
  const items = snapshot.cleanup?.items ?? [];
  const position = items.findIndex((item) => item.state === "running");
  const cleaning = position >= 0 ? { folder: items[position].folder, startedAt: items[position].startedAt ?? now, position: position + 1, total: items.length } : null;
  const chosen = settings.models[settings.backend];
  const model = `${providerLabel(settings.backend)}${chosen ? ` ${chosen}` : ""}`;

  let tone = "";
  let glyph: LucideIcon = CircleCheck;
  let tint: Tint = "green";
  let title = "Waiting for the next session";
  let sub: string;
  let action: { label: string; run: () => void } | null = null;

  if (current) {
    tone = "working";
    glyph = LoaderCircle;
    tint = "blue";
    title = current.applying ? `Writing notes for ${current.project}` : `Reading the ${current.project} session`;
    sub = current.applying ? `Approved candidates · ${model} · ${elapsed(current.startedAt, now)}` : `${providerLabel(current.provider)} session · ${model} · ${elapsed(current.startedAt, now)}`;
  } else if (cleaning) {
    tone = "working";
    glyph = LoaderCircle;
    tint = "blue";
    title = `Cleaning up ${cleaning.folder}`;
    sub = [cleaning.total > 1 && `${cleaning.position} of ${cleaning.total} folders`, model, elapsed(cleaning.startedAt, now)].filter(Boolean).join(" · ");
    action = { label: "View", run: () => void api.openCleanup() };
  } else if (snapshot.syncing) {
    tone = "paused";
    glyph = GitCompare;
    tint = "amber";
    title = "Syncing";
    sub = "Sharing notes with your other PCs. This takes a few seconds.";
  } else if (snapshot.reviewing) {
    tone = "paused";
    glyph = GitCompare;
    tint = "amber";
    title = "Reviewing";
    sub = "Sessions and approved candidates wait while the review window is open. Close it to resume.";
    action = { label: "Open review", run: () => void api.openReview() };
  } else if (settings.paused) {
    tone = "paused";
    glyph = Pause;
    tint = "amber";
    title = "Recording paused";
    sub = "Sessions still enter the queue. Resume to read them in order.";
    action = { label: "Resume", run: () => void togglePause() };
  } else {
    const last = snapshot.recent.find((activity) => activity.kind === "offered" || activity.kind === "recorded" || activity.kind === "no_change");
    sub = last ? `Last session read ${relativePhrase(last.at, now)} · ${last.project ?? providerLabel(last.provider)}` : "When a Claude or Codex session ends, Lore lists what it may be worth remembering.";
  }

  return (
    <section className={`now ${tone}`} aria-label="Current activity">
      <div className="now-top">
        <IconChip glyph={glyph} tint={tint} spin={Boolean(current || cleaning)} />
        <div className="now-main">
          <div className="now-title">{title}</div>
          <div className="now-sub">{sub}</div>
          {(current || cleaning) && (
            <div className="progress" aria-hidden="true">
              <span />
            </div>
          )}
        </div>
        {action && (
          <button type="button" className={`pill ${cleaning ? "ghost" : "primary"}`} onClick={action.run}>
            {action.label}
          </button>
        )}
      </div>
      <QueueLink snapshot={snapshot} />
    </section>
  );
}

interface Task {
  key: string;
  glyph: LucideIcon;
  tint: Tint;
  title: string;
  detail: string;
  action: string;
  run: () => void;
}

function tasks(snapshot: Snapshot, now: number, showQueue: (focus: QueueGroup) => void, syncNow: () => void, showSettings: () => void): Task[] {
  // A session is only read, so it is either waiting, read or failed. A batch that was
  // interrupted while writing is finished by itself and shown in the candidates window.
  const list: Task[] = [];
  const missing = snapshot.hooks.filter((hook) => hook.state !== "connected");
  if (missing.length > 0) {
    list.push({
      key: "hooks",
      glyph: Plug,
      tint: "amber",
      title: `Connect ${missing.map((hook) => providerLabel(hook.provider)).join(" and ")} hooks`,
      detail: "Sessions are read when they end once hooks are connected.",
      action: "Connect",
      run: () => void api.openIntegrations(),
    });
  }
  const { candidates } = snapshot;
  if (candidates.candidates > 0) {
    const auto = snapshot.settings.autoApprove ? `approved automatically after ${plural(snapshot.settings.autoApproveDays, "day")}` : null;
    list.push({
      key: "candidates",
      glyph: ListChecks,
      tint: "blue",
      title: `${plural(candidates.candidates, "candidate")} to review`,
      detail: [`From ${plural(candidates.sessions, "session")}`, `oldest ${relativePhrase(candidates.oldestAt, now)}`, auto].filter(Boolean).join(" · "),
      action: "Review",
      run: () => void api.openCandidates(),
    });
  }
  if (snapshot.failedApplies > 0) {
    list.push({
      key: "apply-failed",
      glyph: CircleAlert,
      tint: "red",
      title: `${plural(snapshot.failedApplies, "approved batch", "approved batches")} not written`,
      detail: "The notes are unchanged. Open the candidates to try again or to take the approval back.",
      action: "Open",
      run: () => void api.openCandidates(),
    });
  }
  if (snapshot.review) {
    list.push({
      key: "review",
      glyph: FileDiff,
      tint: "blue",
      title: `Review ${plural(snapshot.review, "changed note")}`,
      detail: "Changed since your last review by an automatic approval or a cleanup. They are saved and shared already.",
      action: "Review",
      run: () => void api.openReview(),
    });
  }
  const failed = snapshot.attention.filter((job) => job.status === "failed");
  if (failed.length > 0) {
    const first = failed[0];
    list.push({
      key: "failed",
      glyph: CircleAlert,
      tint: "red",
      title: `${plural(failed.length, "session")} could not be read`,
      detail: [first.project ?? providerLabel(first.provider), first.lastError].filter(Boolean).join(" · "),
      action: "View",
      run: () => showQueue("failed"),
    });
  }
  const model = unavailableModel(snapshot);
  if (model) {
    list.push({
      key: "model",
      glyph: CircleAlert,
      tint: "amber",
      title: "Recording model not available",
      detail: `${model} is not in ${providerLabel(snapshot.settings.backend)}’s model list for this account.`,
      action: "Settings",
      run: showSettings,
    });
  }
  if (snapshot.sync?.phase === "failing" && !snapshot.syncing) {
    list.push({
      key: "share",
      glyph: Share2,
      tint: "amber",
      title: "Memory not shared yet",
      detail: snapshot.sync.message ?? "Recorded notes are waiting to reach your other PCs. Lore keeps retrying.",
      action: "Sync",
      run: syncNow,
    });
  }
  if (snapshot.sync?.items) {
    list.push({
      key: "sync-items",
      glyph: GitCompare,
      tint: "blue",
      title: snapshot.sync.items === 1 ? "1 synced note to check" : `${snapshot.sync.items} synced notes to check`,
      detail: "Both PCs changed these notes. They are already merged and shared; look at them when you have time.",
      action: "Open",
      run: () => void api.openSync(),
    });
  }
  // With automatic cleanup on, the due folders are cleaned without anyone asking.
  const due = [...(snapshot.cleanupDue ?? [])].sort((a, b) => b.fresh - a.fresh);
  if (due.length > 0 && !snapshot.cleanup && !snapshot.settings.autoCleanup) {
    // The line is short, so it says why first and names the folders with the most changed notes.
    const named = due.slice(0, 2).map((folder) => folder.name);
    list.push({
      key: "cleanup",
      glyph: Eraser,
      tint: "blue",
      title: `${plural(due.length, "folder")} to clean up`,
      detail: `Many notes changed in ${due.length > 2 ? `${named.join(", ")} and ${due.length - 2} more` : named.join(" and ")}`,
      action: "Open",
      run: () => void api.openCleanup(),
    });
  }
  return list;
}

function NeedsAttention({ snapshot, now }: { snapshot: Snapshot; now: number }) {
  const showQueue = useAppStore((store) => store.showQueue);
  const syncNow = useAppStore((store) => store.syncNow);
  const showSettings = useAppStore((store) => store.showSettings);
  const list = tasks(snapshot, now, showQueue, () => void syncNow(), showSettings);
  if (list.length === 0) return null;
  return (
    <section aria-label="Needs attention">
      <div className="eyebrow">
        <span>
          Needs attention<span className="eyebrow-badge">{list.length}</span>
        </span>
      </div>
      <div className="todo">
        {list.map((task, index) => (
          <div key={task.key} className="todo-row">
            <IconChip glyph={task.glyph} tint={task.tint} />
            <div className="todo-main">
              <div className="todo-title">{task.title}</div>
              <div className="todo-sub" title={task.detail}>
                {task.detail}
              </div>
            </div>
            <button type="button" className={`pill ${index === 0 ? "primary" : "ghost"}`} onClick={task.run}>
              {task.action}
            </button>
          </div>
        ))}
      </div>
    </section>
  );
}

function NotesCard({ anchor, activity, name, canReview, onClose }: { anchor: RefObject<HTMLButtonElement>; activity: Activity; name: string; canReview: boolean; onClose: () => void }) {
  const first = useRef<HTMLButtonElement>(null);
  const gone = goneNotes(activity.summary);
  const firstKept = activity.notes.find((note) => !gone.has(note));
  const summary = summaryLine(activity.summary);
  const batch = /^apply-(\d+)$/.exec(activity.jobId);
  return (
    <AnchoredCard anchor={anchor} label={`Updated notes in ${name}`} role="dialog" className="notes-card" focus={first} onClose={onClose}>
      <div className="notes-card-head">
        {name} · {plural(activity.notes.length, "note")}
      </div>
      {summary && (
        <p className="notes-card-summary" title={summary}>
          {summary}
        </p>
      )}
      {activity.error && <p className="notes-card-summary danger">Stopped early: {activity.error}</p>}
      {activity.notes.map((note) =>
        gone.has(note) ? (
          <div key={note} className="notes-card-row gone" title={gone.get(note)}>
            <FileX size={14} strokeWidth={1.9} />
            <span>{fileName(note)}</span>
          </div>
        ) : (
          <button key={note} ref={note === firstKept ? first : undefined} type="button" className="notes-card-row" title={note} aria-label={`Open ${fileName(note)}`} onClick={() => void api.openNote(note)}>
            <FileText size={14} strokeWidth={1.9} />
            <span>{fileName(note)}</span>
          </button>
        ),
      )}
      {batch && (
        <button
          type="button"
          className="notes-card-foot"
          onClick={() => {
            onClose();
            void api.openCandidates(Number(batch[1]));
          }}
        >
          Show in Candidates
        </button>
      )}
      {canReview && (
        <button
          type="button"
          className="notes-card-foot"
          onClick={() => {
            onClose();
            void api.openReview();
          }}
        >
          Review changes
        </button>
      )}
    </AnchoredCard>
  );
}

function ActivityRow({ activity, now, canReview, waiting }: { activity: Activity; now: number; canReview: boolean; waiting: number }) {
  const notesButton = useRef<HTMLButtonElement>(null);
  const [open, setOpen] = useState(false);
  const kind = KINDS[activity.kind] ?? KINDS.no_change!;
  const name = activity.project ?? providerLabel(activity.provider);
  const hasNotes = activity.notes.length > 0;
  const offered = activity.kind === "offered";
  const label = activity.kind === "recorded" ? `${plural(activity.notes.length, "note")} updated` : kind.label;
  const said = activity.error ?? summaryLine(activity.summary);
  const detail = hasNotes ? activity.notes.map(fileName).join(" · ") : offered ? `${said} · ${waiting > 0 ? `${waiting} waiting` : "all decided"}` : said;
  const kindTone = kind.tone;
  const tone = activity.error ? "danger" : hasNotes ? "" : "muted";
  return (
    <div className="row activity">
      <IconChip glyph={kind.glyph} tint={kind.tint} />
      <div className="row-main">
        <div className="row-title">
          <span className="name" title={name}>
            {name}
          </span>
          <span className="badge">{providerLabel(activity.provider)}</span>
          {hasNotes ? (
            <button ref={notesButton} type="button" className={`row-kind ${kindTone} notes-btn`} aria-haspopup="dialog" aria-expanded={open} onClick={() => setOpen(!open)}>
              {label}
            </button>
          ) : offered && waiting > 0 ? (
            <button type="button" className="row-kind notes-btn" title="Open the candidates" onClick={() => void api.openCandidates()}>
              {label}
            </button>
          ) : (
            <span className={`row-kind ${kindTone}`}>{label}</span>
          )}
        </div>
        <div className={`row-line ${tone}`} title={hasNotes ? activity.notes.join("\n") : detail || undefined}>
          {detail}
        </div>
      </div>
      <span className="row-time">{relativeTime(activity.at, now)}</span>
      {open && <NotesCard anchor={notesButton} activity={activity} name={name} canReview={canReview} onClose={() => setOpen(false)} />}
    </div>
  );
}

const isCleanup = (activity: Activity) => activity.kind === "cleaned" || activity.kind === "cleanup_failed";

/** The recent activity as rows: a cleanup run is one row however many folders it cleaned. */
function activityRows(recent: Activity[]): Activity[][] {
  const rows: Activity[][] = [];
  const runs = new Map<string, Activity[]>();
  for (const activity of recent) {
    // The folders of one run share an id; an entry written before runs existed stands alone.
    const run = isCleanup(activity) && /^cleanup-\d+$/.test(activity.jobId) ? activity.jobId : null;
    const row = run ? runs.get(run) : undefined;
    if (row) {
      row.push(activity);
      continue;
    }
    const started = [activity];
    if (run) runs.set(run, started);
    rows.push(started);
  }
  return rows;
}

function CleanupRow({ folders, now }: { folders: Activity[]; now: number }) {
  const failed = folders.filter((folder) => folder.kind === "cleanup_failed").length;
  const removed = folders.reduce((sum, folder) => sum + folder.notes.filter((note) => goneNotes(folder.summary).has(note)).length, 0);
  const changed = folders.reduce((sum, folder) => sum + folder.notes.length, 0) - removed;
  const only = folders.length === 1 ? folders[0] : null;
  const name = only ? (only.project ?? "Cleanup") : plural(folders.length, "folder");
  const allFailed = failed === folders.length;
  const label = allFailed ? "Cleanup failed" : changed + removed === 0 ? "Nothing to clean up" : `${changed} changed · ${removed} removed`;
  const names = folders.map((folder) => folder.project).filter(Boolean).reverse().join(" · ");
  const stopped = only?.kind === "cleaned" && only.error ? `Stopped early: ${only.error}` : null;
  // The line is cut when it is long, so a failure among the folders comes before their names.
  const detail = only ? (stopped ?? only.error ?? summaryLine(only.summary)) : `${failed > 0 && !allFailed ? `${failed} failed · ` : ""}${names}`;
  const danger = allFailed || Boolean(stopped);
  return (
    <div className="row activity">
      <IconChip glyph={allFailed ? CircleAlert : Eraser} tint={allFailed ? "red" : "green"} />
      <div className="row-main">
        <div className="row-title">
          <span className="name" title={name}>
            {name}
          </span>
          <span className="badge">{providerLabel(folders[0].provider)}</span>
          <button type="button" className={`row-kind ${allFailed ? "danger" : changed + removed > 0 ? "ok" : ""} notes-btn`} title="Open the cleanup window" onClick={() => void api.openCleanup()}>
            {label}
          </button>
        </div>
        <div className={`row-line ${danger ? "danger" : changed + removed > 0 ? "" : "muted"}`} title={detail || undefined}>
          {detail}
        </div>
      </div>
      <span className="row-time">{relativeTime(folders[0].at, now)}</span>
    </div>
  );
}

export function StatusView({ snapshot }: { snapshot: Snapshot }) {
  const now = useNow(15_000);
  const clearActivity = useAppStore((store) => store.clearActivity);
  const [confirming, setConfirming] = useState(false);
  const clearButton = useRef<HTMLButtonElement>(null);
  const recent = activityRows(snapshot.recent.filter((activity) => KINDS[activity.kind]));
  const canReview = Boolean(snapshot.review);

  return (
    <div className="body">
      <NowCard snapshot={snapshot} now={now} />
      <NeedsAttention snapshot={snapshot} now={now} />

      <section aria-label="Recent activity">
        <div className="eyebrow">
          Recent activity
          {snapshot.recent.length > 0 && (
            <button ref={clearButton} type="button" className="eyebrow-btn" aria-label="Clear history" aria-expanded={confirming} onClick={() => setConfirming(!confirming)}>
              Clear
            </button>
          )}
        </div>
        {confirming && snapshot.recent.length > 0 && (
          <ConfirmCard
            anchor={clearButton}
            label="Clear history confirmation"
            message="Clear recent activity? Memory notes and queued sessions will be kept."
            action="Clear all"
            onConfirm={() => void clearActivity()}
            onClose={() => setConfirming(false)}
          />
        )}
        {recent.length === 0 ? (
          <div className="empty">
            <div className="empty-icon">
              <Inbox size={20} strokeWidth={1.75} />
            </div>
            <div className="empty-title">No sessions read yet</div>
            <div>Candidates and the notes written from them will appear here.</div>
          </div>
        ) : (
          recent.map((row) => (isCleanup(row[0]) ? <CleanupRow key={row[0].id} folders={row} now={now} /> : <ActivityRow key={row[0].id} activity={row[0]} now={now} canReview={canReview} waiting={snapshot.candidates.byJob[row[0].jobId] ?? 0} />))
        )}
      </section>
    </div>
  );
}
