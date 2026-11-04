import {
  CircleAlert,
  CircleCheck,
  CircleDashed,
  FileCheck2,
  FileText,
  Inbox,
  LoaderCircle,
  Pause,
  Plug,
  RotateCw,
  type LucideIcon,
} from "lucide-react";

import * as api from "@/ipc/api";
import type { Activity, ActivityKind, Job, Snapshot } from "@/ipc/contract";
import { elapsed, fileName, minutesUntil, providerLabel, relativeTime } from "@/format";
import { useAppStore } from "@/store/appStore";
import { IconChip, type Tint } from "@/ui/IconChip";

const KINDS: Partial<Record<ActivityKind, { glyph: LucideIcon; tint: Tint; label: string; tone: string }>> = {
  recorded: { glyph: FileCheck2, tint: "green", label: "기록함", tone: "ok" },
  no_change: { glyph: CircleDashed, tint: "neutral", label: "기록할 내용 없음", tone: "" },
  retry_scheduled: { glyph: RotateCw, tint: "amber", label: "다시 시도 예정", tone: "warn" },
  failed: { glyph: CircleAlert, tint: "red", label: "실패", tone: "danger" },
};

function NowCard({ snapshot, now }: { snapshot: Snapshot; now: number }) {
  const togglePause = useAppStore((store) => store.togglePause);
  const { current, settings } = snapshot;
  const model = `${providerLabel(settings.backend)}${settings.model ? ` ${settings.model}` : ""}`;

  if (current) {
    return (
      <section className="now working" aria-label="진행 중">
        <IconChip glyph={LoaderCircle} tint="blue" spin />
        <div className="now-main">
          <div className="now-title">{current.project} 세션을 정리하고 있습니다</div>
          <div className="now-sub">
            {providerLabel(current.provider)} 세션 · {model} · {elapsed(current.startedAt, now)}
          </div>
          <div className="progress" aria-hidden="true">
            <span />
          </div>
        </div>
      </section>
    );
  }

  if (settings.paused) {
    return (
      <section className="now paused" aria-label="진행 중">
        <IconChip glyph={Pause} tint="amber" />
        <div className="now-main">
          <div className="now-title">기록을 멈췄습니다</div>
          <div className="now-sub">끝난 세션은 계속 큐에 쌓이고, 다시 시작하면 차례로 기록합니다.</div>
        </div>
        <button type="button" className="pill primary" onClick={() => void togglePause()}>
          다시 시작
        </button>
      </section>
    );
  }

  const last = snapshot.recent.find((activity) => activity.kind === "recorded" || activity.kind === "no_change");
  return (
    <section className="now" aria-label="진행 중">
      <IconChip glyph={CircleCheck} tint="green" />
      <div className="now-main">
        <div className="now-title">새 세션을 기다리고 있습니다</div>
        <div className="now-sub">
          {last ? `마지막 기록 ${relativeTime(last.at, now)} · ${last.project ?? providerLabel(last.provider)}` : "Claude·Codex 세션이 끝나면 자동으로 기록합니다."}
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
    <section className="callout" aria-label="훅 연결 필요">
      <IconChip glyph={Plug} tint="amber" size={28} />
      <div className="callout-main">
        <div className="callout-title">{names} 훅 연결 필요</div>
        <div className="callout-sub">연결 전에는 세션이 끝나도 기록하지 않습니다.</div>
      </div>
      <button type="button" className="pill primary" onClick={showSettings}>
        연결
      </button>
    </section>
  );
}

function Stats({ snapshot }: { snapshot: Snapshot }) {
  const { counts } = snapshot;
  const items: { label: string; value: number; tone: string }[] = [
    { label: "대기", value: counts.ready, tone: counts.ready === 0 ? "muted" : "" },
    { label: "재시도 대기", value: counts.waiting, tone: counts.waiting > 0 ? "warn" : "muted" },
    { label: "실패", value: counts.failed, tone: counts.failed > 0 ? "danger" : "muted" },
    { label: "완료", value: counts.done, tone: counts.done === 0 ? "muted" : "" },
  ];
  return (
    <section className="stats" aria-label="큐 상태">
      {items.map((item) => (
        <div key={item.label} className={`stat ${item.tone}`}>
          <div className="stat-value">{item.value}</div>
          <div className="stat-label">{item.label}</div>
        </div>
      ))}
    </section>
  );
}

function AttentionRow({ job, now }: { job: Job; now: number }) {
  const retry = useAppStore((store) => store.retry);
  const failed = job.status === "failed";
  const status = failed ? "실패" : job.status === "index_pending" ? "색인 갱신 대기" : "문서 저장 중단";
  return (
    <div className="row">
      <IconChip glyph={failed ? CircleAlert : RotateCw} tint={failed ? "red" : "amber"} />
      <div className="row-main">
        <div className="row-title">
          <span className="name">{job.project ?? providerLabel(job.provider)}</span>
          <span className="badge">{providerLabel(job.provider)}</span>
          <span className={`row-kind ${failed ? "danger" : "warn"}`}>{status}</span>
        </div>
        {job.lastError && <div className="row-sub danger">{job.lastError}</div>}
        {!failed && job.nextAttemptAt > now && <div className="row-sub">약 {minutesUntil(job.nextAttemptAt, now)}분 뒤 다시 시도합니다.</div>}
      </div>
      {failed && (
        <button type="button" className="pill primary" onClick={() => void retry(job.id)}>
          다시 시도
        </button>
      )}
    </div>
  );
}

function ActivityRow({ activity, now }: { activity: Activity; now: number }) {
  const kind = KINDS[activity.kind] ?? KINDS.no_change!;
  const detail = activity.error ?? activity.summary;
  const label = activity.kind === "recorded" ? `문서 ${activity.notes.length}개 고침` : kind.label;
  return (
    <div className="row">
      <IconChip glyph={kind.glyph} tint={kind.tint} />
      <div className="row-main">
        <div className="row-title">
          <span className="name">{activity.project ?? providerLabel(activity.provider)}</span>
          <span className="badge">{providerLabel(activity.provider)}</span>
          <span className={`row-kind ${kind.tone}`}>{label}</span>
        </div>
        {detail && <div className={activity.error ? "row-sub danger" : "row-sub"}>{detail}</div>}
        {activity.notes.length > 0 && (
          <div className="files">
            {activity.notes.map((note) => (
              <button key={note} type="button" className="file-chip" title={note} aria-label={`${fileName(note)} 열기`} onClick={() => void api.openNote(note)}>
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
  const now = Math.floor(Date.now() / 1000);
  const recent = snapshot.recent.filter((activity) => KINDS[activity.kind]);

  return (
    <div className="body">
      <NowCard snapshot={snapshot} now={now} />
      <HookCallout snapshot={snapshot} />
      <Stats snapshot={snapshot} />

      {snapshot.attention.length > 0 && (
        <section aria-label="확인 필요">
          <div className="eyebrow">확인 필요</div>
          {snapshot.attention.map((job) => (
            <AttentionRow key={job.id} job={job} now={now} />
          ))}
        </section>
      )}

      <section aria-label="최근 기록">
        <div className="eyebrow">최근 기록</div>
        {recent.length === 0 ? (
          <div className="empty">
            <div className="empty-icon">
              <Inbox size={20} strokeWidth={1.75} />
            </div>
            <div className="empty-title">아직 기록한 세션이 없습니다</div>
            <div>세션이 끝나고 기록을 마치면 여기에 표시됩니다.</div>
          </div>
        ) : (
          recent.map((activity) => <ActivityRow key={activity.id} activity={activity} now={now} />)
        )}
      </section>
    </div>
  );
}
