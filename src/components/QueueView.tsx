import { CircleAlert, Clock, LoaderCircle, RotateCw, type LucideIcon } from "lucide-react";
import { useEffect, useRef } from "react";

import { JobActions } from "@/components/JobActions";
import type { Current, Job, Snapshot } from "@/ipc/contract";
import { elapsed, fileName, minutesUntil, providerLabel, relativeTime } from "@/format";
import { useAppStore, type QueueGroup } from "@/store/appStore";
import { IconChip, type Tint } from "@/ui/IconChip";
import { useNow } from "@/useNow";

const GROUPS: { group: QueueGroup; title: string; empty: string; glyph: LucideIcon; tint: Tint }[] = [
  { group: "recording", title: "Recording", empty: "No sessions are being recorded.", glyph: LoaderCircle, tint: "blue" },
  { group: "ready", title: "Pending", empty: "No sessions are waiting to be recorded.", glyph: Clock, tint: "neutral" },
  { group: "waiting", title: "Retrying", empty: "No sessions are waiting for a retry.", glyph: RotateCw, tint: "amber" },
  { group: "failed", title: "Failed", empty: "No failed sessions.", glyph: CircleAlert, tint: "red" },
];

function groupOf(job: Job, current: Current | null, now: number): QueueGroup {
  if (job.id === current?.jobId) return "recording";
  if (job.status === "failed") return "failed";
  return job.nextAttemptAt > now ? "waiting" : "ready";
}

function stateLabel(job: Job, group: QueueGroup, current: Current | null, now: number): { text: string; tone: string } {
  switch (group) {
    case "recording":
      return { text: current ? elapsed(current.startedAt, now) : "Recording", tone: "" };
    case "waiting":
      return { text: `Retry in about ${minutesUntil(job.nextAttemptAt, now)} min`, tone: "warn" };
    case "failed":
      return { text: "Failed", tone: "danger" };
    case "ready":
      if (job.status === "applying") return { text: "Save interrupted", tone: "warn" };
      if (job.status === "index_pending") return { text: "Index pending", tone: "warn" };
      return { text: job.cursorLine > 0 ? "Resume recording" : "Pending", tone: "" };
  }
}

function QueueRow({ job, group, current, now }: { job: Job; group: QueueGroup; current: Current | null; now: number }) {
  const meta = GROUPS.find((entry) => entry.group === group)!;
  const name = job.project ?? providerLabel(job.provider);
  const session = job.sessionId ?? fileName(job.transcript.replace(/\\/g, "/"));
  const state = stateLabel(job, group, current, now);
  const error = group === "waiting" ? [`${job.attempts} failed attempts`, job.lastError].filter(Boolean).join(" · ") : job.lastError;
  return (
    <div className="row">
      <IconChip glyph={meta.glyph} tint={meta.tint} spin={group === "recording"} />
      <div className="row-main">
        <div className="row-title">
          <span className="name" title={name}>
            {name}
          </span>
          <span className="badge">{providerLabel(job.provider)}</span>
          <span className={`row-kind ${state.tone}`}>{state.text}</span>
        </div>
        <div className="row-meta mono" title={`Session ${session}\n${job.transcript}`}>
          {session}
        </div>
        {error && group !== "recording" && (
          <div className={group === "failed" ? "row-sub danger" : "row-sub"} title={error}>
            {error}
          </div>
        )}
      </div>
      {group === "failed" ? (
        <JobActions job={job} />
      ) : (
        group !== "recording" && (
          <span className="row-time" title="Time enqueued or last updated">
            {relativeTime(job.updatedAt, now)}
          </span>
        )
      )}
    </div>
  );
}

export function QueueView({ snapshot }: { snapshot: Snapshot }) {
  const now = useNow(15_000);
  const queue = useAppStore((store) => store.queue);
  const focus = useAppStore((store) => store.queueFocus);
  const sections = useRef<Partial<Record<QueueGroup, HTMLElement | null>>>({});
  const scrolled = useRef(false);

  useEffect(() => {
    if (!queue || scrolled.current) return;
    scrolled.current = true;
    sections.current[focus]?.scrollIntoView({ block: "start" });
  }, [queue, focus]);

  if (!queue) {
    return (
      <div className="body">
        <div className="empty">Loading queue…</div>
      </div>
    );
  }

  const grouped = new Map<QueueGroup, Job[]>(GROUPS.map(({ group }) => [group, []]));
  for (const job of queue) grouped.get(groupOf(job, snapshot.current, now))!.push(job);

  return (
    <div className="body">
      {GROUPS.map(({ group, title, empty }) => {
        const jobs = grouped.get(group)!;
        return (
          <section
            key={group}
            ref={(element) => {
              sections.current[group] = element;
            }}
            aria-label={title}
          >
            <div className="eyebrow">
              <span>
                {title}
                <span className="eyebrow-count">{jobs.length}</span>
              </span>
            </div>
            {jobs.length === 0 ? <div className="queue-empty">{empty}</div> : jobs.map((job) => <QueueRow key={job.id} job={job} group={group} current={snapshot.current} now={now} />)}
          </section>
        );
      })}
    </div>
  );
}
