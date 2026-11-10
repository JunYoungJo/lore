import { useCallback, useEffect, useRef, useState } from "react";
import { GitCompare, LoaderCircle, RefreshCw, Sparkles, TriangleAlert } from "lucide-react";
import * as api from "@/ipc/api";
import type { SyncChoice, SyncIssue, SyncPlan, SyncPreview, SyncSnapshot } from "@/ipc/contract";
import { useTheme } from "@/theme";

const SIDE: Record<string, string> = { base: "공통 기준", local: "이 PC", remote: "공유 저장소" };
const PHASE: Record<string, string> = { idle: "동기화 대기", review: "양쪽 변경 확인 필요", applying: "문서 반영 대기", index_pending: "문서 반영됨 · 검색 갱신 대기", push_pending: "로컬 반영됨 · 공유 대기", error: "동기화 확인 필요" };

function Comparison({ plan, paths }: { plan: SyncPlan; paths: Set<string> }) {
  return <details className="sync-comparison"><summary>공통 기준과 양쪽 문서 변경 보기</summary>
    {plan.files.filter((file) => paths.has(file.path)).map((file) => <section key={file.path}>
      <div className="sync-path mono">{file.path}{file.textConflict ? " · Git 충돌 있음" : ""}</div>
      <details><summary>마지막 공통 기준</summary><pre className="sync-document">{file.base ?? "문서 없음"}</pre></details>
      <div className="sync-two"><div><div className="eyebrow">이 PC</div><pre className="sync-document">{file.local ?? "문서 없음 / 삭제됨"}</pre></div><div><div className="eyebrow">공유 저장소</div><pre className="sync-document">{file.remote ?? "문서 없음 / 삭제됨"}</pre></div></div>
    </section>)}
  </details>;
}

function Issue({ issue, index, plan, choice, onChange, disabled }: { issue: SyncIssue; index: number; plan: SyncPlan; choice?: SyncChoice; onChange: (choice: SyncChoice) => void; disabled: boolean }) {
  const option = choice ? issue.options[choice.option] : undefined;
  return <section className="sync-issue" aria-label={issue.title}>
    <div className="sync-issue-title">{index + 1}. {issue.title}</div><p>{issue.explanation}</p>
    <div className="sync-two">{issue.evidence.map((evidence, evidenceIndex) => {
      const source = plan.sources.find((item) => item.id === evidence.source);
      return <div className="sync-evidence" key={`${evidence.source}-${evidenceIndex}`}><div className="eyebrow">{SIDE[source?.side ?? ""] ?? "근거"} · {source?.path}</div><blockquote>{evidence.quote}</blockquote>
        {source && <details><summary>{source.path.startsWith("sources/") ? "근거 대화 보기" : "근거 문서 보기"}</summary><pre className="sync-document">{source.text}</pre></details>}
      </div>;
    })}</div>
    <fieldset disabled={disabled} className="sync-choices"><legend>앞으로 적용할 내용</legend>{issue.options.map((item, optionIndex) => <label key={optionIndex} className="sync-option">
      <input type="radio" name={`sync-issue-${index}`} checked={choice?.option === optionIndex} onChange={() => onChange({ issue: index, option: optionIndex, customText: null })} />
      <span><strong>{item.label}</strong><span className="sync-option-detail">{item.description}</span></span>
    </label>)}</fieldset>
    {option?.edits.length === 1 && <label className="sync-option"><input type="checkbox" disabled={disabled} checked={choice?.customText !== null} onChange={(event) => onChange({ ...choice!, customText: event.target.checked ? option.edits[0].text : null })} /> 선택한 안의 문구 직접 정리</label>}
    {choice?.customText !== null && choice?.customText !== undefined && <label className="sync-custom">최종 문구<textarea className="input" rows={5} value={choice.customText} disabled={disabled} onChange={(event) => onChange({ ...choice, customText: event.target.value })} /></label>}
    <Comparison plan={plan} paths={new Set(issue.options.flatMap((item) => item.edits.map((edit) => edit.path)))} />
  </section>;
}

export function SyncApp() {
  useTheme();
  const [snapshot, setSnapshot] = useState<SyncSnapshot | null>(null);
  const [pending, setPending] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [choices, setChoices] = useState<Record<number, SyncChoice>>({});
  const [preview, setPreview] = useState<SyncPreview | null>(null);
  const [previewError, setPreviewError] = useState<string | null>(null);
  const alive = useRef(true);
  const load = useCallback(async () => {
    const next = await api.getSync();
    if (alive.current) setSnapshot(next);
    return next;
  }, []);
  useEffect(() => {
    alive.current = true;
    void load().catch((failure) => setError(String(failure)));
    const stop = api.onStateChanged(() => void load().catch((failure) => setError(String(failure))));
    return () => { alive.current = false; void stop.then((unlisten) => unlisten()); };
  }, [load]);
  const plan = snapshot?.plan;
  const busy = pending !== null || snapshot?.busy === true;
  useEffect(() => { setChoices({}); setPreview(null); setPreviewError(null); }, [plan?.id]);
  const selected = Object.values(choices).sort((a, b) => a.issue - b.issue);
  const complete = !!plan?.analysis && selected.length === plan.analysis.issues.length;
  const serialized = JSON.stringify(selected);
  useEffect(() => {
    setPreview(null); setPreviewError(null);
    if (!plan?.analysis || !complete || plan.phase !== "review" || busy) return;
    let current = true;
    const timer = window.setTimeout(() => {
      api.previewSync(plan.id, JSON.parse(serialized) as SyncChoice[]).then((next) => { if (current) setPreview(next); }, (failure) => { if (current) setPreviewError(String(failure)); });
    }, 200);
    return () => { current = false; window.clearTimeout(timer); };
  }, [plan?.id, plan?.analysis, complete, serialized, busy, plan?.phase]);
  const run = async (label: string, action: () => Promise<unknown>) => {
    setPending(label); setError(null); setNotice(null); setPreview(null);
    try {
      await action();
      const next = await load();
      if (next.summary.phase === "idle") setNotice("문서 공유와 검색 갱신을 완료했습니다.");
    } catch (failure) { if (alive.current) setError(String(failure)); }
    finally { if (alive.current) setPending(null); }
  };
  const choose = (choice: SyncChoice) => { setPreview(null); setChoices((current) => ({ ...current, [choice.issue]: choice })); };

  return <main className="review sync-window">
    <header className="review-head"><div className="head-text"><div className="head-title">기억 동기화 검토</div><div className="head-sub mono">{snapshot?.root}</div></div><span className="review-count">{busy ? pending ?? "동기화 작업 중" : PHASE[snapshot?.summary.phase ?? "idle"]}</span></header>
    <div className="sync-scroll">
      <div className="sync-toolbar"><button className="btn" disabled={busy} onClick={() => void run("공유 저장소 확인 중", api.prepareSync)}><RefreshCw size={14} />공유 저장소 확인</button>
        <button className="btn" disabled={busy} onClick={() => void api.openReview()}>로컬 변경 검토·커밋</button>
        <label className="sync-option"><input type="checkbox" checked={snapshot?.enabled ?? false} disabled={busy || !snapshot} onChange={(event) => void run("설정 저장 중", () => api.setSyncEnabled(event.target.checked))} />5분마다 자동 동기화</label>
      </div>
      <p className="help">현재 Git 브랜치에 연결된 remote/upstream을 사용합니다. 검토하지 않은 로컬 변경은 자동 커밋하지 않습니다. 모델 비교는 선택한 기록 모델과 사용 한도를 사용합니다.</p>
      {error && <div className="banner" role="alert"><TriangleAlert size={16} /><span>{error}</span></div>}
      {notice && <div className="sync-notice" role="status">{notice}</div>}
      {snapshot?.summary.message && <div className="banner" role="status">{snapshot.summary.message}</div>}
      {busy && <div className="sync-progress" role="status"><LoaderCircle size={16} className="spin" />{pending ?? "진행 중인 기록이 끝나면 동기화합니다."}</div>}
      {!plan && !busy && <div className="sync-empty"><GitCompare size={24} /><p>공유 저장소를 확인하면 가져올 변경과 보낼 커밋을 처리합니다.</p><div>양쪽 이력이 갈라졌으면 문서를 바꾸지 않고 비교 자료를 준비합니다.</div></div>}
      {plan && <>
        <div className="sync-meta mono">{plan.branch} ↔ {plan.remoteName}/{plan.remoteRef.replace("refs/heads/", "")} · 이 PC {plan.local.slice(0, 8)} · 공유 {plan.remote.slice(0, 8)}</div>
        {plan.phase === "review" && <>
          <p className="sync-hold">기억 기록은 대기 중입니다. Claude·Codex 작업과 훅의 큐 접수는 계속할 수 있습니다. 창을 닫아도 양쪽 내용과 검토 자료는 보존됩니다.</p>
          <div className="sync-toolbar"><button className="btn primary" disabled={busy || plan.unsupported.length > 0} onClick={() => void run("모델이 양쪽 문서와 근거를 비교하는 중", api.analyzeSync)}><Sparkles size={15} />{plan.analysis ? "모델로 다시 비교" : "모델로 비교"}</button><span className="help">모델은 충돌 후보와 선택지를 제안합니다. 최종 판단은 아래 근거와 문서를 보고 확인하십시오.</span></div>
          {plan.unsupported.length > 0 && <div className="banner" role="alert">사본·삭제/이름 변경 등 지원하지 않는 충돌은 외부 Git 도구에서 먼저 해결해야 합니다: {plan.unsupported.join(", ")}</div>}
          {plan.analysis && <><p className="sync-analysis">{plan.analysis.summary}</p>
            {plan.analysis.issues.map((issue, index) => <Issue key={`${plan.id}-${index}`} issue={issue} index={index} plan={plan} choice={choices[index]} disabled={busy} onChange={choose} />)}
            {plan.analysis.issues.length === 0 && <p>모델 검토 항목이 없습니다. 문서 전체의 병합 미리보기를 확인한 뒤 반영하십시오.</p>}
          </>}
          {!plan.analysis && <Comparison plan={plan} paths={new Set(plan.files.map((file) => file.path))} />}
          {(plan.otherFiles ?? []).length > 0 && <details><summary>함께 병합·공유할 다른 파일 {(plan.otherFiles ?? []).length}개</summary><p className="help">대화 사본에는 인용한 문장 외의 대화도 포함될 수 있습니다. 파일 단위로 공유됩니다.</p>{plan.otherFiles.map((path) => <div key={path} className="sync-path mono">{path}</div>)}</details>}
          {previewError && <div className="banner" role="alert">{previewError}</div>}
          {preview && <section className="sync-final" aria-label="최종 문서 미리보기"><h2>최종 문서 미리보기</h2><p className="help">문서 전체를 확인하십시오. 기존 근거 링크는 보존되고, 바꾸지 않은 문서와 대화 사본은 Git 병합안 그대로 유지됩니다.</p>
            {plan.files.filter((file) => !preview.files[file.path]).map((file) => <div key={file.path} className="sync-path">삭제: {file.path}</div>)}
            {Object.entries(preview.files).map(([path, text], index) => <details key={path} open={index === 0}><summary>{path}</summary><pre className="sync-document">{text}</pre></details>)}
          </section>}
          <div className="sync-footer"><button className="btn" onClick={() => void api.deferSync()} disabled={busy}>나중에</button><button className="btn primary" disabled={busy || !preview} onClick={() => { if (preview) void run("최종 문구 반영·공유·검색 갱신 중", () => api.applySync(plan.id, selected, preview.token)); }}>이 내용으로 반영·공유</button></div>
        </>}
        {plan.phase !== "review" && <div className="sync-footer"><button className="btn primary" disabled={busy} onClick={() => void run("중단된 동기화 재개 중", api.resumeSync)}>{plan.phase === "push_pending" ? "전송 다시 시도" : "반영·검색 갱신 다시 시도"}</button></div>}
      </>}
    </div>
  </main>;
}
