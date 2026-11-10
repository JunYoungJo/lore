import { useCallback, useEffect, useRef, useState } from "react";
import { GitCompare, LoaderCircle, RefreshCw, Sparkles, TriangleAlert } from "lucide-react";
import * as api from "@/ipc/api";
import type { SyncChoice, SyncIssue, SyncPlan, SyncPreview, SyncSnapshot } from "@/ipc/contract";
import { useTheme } from "@/theme";

const SIDE: Record<string, string> = { base: "Base", local: "Local", remote: "Remote" };
const PHASE: Record<string, string> = { idle: "Sync idle", review: "Review diverged changes", applying: "Apply pending", index_pending: "Applied · Index pending", push_pending: "Applied locally · Push pending", error: "Sync needs attention" };

function Comparison({ plan, paths }: { plan: SyncPlan; paths: Set<string> }) {
  return <details className="sync-comparison"><summary>Compare base, local, and remote documents</summary>
    {plan.files.filter((file) => paths.has(file.path)).map((file) => <section key={file.path}>
      <div className="sync-path mono">{file.path}{file.textConflict ? " · Git conflict" : ""}</div>
      <details><summary>Common base</summary><pre className="sync-document">{file.base ?? "No document"}</pre></details>
      <div className="sync-two"><div><div className="eyebrow">Local</div><pre className="sync-document">{file.local ?? "No document / deleted"}</pre></div><div><div className="eyebrow">Remote</div><pre className="sync-document">{file.remote ?? "No document / deleted"}</pre></div></div>
    </section>)}
  </details>;
}

function Issue({ issue, index, plan, choice, onChange, disabled }: { issue: SyncIssue; index: number; plan: SyncPlan; choice?: SyncChoice; onChange: (choice: SyncChoice) => void; disabled: boolean }) {
  const option = choice ? issue.options[choice.option] : undefined;
  return <section className="sync-issue" aria-label={issue.title}>
    <div className="sync-issue-title">{index + 1}. {issue.title}</div><p>{issue.explanation}</p>
    <div className="sync-two">{issue.evidence.map((evidence, evidenceIndex) => {
      const source = plan.sources.find((item) => item.id === evidence.source);
      return <div className="sync-evidence" key={`${evidence.source}-${evidenceIndex}`}><div className="eyebrow">{SIDE[source?.side ?? ""] ?? "Evidence"} · {source?.path}</div><blockquote>{evidence.quote}</blockquote>
        {source && <details><summary>{source.path.startsWith("sources/") ? "View source conversation" : "View source document"}</summary><pre className="sync-document">{source.text}</pre></details>}
      </div>;
    })}</div>
    <fieldset disabled={disabled} className="sync-choices"><legend>Policy to keep</legend>{issue.options.map((item, optionIndex) => <label key={optionIndex} className="sync-option">
      <input type="radio" name={`sync-issue-${index}`} checked={choice?.option === optionIndex} onChange={() => onChange({ issue: index, option: optionIndex, customText: null })} />
      <span><strong>{item.label}</strong><span className="sync-option-detail">{item.description}</span></span>
    </label>)}</fieldset>
    {option?.edits.length === 1 && <label className="sync-option"><input type="checkbox" disabled={disabled} checked={choice?.customText !== null} onChange={(event) => onChange({ ...choice!, customText: event.target.checked ? option.edits[0].text : null })} /> Edit the selected wording</label>}
    {choice?.customText !== null && choice?.customText !== undefined && <label className="sync-custom">Final wording<textarea className="input" rows={5} value={choice.customText} disabled={disabled} onChange={(event) => onChange({ ...choice, customText: event.target.value })} /></label>}
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
      if (next.summary.phase === "idle") setNotice("Documents shared and index refreshed.");
    } catch (failure) { if (alive.current) setError(String(failure)); }
    finally { if (alive.current) setPending(null); }
  };
  const choose = (choice: SyncChoice) => { setPreview(null); setChoices((current) => ({ ...current, [choice.issue]: choice })); };

  return <main className="review sync-window">
    <header className="review-head"><div className="head-text"><div className="head-title">Review memory sync</div><div className="head-sub mono">{snapshot?.root}</div></div><span className="review-count">{busy ? pending ?? "Syncing" : PHASE[snapshot?.summary.phase ?? "idle"]}</span></header>
    <div className="sync-scroll">
      <div className="sync-toolbar"><button className="btn" disabled={busy} onClick={() => void run("Checking remote", api.prepareSync)}><RefreshCw size={14} />Check remote</button>
        <button className="btn" disabled={busy} onClick={() => void api.openReview()}>Review & commit local changes</button>
        <label className="sync-option"><input type="checkbox" checked={snapshot?.enabled ?? false} disabled={busy || !snapshot} onChange={(event) => void run("Saving settings", () => api.setSyncEnabled(event.target.checked))} />Auto-sync every 5 minutes</label>
      </div>
      <p className="help">Uses the current branch’s configured upstream. Unreviewed local changes are not committed automatically. Comparison uses your selected recording model and its usage allowance.</p>
      {error && <div className="banner" role="alert"><TriangleAlert size={16} /><span>{error}</span></div>}
      {notice && <div className="sync-notice" role="status">{notice}</div>}
      {snapshot?.summary.message && <div className="banner" role="status">{snapshot.summary.message}</div>}
      {busy && <div className="sync-progress" role="status"><LoaderCircle size={16} className="spin" />{pending ?? "Sync will start after the current recording finishes."}</div>}
      {!plan && !busy && <div className="sync-empty"><GitCompare size={24} /><p>Check the remote to receive changes and send committed memory.</p><div>If histories have diverged, a comparison is prepared before changing your documents.</div></div>}
      {plan && <>
        <div className="sync-meta mono">{plan.branch} ↔ {plan.remoteName}/{plan.remoteRef.replace("refs/heads/", "")} · Local {plan.local.slice(0, 8)} · Remote {plan.remote.slice(0, 8)}</div>
        {plan.phase === "review" && <>
          <p className="sync-hold">Memory recording is on hold. Claude and Codex work and hook intake can continue. Both versions and the review are kept when you close this window.</p>
          <div className="sync-toolbar"><button className="btn primary" disabled={busy || plan.unsupported.length > 0} onClick={() => void run("Comparing documents and evidence", api.analyzeSync)}><Sparkles size={15} />{plan.analysis ? "Compare again with model" : "Compare with model"}</button><span className="help">The model proposes potential conflicts and options. Check the evidence and documents before deciding.</span></div>
          {plan.unsupported.length > 0 && <div className="banner" role="alert">Resolve unsupported conflicts, such as conversation copies, deletions, or renames, in an external Git tool first: {plan.unsupported.join(", ")}</div>}
          {plan.analysis && <><p className="sync-analysis">{plan.analysis.summary}</p>
            {plan.analysis.issues.map((issue, index) => <Issue key={`${plan.id}-${index}`} issue={issue} index={index} plan={plan} choice={choices[index]} disabled={busy} onChange={choose} />)}
            {plan.analysis.issues.length === 0 && <p>No issues to choose from. Check the full merged document preview before applying.</p>}
          </>}
          {!plan.analysis && <Comparison plan={plan} paths={new Set(plan.files.map((file) => file.path))} />}
          {(plan.otherFiles ?? []).length > 0 && <details><summary>Other files to merge and share: {(plan.otherFiles ?? []).length}</summary><p className="help">Conversation copies may contain more than the quoted sentences. Entire files are shared.</p>{plan.otherFiles.map((path) => <div key={path} className="sync-path mono">{path}</div>)}</details>}
          {previewError && <div className="banner" role="alert">{previewError}</div>}
          {preview && <section className="sync-final" aria-label="Final document preview"><h2>Final document preview</h2><p className="help">Check the complete documents. Existing source links are preserved. Unedited documents and conversation copies use the Git merge result.</p>
            {plan.files.filter((file) => !preview.files[file.path]).map((file) => <div key={file.path} className="sync-path">Deleted: {file.path}</div>)}
            {Object.entries(preview.files).map(([path, text], index) => <details key={path} open={index === 0}><summary>{path}</summary><pre className="sync-document">{text}</pre></details>)}
          </section>}
          <div className="sync-footer"><button className="btn" onClick={() => void api.deferSync()} disabled={busy}>Later</button><button className="btn primary" disabled={busy || !preview} onClick={() => { if (preview) void run("Applying, indexing, and sharing", () => api.applySync(plan.id, selected, preview.token)); }}>Apply & share</button></div>
        </>}
        {plan.phase !== "review" && <div className="sync-footer"><button className="btn primary" disabled={busy} onClick={() => void run("Resuming sync", api.resumeSync)}>{plan.phase === "push_pending" ? "Retry push" : "Retry apply & index"}</button></div>}
      </>}
    </div>
  </main>;
}
