import {
  CircleAlert,
  CircleCheck,
  CircleDashed,
  FileCheck2,
  FileText,
  GitCompare,
  Inbox,
  LoaderCircle,
  Pause,
  Plug,
  RotateCw,
  type LucideIcon,
} from "lucide-react";
import { useRef, useState } from "react";

import { JobActions } from "@/components/JobActions";
import * as api from "@/ipc/api";
import type { Activity, ActivityKind, Job, Snapshot } from "@/ipc/contract";
import { elapsed, fileName, minutesUntil, providerLabel, relativeTime } from "@/format";
import { useAppStore, type QueueGroup } from "@/store/appStore";
import { ConfirmCard } from "@/ui/ConfirmCard";
import { IconChip, type Tint } from "@/ui/IconChip";
import { useNow } from "@/useNow";

const KINDS: Partial<Record<ActivityKind, { glyph: LucideIcon; tint: Tint; label: string; tone: string }>> = {
  recorded: { glyph: FileCheck2, tint: "green", label: "Recorded", tone: "ok" },
  no_change: { glyph: CircleDashed, tint: "neutral", label: "No new memory", tone: "" },
  retry_scheduled: { glyph: RotateCw, tint: "amber", label: "Retry scheduled", tone: "warn" },
  failed: { glyph: CircleAlert, tint: "red", label: "Failed", tone: "danger" },
};

function NowCard({ snapshot, now }: { snapshot: Snapshot; now: number }) {
  const togglePause = useAppStore((store) => store.togglePause);
  const { current, settings } = snapshot;
  const chosen = settings.models[settings.backend];
  const model = `${providerLabel(settings.backend)}${chosen ? ` ${chosen}` : ""}`;

  if (current) {
    return (
      <section className="now working" aria-label="Current activity">
        <IconChip glyph={LoaderCircle} tint="blue" spin />
        <div className="now-main">
          <div className="now-title">Recording {current.project}</div>
          <div className="now-sub">
            {providerLabel(current.provider)} session · {model} · {elapsed(current.startedAt, now)}
          </div>
          <div className="progress" aria-hidden="true">
            <span />
          </div>
        </div>
      </section>
    );
  }

  if (snapshot.syncing || snapshot.sync?.blocked) {
    return <section className="now paused" aria-label="Current activity"><IconChip glyph={GitCompare} tint="amber" /><div className="now-main"><div className="now-title">{snapshot.syncing ? "Syncing" : "Sync review needed"}</div><div className="now-sub">Recording is on hold. Pending changes and queued sessions are kept when you close the sync window.</div></div><button type="button" className="pill primary" onClick={() => void api.openSync()}>Sync</button></section>;
  }

  if (snapshot.reviewing) {
    return (
      <section className="now paused" aria-label="Current activity">
        <IconChip glyph={GitCompare} tint="amber" />
        <div className="now-main">
          <div className="now-title">Reviewing</div>
          <div className="now-sub">New recordings wait while the review window is open. Close it to resume.</div>
        </div>
        <button type="button" className="pill primary" onClick={() => void api.openReview()}>
          Open review
        </button>
      </section>
    );
  }

  if (settings.paused) {
    return (
      <section className="now paused" aria-label="Current activity">
        <IconChip glyph={Pause} tint="amber" />
        <div className="now-main">
          <div className="now-title">Recording paused</div>
          <div className="now-sub">Sessions still enter the queue. Resume to record them in order.</div>
        </div>
        <button type="button" className="pill primary" onClick={() => void togglePause()}>
          Resume
        </button>
      </section>
    );
  }

  const last = snapshot.recent.find((activity) => activity.kind === "recorded" || activity.kind === "no_change");
  return (
    <section className="now" aria-label="Current activity">
      <IconChip glyph={CircleCheck} tint="green" />
      <div className="now-main">
        <div className="now-title">Waiting for the next session</div>
        <div className="now-sub">
          {last ? `Last recorded ${relativeTime(last.at, now)} · ${last.project ?? providerLabel(last.provider)}` : "Memory is recorded when a Claude or Codex session ends."}
        </div>
      </div>
    </section>
  );
}

function HookCallout({ snapshot }: { snapshot: Snapshot }) {
  const showSettings = useAppStore((store) => store.showSettings);
  const missing = snapshot.hooks.filter((hook) => hook.state !== "connected");
  if (missing.length === 0) return null;
  const names = missing.map((hook) => providerLabel(hook.provider)).join("·");
  return (
    <section className="callout" aria-label="Hooks need connecting">
      <IconChip glyph={Plug} tint="amber" size={28} />
      <div className="callout-main">
        <div className="callout-title">Connect {names} hooks</div>
        <div className="callout-sub">Connect hooks to capture sessions when they end.</div>
      </div>
      <button type="button" className="pill primary" onClick={showSettings}>
        Connect
      </button>
    </section>
  );
}

function Stats({ snapshot }: { snapshot: Snapshot }) {
  const showQueue = useAppStore((store) => store.showQueue);
  const { counts, review } = snapshot;
  const items: { group: QueueGroup; label: string; value: number; tone: string; hint: string }[] = [
    { group: "ready", label: "Pending", value: counts.ready, tone: counts.ready === 0 ? "muted" : "", hint: "View sessions waiting to be recorded" },
    { group: "waiting", label: "Retrying", value: counts.waiting, tone: counts.waiting > 0 ? "warn" : "muted", hint: "View sessions waiting for a retry" },
    { group: "failed", label: "Failed", value: counts.failed, tone: counts.failed > 0 ? "danger" : "muted", hint: "View failed sessions" },
  ];
  const reviewValue = review === null ? "–" : String(review);
  return (
    <section className="stats" aria-label="Queue status">
      {items.map((item) => (
        <button key={item.label} type="button" className={`stat stat-button ${item.tone}`} aria-label={`${item.label} ${item.value}`} title={item.hint} onClick={() => showQueue(item.group)}>
          <div className="stat-value">{item.value}</div>
          <div className="stat-label">{item.label}</div>
        </button>
      ))}
      <button type="button" className={`stat stat-button ${review ? "" : "muted"}`} aria-label={`To review ${reviewValue}`} title="Review and commit changed notes" onClick={() => void api.openReview()}>
        <div className="stat-value">{reviewValue}</div>
        <div className="stat-label">To review</div>
      </button>
    </section>
  );
}

function AttentionRow({ job, now }: { job: Job; now: number }) {
  const failed = job.status === "failed";
  const status = failed ? "Failed" : job.status === "index_pending" ? "Index pending" : "Save interrupted";
  const name = job.project ?? providerLabel(job.provider);
  return (
    <div className="row">
      <IconChip glyph={failed ? CircleAlert : RotateCw} tint={failed ? "red" : "amber"} />
      <div className="row-main">
        <div className="row-title">
          <span className="name" title={name}>
            {name}
          </span>
          <span className="badge">{providerLabel(job.provider)}</span>
          <span className={`row-kind ${failed ? "danger" : "warn"}`}>{status}</span>
        </div>
        {job.lastError && (
          <div className="row-sub danger" title={job.lastError}>
            {job.lastError}
          </div>
        )}
        {!failed && job.nextAttemptAt > now && <div className="row-sub">Retry in about {minutesUntil(job.nextAttemptAt, now)} min.</div>}
      </div>
      {failed && <JobActions job={job} />}
    </div>
  );
}

function ActivityRow({ activity, now }: { activity: Activity; now: number }) {
  const kind = KINDS[activity.kind] ?? KINDS.no_change!;
  const detail = activity.error ?? activity.summary;
  const label = activity.kind === "recorded" ? `${activity.notes.length} note${activity.notes.length === 1 ? "" : "s"} updated` : kind.label;
  const name = activity.project ?? providerLabel(activity.provider);
  return (
    <div className="row">
      <IconChip glyph={kind.glyph} tint={kind.tint} />
      <div className="row-main">
        <div className="row-title">
          <span className="name" title={name}>
            {name}
          </span>
          <span className="badge">{providerLabel(activity.provider)}</span>
          <span className={`row-kind ${kind.tone}`}>{label}</span>
        </div>
        {detail && (
          <div className={activity.error ? "row-sub danger" : "row-sub"} title={detail}>
            {detail}
          </div>
        )}
        {activity.notes.length > 0 && (
          <div className="files">
            {activity.notes.map((note) => (
              <button key={note} type="button" className="file-chip" title={note} aria-label={`Open ${fileName(note)}`} onClick={() => void api.openNote(note)}>
                <FileText size={12} strokeWidth={1.9} />
                <span>{fileName(note)}</span>
              </button>
            ))}
          </div>
        )}
      </div>
      <span className="row-time">{relativeTime(activity.at, now)}</span>
    </div>
  );
}

export function StatusView({ snapshot }: { snapshot: Snapshot }) {
  const now = useNow(15_000);
  const clearActivity = useAppStore((store) => store.clearActivity);
  const [confirming, setConfirming] = useState(false);
  const clearButton = useRef<HTMLButtonElement>(null);
  const recent = snapshot.recent.filter((activity) => KINDS[activity.kind]);

  return (
    <div className="body">
      <NowCard snapshot={snapshot} now={now} />
      <HookCallout snapshot={snapshot} />
      <Stats snapshot={snapshot} />
      {!snapshot.syncing && !snapshot.sync?.blocked && <section className="card" aria-label="Memory sync"><div className="sync-toolbar"><span className="grow">{snapshot.sync?.phase === "push_pending" ? "Push pending" : "Share memory across PCs"}</span><button className="pill" onClick={() => void api.openSync()}>Sync</button></div></section>}

      {snapshot.attention.length > 0 && (
        <section aria-label="Needs attention">
          <div className="eyebrow">Needs attention</div>
          {snapshot.attention.map((job) => (
            <AttentionRow key={job.id} job={job} now={now} />
          ))}
        </section>
      )}

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
          recent.map((activity) => <ActivityRow key={activity.id} activity={activity} now={now} />)
        )}
      </section>
    </div>
  );
}
