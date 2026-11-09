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
  recorded: { glyph: FileCheck2, tint: "green", label: "기록함", tone: "ok" },
  no_change: { glyph: CircleDashed, tint: "neutral", label: "기록할 내용 없음", tone: "" },
  retry_scheduled: { glyph: RotateCw, tint: "amber", label: "다시 시도 예정", tone: "warn" },
  failed: { glyph: CircleAlert, tint: "red", label: "실패", tone: "danger" },
};

function NowCard({ snapshot, now }: { snapshot: Snapshot; now: number }) {
  const togglePause = useAppStore((store) => store.togglePause);
  const { current, settings } = snapshot;
  const chosen = settings.models[settings.backend];
  const model = `${providerLabel(settings.backend)}${chosen ? ` ${chosen}` : ""}`;

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

  if (snapshot.reviewing) {
    return (
      <section className="now paused" aria-label="진행 중">
        <IconChip glyph={GitCompare} tint="amber" />
        <div className="now-main">
          <div className="now-title">검토 중</div>
          <div className="now-sub">검토 창이 열려 있는 동안에는 새 기록을 시작하지 않습니다. 창을 닫으면 차례로 기록합니다.</div>
        </div>
        <button type="button" className="pill primary" onClick={() => void api.openReview()}>
          검토 창
        </button>
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
  const showQueue = useAppStore((store) => store.showQueue);
  const { counts, review } = snapshot;
  const items: { group: QueueGroup; label: string; value: number; tone: string; hint: string }[] = [
    { group: "ready", label: "대기", value: counts.ready, tone: counts.ready === 0 ? "muted" : "", hint: "기록을 기다리는 세션을 봅니다" },
    { group: "waiting", label: "재시도 대기", value: counts.waiting, tone: counts.waiting > 0 ? "warn" : "muted", hint: "다시 시도를 기다리는 세션을 봅니다" },
    { group: "failed", label: "실패", value: counts.failed, tone: counts.failed > 0 ? "danger" : "muted", hint: "실패한 세션을 봅니다" },
  ];
  const reviewValue = review === null ? "–" : String(review);
  return (
    <section className="stats" aria-label="큐 상태">
      {items.map((item) => (
        <button key={item.label} type="button" className={`stat stat-button ${item.tone}`} aria-label={`${item.label} ${item.value}`} title={item.hint} onClick={() => showQueue(item.group)}>
          <div className="stat-value">{item.value}</div>
          <div className="stat-label">{item.label}</div>
        </button>
      ))}
      <button type="button" className={`stat stat-button ${review ? "" : "muted"}`} aria-label={`검토 대기 ${reviewValue}`} title="바뀐 기억 문서를 검토하고 커밋합니다" onClick={() => void api.openReview()}>
        <div className="stat-value">{reviewValue}</div>
        <div className="stat-label">검토 대기</div>
      </button>
    </section>
  );
}

function AttentionRow({ job, now }: { job: Job; now: number }) {
  const failed = job.status === "failed";
  const status = failed ? "실패" : job.status === "index_pending" ? "색인 갱신 대기" : "문서 저장 중단";
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
        {!failed && job.nextAttemptAt > now && <div className="row-sub">약 {minutesUntil(job.nextAttemptAt, now)}분 뒤 다시 시도합니다.</div>}
      </div>
      {failed && <JobActions job={job} />}
    </div>
  );
}

function ActivityRow({ activity, now }: { activity: Activity; now: number }) {
  const kind = KINDS[activity.kind] ?? KINDS.no_change!;
  const detail = activity.error ?? activity.summary;
  const label = activity.kind === "recorded" ? `문서 ${activity.notes.length}개 고침` : kind.label;
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

      {snapshot.attention.length > 0 && (
        <section aria-label="확인 필요">
          <div className="eyebrow">확인 필요</div>
          {snapshot.attention.map((job) => (
            <AttentionRow key={job.id} job={job} now={now} />
          ))}
        </section>
      )}

      <section aria-label="최근 기록">
        <div className="eyebrow">
          최근 기록
          {snapshot.recent.length > 0 && (
            <button ref={clearButton} type="button" className="eyebrow-btn" aria-label="최근 기록 지우기" aria-expanded={confirming} onClick={() => setConfirming(!confirming)}>
              지우기
            </button>
          )}
        </div>
        {confirming && snapshot.recent.length > 0 && (
          <ConfirmCard
            anchor={clearButton}
            label="최근 기록 지우기 확인"
            message="최근 기록을 모두 지울까요? 기억 문서와 큐는 그대로 둡니다."
            action="모두 지우기"
            onConfirm={() => void clearActivity()}
            onClose={() => setConfirming(false)}
          />
        )}
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
