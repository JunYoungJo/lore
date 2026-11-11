import {
  ChevronRight,
  CircleAlert,
  CircleCheck,
  CircleDashed,
  FileCheck2,
  FileDiff,
  FileText,
  GitCompare,
  Inbox,
  LoaderCircle,
  Pause,
  Plug,
  RotateCw,
  Share2,
  type LucideIcon,
} from "lucide-react";
import { useRef, useState, type RefObject } from "react";

import * as api from "@/ipc/api";
import type { Activity, ActivityKind, Snapshot } from "@/ipc/contract";
import { elapsed, fileName, minutesUntil, providerLabel, relativePhrase, relativeTime } from "@/format";
import { unavailableModel } from "@/recordingModel";
import { useAppStore, type QueueGroup } from "@/store/appStore";
import { AnchoredCard } from "@/ui/AnchoredCard";
import { ConfirmCard } from "@/ui/ConfirmCard";
import { IconChip, type Tint } from "@/ui/IconChip";
import { useNow } from "@/useNow";

const KINDS: Partial<Record<ActivityKind, { glyph: LucideIcon; tint: Tint; label: string; tone: string }>> = {
  recorded: { glyph: FileCheck2, tint: "green", label: "Recorded", tone: "ok" },
  no_change: { glyph: CircleDashed, tint: "neutral", label: "No new memory", tone: "" },
  retry_scheduled: { glyph: RotateCw, tint: "amber", label: "Retry scheduled", tone: "warn" },
  failed: { glyph: CircleAlert, tint: "red", label: "Failed", tone: "danger" },
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
    title = `Recording ${current.project}`;
    sub = `${providerLabel(current.provider)} session · ${model} · ${elapsed(current.startedAt, now)}`;
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
    sub = "New recordings wait while the review window is open. Close it to resume.";
    action = { label: "Open review", run: () => void api.openReview() };
  } else if (settings.paused) {
    tone = "paused";
    glyph = Pause;
    tint = "amber";
    title = "Recording paused";
    sub = "Sessions still enter the queue. Resume to record them in order.";
    action = { label: "Resume", run: () => void togglePause() };
  } else {
    const last = snapshot.recent.find((activity) => activity.kind === "recorded" || activity.kind === "no_change");
    sub = last ? `Last memory ${relativePhrase(last.at, now)} · ${last.project ?? providerLabel(last.provider)}` : "Memory is recorded when a Claude or Codex session ends.";
  }

  return (
    <section className={`now ${tone}`} aria-label="Current activity">
      <div className="now-top">
        <IconChip glyph={glyph} tint={tint} spin={Boolean(current)} />
        <div className="now-main">
          <div className="now-title">{title}</div>
          <div className="now-sub">{sub}</div>
          {current && (
            <div className="progress" aria-hidden="true">
              <span />
            </div>
          )}
        </div>
        {action && (
          <button type="button" className="pill primary" onClick={action.run}>
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
  const list: Task[] = [];
  const missing = snapshot.hooks.filter((hook) => hook.state !== "connected");
  if (missing.length > 0) {
    list.push({
      key: "hooks",
      glyph: Plug,
      tint: "amber",
      title: `Connect ${missing.map((hook) => providerLabel(hook.provider)).join(" and ")} hooks`,
      detail: "Sessions are captured when they end once hooks are connected.",
      action: "Connect",
      run: () => void api.openIntegrations(),
    });
  }
  if (snapshot.review) {
    list.push({
      key: "review",
      glyph: FileDiff,
      tint: "blue",
      title: `Review ${plural(snapshot.review, "changed note")}`,
      detail: "Recorded since your last review. They are saved and shared already; review when you want.",
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
      title: `${plural(failed.length, "recording")} failed`,
      detail: [first.project ?? providerLabel(first.provider), first.lastError].filter(Boolean).join(" · "),
      action: "View",
      run: () => showQueue("failed"),
    });
  }
  const interrupted = snapshot.attention.filter((job) => job.status !== "failed");
  if (interrupted.length > 0) {
    const first = interrupted[0];
    const state = first.status === "index_pending" ? "Index pending" : "Save interrupted";
    const retry = first.nextAttemptAt > now ? `retry in about ${minutesUntil(first.nextAttemptAt, now)} min` : null;
    list.push({
      key: "interrupted",
      glyph: RotateCw,
      tint: "amber",
      title: `${plural(interrupted.length, "recording")} interrupted`,
      detail: [first.project ?? providerLabel(first.provider), state, retry].filter(Boolean).join(" · "),
      action: "View",
      run: () => showQueue("ready"),
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
  return (
    <AnchoredCard anchor={anchor} label={`Updated notes in ${name}`} role="dialog" className="notes-card" focus={first} onClose={onClose}>
      <div className="notes-card-head">
        {name} · {plural(activity.notes.length, "note")}
      </div>
      {activity.summary && (
        <p className="notes-card-summary" title={activity.summary}>
          {activity.summary}
        </p>
      )}
      {activity.notes.map((note, index) => (
        <button key={note} ref={index === 0 ? first : undefined} type="button" className="notes-card-row" title={note} aria-label={`Open ${fileName(note)}`} onClick={() => void api.openNote(note)}>
          <FileText size={14} strokeWidth={1.9} />
          <span>{fileName(note)}</span>
        </button>
      ))}
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

function ActivityRow({ activity, now, canReview }: { activity: Activity; now: number; canReview: boolean }) {
  const notesButton = useRef<HTMLButtonElement>(null);
  const [open, setOpen] = useState(false);
  const kind = KINDS[activity.kind] ?? KINDS.no_change!;
  const name = activity.project ?? providerLabel(activity.provider);
  const hasNotes = activity.notes.length > 0;
  const label = activity.kind === "recorded" ? `${plural(activity.notes.length, "note")} updated` : kind.label;
  const detail = hasNotes ? activity.notes.map(fileName).join(" · ") : activity.error ?? activity.summary ?? "";
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
            <button ref={notesButton} type="button" className={`row-kind ${kind.tone} notes-btn`} aria-haspopup="dialog" aria-expanded={open} onClick={() => setOpen(!open)}>
              {label}
            </button>
          ) : (
            <span className={`row-kind ${kind.tone}`}>{label}</span>
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

export function StatusView({ snapshot }: { snapshot: Snapshot }) {
  const now = useNow(15_000);
  const clearActivity = useAppStore((store) => store.clearActivity);
  const [confirming, setConfirming] = useState(false);
  const clearButton = useRef<HTMLButtonElement>(null);
  const recent = snapshot.recent.filter((activity) => KINDS[activity.kind]);
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
            <div className="empty-title">No sessions recorded yet</div>
            <div>Completed recordings will appear here.</div>
          </div>
        ) : (
          recent.map((activity) => <ActivityRow key={activity.id} activity={activity} now={now} canReview={canReview} />)
        )}
      </section>
    </div>
  );
}
