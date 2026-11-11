import { useCallback, useEffect, useRef, useState } from "react";
import { Check, ChevronRight, CircleCheck, CloudUpload, Eye, FileText, Info, LoaderCircle, Sparkles, TriangleAlert } from "lucide-react";

import * as api from "@/ipc/api";
import type { SyncChoice, SyncFileLines, SyncIssue, SyncPlan, SyncPreview, SyncSnapshot } from "@/ipc/contract";
import { DiffView } from "@/components/DiffView";
import { fileName, relativePhrase } from "@/format";
import { useTheme } from "@/theme";
import { BrandMark } from "@/ui/BrandMark";
import { WindowClose } from "@/ui/WindowClose";
import { useNow } from "@/useNow";

const SIDE: Record<string, string> = { base: "Base", local: "This PC", remote: "Other PCs" };

type View = { kind: "overview" } | { kind: "issue"; index: number } | { kind: "file"; path: string } | { kind: "others" } | { kind: "result" };

const plural = (count: number, one: string, many = `${one}s`) => `${count} ${count === 1 ? one : many}`;
const project = (path: string) => (path.startsWith("projects/") ? path.split("/")[1] : path.split("/")[0]);
const upstreamOf = (tracking: string | null) => tracking?.split("↔ ")[1] ?? "the upstream branch";

function headerState(snapshot: SyncSnapshot, busy: boolean): { label: string; tone: "" | "ok" | "warn" } {
  if (busy) return { label: "Syncing…", tone: "" };
  switch (snapshot.summary.phase) {
    case "review":
      return { label: "Review needed", tone: "warn" };
    case "push_pending":
      return { label: "Not shared yet", tone: "warn" };
    case "index_pending":
      return { label: "Search not updated", tone: "warn" };
    case "applying":
      return { label: "Not finished", tone: "warn" };
    case "error":
      return { label: "Needs attention", tone: "warn" };
    default:
      return snapshot.summary.lastSyncedAt ? { label: "Up to date", tone: "ok" } : { label: "Not synced yet", tone: "" };
  }
}

function Counts({ lines }: { lines?: SyncFileLines }) {
  if (!lines) return null;
  const side = (name: string, added: number, removed: number) => (
    <>
      {name}
      {added > 0 && <span className="sync-plus"> +{added}</span>}
      {removed > 0 && <span className="sync-minus"> −{removed}</span>}
    </>
  );
  return (
    <>
      {" · "}
      {side("This PC", lines.local.added, lines.local.removed)}
      {" · "}
      {side("Other PCs", lines.remote.added, lines.remote.removed)}
    </>
  );
}

function Documents({ plan, paths }: { plan: SyncPlan; paths: Set<string> }) {
  return (
    <>
      {plan.files
        .filter((file) => paths.has(file.path))
        .map((file) => (
          <section key={file.path} className="sync-docs">
            <div className="sync-path mono">
              {file.path}
              {file.textConflict ? " · Git conflict" : ""}
            </div>
            <details>
              <summary>Common base</summary>
              <pre className="sync-document">{file.base ?? "No document"}</pre>
            </details>
            <div className="sync-two">
              <div>
                <div className="sync-ev-label">This PC</div>
                <pre className="sync-document">{file.local ?? "No document / deleted"}</pre>
              </div>
              <div>
                <div className="sync-ev-label">Other PCs</div>
                <pre className="sync-document">{file.remote ?? "No document / deleted"}</pre>
              </div>
            </div>
          </section>
        ))}
    </>
  );
}

function IssueView({ issue, index, total, plan, choice, onChange, disabled }: { issue: SyncIssue; index: number; total: number; plan: SyncPlan; choice?: SyncChoice; onChange: (choice: SyncChoice) => void; disabled: boolean }) {
  const option = choice ? issue.options[choice.option] : undefined;
  const target = issue.options[0]?.edits[0]?.path;
  return (
    <section className="sync-pane-inner" aria-label={issue.title}>
      <span className="sync-over">
        Decision {index + 1} of {total}
      </span>
      <h1 className="sync-title">{issue.title}</h1>
      <p className="sync-lead">{issue.explanation}</p>
      <div className="sync-two sync-evidence-grid">
        {issue.evidence.map((evidence, evidenceIndex) => {
          const source = plan.sources.find((item) => item.id === evidence.source);
          return (
            <div className="sync-ev" key={`${evidence.source}-${evidenceIndex}`}>
              <div className="sync-ev-label">
                <b>{SIDE[source?.side ?? ""] ?? "Evidence"}</b> · {source?.path}
              </div>
              <blockquote>{evidence.quote}</blockquote>
              {source && (
                <details>
                  <summary>{source.path.startsWith("sources/") ? "View source conversation" : "View source document"}</summary>
                  <pre className="sync-document">{source.text}</pre>
                </details>
              )}
            </div>
          );
        })}
      </div>
      <fieldset disabled={disabled} className="sync-options">
        <legend>
          {target ? (
            <>
              What should <code>{target}</code> say?
            </>
          ) : (
            "Choose what to keep"
          )}
        </legend>
        {issue.options.map((item, optionIndex) => (
          <label key={optionIndex} className={`sync-opt ${choice?.option === optionIndex ? "on" : ""}`}>
            <input type="radio" name={`sync-issue-${index}`} checked={choice?.option === optionIndex} onChange={() => onChange({ issue: index, option: optionIndex, customText: null })} />
            <span>
              <strong>{item.label}</strong>
              <small>{item.description}</small>
            </span>
          </label>
        ))}
      </fieldset>
      {option?.edits.length === 1 && (
        <label className="sync-check">
          <input type="checkbox" disabled={disabled} checked={choice?.customText !== null} onChange={(event) => onChange({ ...choice!, customText: event.target.checked ? option.edits[0].text : null })} />
          Edit the selected wording
        </label>
      )}
      {choice?.customText !== null && choice?.customText !== undefined && (
        <label className="sync-custom">
          Final wording
          <textarea className="input" rows={5} value={choice.customText} disabled={disabled} onChange={(event) => onChange({ ...choice, customText: event.target.value })} />
        </label>
      )}
      <details className="sync-compare">
        <summary>Compare base, this PC and other PCs</summary>
        <Documents plan={plan} paths={new Set(issue.options.flatMap((item) => item.edits.map((edit) => edit.path)))} />
      </details>
    </section>
  );
}

function ResultView({ plan, preview, previewError }: { plan: SyncPlan; preview: SyncPreview | null; previewError: string | null }) {
  const paths = plan.files.map((file) => file.path);
  const [active, setActive] = useState(paths[0] ?? "");
  const deleted = preview !== null && !(active in preview.files);
  return (
    <section className="sync-pane-inner" aria-label="Final check">
      <span className="sync-over">Final check</span>
      <h1 className="sync-title">Check the result before sharing</h1>
      <p className="sync-lead">This is what the notes will say on every PC. Lines that differ from this PC’s current version are highlighted.</p>
      {previewError && (
        <div className="banner" role="alert">
          <TriangleAlert size={15} />
          <span>{previewError}</span>
        </div>
      )}
      {!preview && !previewError && (
        <div className="sync-progress" role="status">
          <LoaderCircle size={16} className="spin" />
          Preparing the final documents…
        </div>
      )}
      {preview && (
        <>
          {paths.length > 1 && (
            <div className="sync-tabs" role="tablist" aria-label="Changed notes">
              {paths.map((path) => (
                <button key={path} type="button" role="tab" aria-selected={path === active} className={path === active ? "on" : ""} onClick={() => setActive(path)}>
                  {fileName(path)}
                </button>
              ))}
            </div>
          )}
          {deleted ? <p className="sync-lead">This note is deleted: {active}</p> : <DiffView path={active} rows={preview.diffs?.[active] ?? []} labels={["This PC now", "After applying"]} />}
          {plan.otherFiles.length > 0 && <p className="sync-note">The other {plural(plan.otherFiles.length, "file")} use the Git merge result. Existing source links are kept.</p>}
        </>
      )}
    </section>
  );
}

const RECOVERY: Record<string, { title: string; lead: (upstream: string) => string; primary: string; secondary?: string; safe: string }> = {
  push_pending: {
    title: "Applied on this PC, not shared yet",
    lead: (upstream) => `The merged notes are saved and searchable on this PC. Sending them to ${upstream} failed, so your other PCs don’t have them yet.`,
    primary: "Sync again",
    secondary: "Retry sending only",
    safe: "Recording continues while this waits. Nothing on this PC is lost.",
  },
  index_pending: {
    title: "Applied on this PC, search not updated",
    lead: () => "The merged notes are in your memory folder, but this PC’s search index did not refresh.",
    primary: "Retry search update",
    safe: "Recording waits until this finishes. Nothing is applied twice.",
  },
  applying: {
    title: "Sync stopped before applying",
    lead: () => "The reviewed result is kept. Applying it to your memory folder did not finish.",
    primary: "Retry apply",
    safe: "Recording waits until this finishes. Your choices are kept.",
  },
};

export function SyncApp() {
  useTheme();
  const now = useNow(15_000);
  const [snapshot, setSnapshot] = useState<SyncSnapshot | null>(null);
  const [pending, setPending] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [choices, setChoices] = useState<Record<number, SyncChoice>>({});
  const [preview, setPreview] = useState<SyncPreview | null>(null);
  const [previewError, setPreviewError] = useState<string | null>(null);
  const [view, setView] = useState<View>({ kind: "overview" });
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
    const stopProgress = api.onSyncComparisonProgress((comparison) => {
      if (alive.current) setSnapshot((current) => current ? { ...current, comparison } : current);
    });
    return () => {
      alive.current = false;
      void stop.then((unlisten) => unlisten());
      void stopProgress.then((unlisten) => unlisten());
    };
  }, [load]);

  const plan = snapshot?.plan;
  const analysis = plan?.analysis ?? null;
  const busy = pending !== null || snapshot?.busy === true;
  const comparison = snapshot?.comparison;
  const resumable = !analysis && comparison && comparison.stage !== "complete";
  const comparisonStatus = comparison?.stage === "checking"
    ? `Checking consistency across ${comparison.total} groups…`
    : comparison ? `${comparison.completed} of ${comparison.total} groups compared${busy && comparison.reads ? " · Reading supporting context" : ""}` : null;
  useEffect(() => {
    setChoices({});
    setPreview(null);
    setPreviewError(null);
  }, [plan?.id]);
  useEffect(() => {
    if (!analysis) setView({ kind: "overview" });
    else setView(analysis.issues.length > 0 ? { kind: "issue", index: 0 } : { kind: "result" });
  }, [plan?.id, analysis]);

  const selected = Object.values(choices).sort((a, b) => a.issue - b.issue);
  const complete = !!analysis && selected.length === analysis.issues.length;
  const serialized = JSON.stringify(selected);
  useEffect(() => {
    setPreview(null);
    setPreviewError(null);
    if (!plan?.analysis || !complete || plan.phase !== "review" || busy) return;
    let current = true;
    const timer = window.setTimeout(() => {
      api.previewSync(plan.id, JSON.parse(serialized) as SyncChoice[]).then(
        (next) => {
          if (current) setPreview(next);
        },
        (failure) => {
          if (current) setPreviewError(String(failure));
        },
      );
    }, 200);
    return () => {
      current = false;
      window.clearTimeout(timer);
    };
  }, [plan?.id, plan?.analysis, complete, serialized, busy, plan?.phase]);

  const run = async (label: string, action: () => Promise<unknown>) => {
    setPending(label);
    setError(null);
    setNotice(null);
    setPreview(null);
    try {
      await action();
      const next = await load();
      if (next.summary.phase === "idle" && label !== "Checking remote") setNotice("Documents shared and index refreshed.");
    } catch (failure) {
      if (alive.current) setError(String(failure));
    } finally {
      if (alive.current) setPending(null);
    }
  };
  const close = () => void api.deferSync().catch((failure) => setError(String(failure)));
  const choose = (choice: SyncChoice) => {
    setPreview(null);
    setChoices((current) => ({ ...current, [choice.issue]: choice }));
  };

  if (!snapshot) {
    return (
      <main className="sync-window">
        <div className="empty">{error ?? "Loading…"}</div>
      </main>
    );
  }

  const state = headerState(snapshot, busy);
  const upstream = upstreamOf(snapshot.tracking);
  const errorBanner = error && (
    <div className="banner" role="alert">
      <TriangleAlert size={15} />
      <span>{error}</span>
    </div>
  );

  let body;
  if (plan && plan.phase === "review") {
    const decided = selected.length;
    const total = analysis?.issues.length ?? 0;
    const lines = (path: string) => snapshot.changes.find((change) => change.path === path);
    body = (
      <>
        <div className="sync-main">
          <aside className="sync-rail" aria-label="Sync review steps">
            {analysis ? (
              <>
                <div className="sync-rail-label">
                  <span>Decisions</span>
                  <span>
                    {decided} of {total}
                  </span>
                </div>
                {analysis.issues.map((issue, index) => {
                  const choice = choices[index];
                  const target = issue.options[0]?.edits[0]?.path;
                  return (
                    <button key={index} type="button" className={`sync-item ${view.kind === "issue" && view.index === index ? "on" : ""}`} onClick={() => setView({ kind: "issue", index })}>
                      <span className={`sync-mark ${choice ? "done" : ""}`}>{choice ? <Check size={11} strokeWidth={3} /> : index + 1}</span>
                      <span className="sync-item-main">
                        <span className="sync-item-title">{issue.title}</span>
                        <span className="sync-item-sub">
                          {target ? `${fileName(target)} · ` : ""}
                          {choice ? issue.options[choice.option]?.label : "Not decided"}
                        </span>
                      </span>
                    </button>
                  );
                })}
              </>
            ) : (
              <>
                <div className="sync-rail-label">
                  <span>Changed on both PCs</span>
                  <span>{plan.files.length}</span>
                </div>
                {plan.files.map((file) => (
                  <button key={file.path} type="button" className={`sync-item ${view.kind === "file" && view.path === file.path ? "on" : ""}`} onClick={() => setView({ kind: "file", path: file.path })}>
                    <FileText size={16} strokeWidth={1.9} />
                    <span className="sync-item-main">
                      <span className="sync-item-title" title={file.path}>
                        {fileName(file.path)}
                      </span>
                      <span className="sync-item-sub">
                        {project(file.path)}
                        <Counts lines={lines(file.path)} />
                      </span>
                    </span>
                  </button>
                ))}
              </>
            )}
            {plan.otherFiles.length > 0 && (
              <>
                <div className="sync-rail-label">
                  <span>{analysis ? "Other changes" : "Merge automatically"}</span>
                  <span>{plan.otherFiles.length}</span>
                </div>
                <button type="button" className={`sync-item ${view.kind === "others" ? "on" : ""}`} onClick={() => setView({ kind: "others" })}>
                  <ChevronRight size={16} strokeWidth={1.9} />
                  <span className="sync-item-main">
                    <span className="sync-item-title">{plural(plan.otherFiles.length, "file")} merge automatically</span>
                    <span className="sync-item-sub">Changed on one side only</span>
                  </span>
                </button>
              </>
            )}
            {analysis && (
              <>
                <div className="sync-rail-label">
                  <span>Result</span>
                </div>
                <button type="button" className={`sync-item ${view.kind === "result" ? "on" : ""}`} disabled={!complete} onClick={() => setView({ kind: "result" })}>
                  <Eye size={16} strokeWidth={1.9} />
                  <span className="sync-item-main">
                    <span className="sync-item-title">Final check</span>
                    <span className="sync-item-sub">{complete ? plural(plan.files.length, "note") : "After all decisions"}</span>
                  </span>
                </button>
              </>
            )}
            <div className="sync-rail-foot">
              {analysis && (
                <button type="button" className="link-btn" disabled={busy || plan.unsupported.length > 0} onClick={() => void run("Comparing documents and evidence", api.analyzeSync)}>
                  Compare again with model
                </button>
              )}
              <button type="button" className="link-btn" disabled={busy} onClick={() => void run("Checking remote", api.prepareSync)}>
                Check remote again
              </button>
              {!analysis && <p>Recording is on hold. Sessions still enter the queue.</p>}
            </div>
          </aside>
          <div className="sync-pane">
            {errorBanner}
            {view.kind === "issue" && analysis?.issues[view.index] ? (
              <IssueView key={`${plan.id}-${view.index}`} issue={analysis.issues[view.index]} index={view.index} total={total} plan={plan} choice={choices[view.index]} disabled={busy} onChange={choose} />
            ) : view.kind === "result" && analysis ? (
              <ResultView plan={plan} preview={preview} previewError={previewError} />
            ) : view.kind === "file" ? (
              <section className="sync-pane-inner" aria-label={`Differences in ${view.path}`}>
                <span className="sync-over">Raw differences</span>
                <h1 className="sync-title">{fileName(view.path)}</h1>
                <Documents plan={plan} paths={new Set([view.path])} />
              </section>
            ) : view.kind === "others" ? (
              <section className="sync-pane-inner" aria-label="Files that merge automatically">
                <span className="sync-over">Other changes</span>
                <h1 className="sync-title">{plural(plan.otherFiles.length, "file")} merge automatically</h1>
                <p className="sync-lead">Each was changed on one side only. They are shared as whole files, including conversation copies that may hold more than the quoted sentences.</p>
                <div className="sync-file-list">
                  {plan.otherFiles.map((path) => (
                    <div key={path} className="sync-path mono">
                      {path}
                    </div>
                  ))}
                </div>
              </section>
            ) : (
              <section className="sync-pane-inner" aria-label="Sync review">
                <span className="sync-over">Sync review</span>
                <h1 className="sync-title">Both PCs changed the same notes</h1>
                <p className="sync-lead">
                  Since the last sync, this PC and another PC changed {plan.files.length === 1 ? "the same note" : `${plan.files.length} of the same notes`}. Lore prepared the merge without touching your files. Compare the two versions to decide what each note should say.
                </p>
                {plan.unsupported.length > 0 && (
                  <div className="banner" role="alert">
                    <TriangleAlert size={15} />
                    <span>These changes can’t be resolved here, such as conversation copies, deletions, or renames. Resolve them in an external Git tool, then check the remote again: {plan.unsupported.join(", ")}</span>
                  </div>
                )}
                {analysis ? (
                  <p className="sync-lead sync-summary">{analysis.summary}</p>
                ) : (
                  <div className="sync-card">
                    <span className="sync-card-icon">
                      <Sparkles size={18} strokeWidth={1.9} />
                    </span>
                    <div>
                      <h2>Compare with the recording model</h2>
                      <p>{snapshot.model} reads both versions and the relevant conversation passages, compares related notes in groups, then checks the results together. Larger reviews can take several minutes. Uses your account’s allowance.</p>
                      {resumable && <p role="status">{comparisonStatus}{!busy && " · Progress saved. Resume to continue."}</p>}
                      {!busy && comparison?.error && !error && <p role="alert">{comparison.error}</p>}
                      <div className="sync-card-actions">
                        <button type="button" className="btn primary" disabled={busy || plan.unsupported.length > 0} onClick={() => void run("Comparing documents and evidence", api.analyzeSync)}>
                          {resumable ? "Resume comparison" : "Compare with model"}
                        </button>
                        {plan.files.length > 0 && (
                          <button type="button" className="btn ghost" onClick={() => setView({ kind: "file", path: plan.files[0].path })}>
                            View raw differences
                          </button>
                        )}
                      </div>
                    </div>
                  </div>
                )}
                <p className="sync-note">
                  <Info size={15} strokeWidth={1.9} />
                  Nothing changes in your notes until you apply.
                </p>
              </section>
            )}
          </div>
        </div>
        <footer className="sync-actionbar">
          {busy ? (
            <span className="sync-busy" role="status">
              <LoaderCircle size={15} className="spin" />
              {pending === "Comparing documents and evidence" && comparisonStatus ? comparisonStatus : pending ?? "Sync will start after the current recording finishes."}
            </span>
          ) : analysis ? (
            <span className="sync-progress-bar">
              <span className="sync-track">
                <span style={{ width: `${total ? (decided / total) * 100 : 100}%` }} />
              </span>
              {decided} of {total} decided
            </span>
          ) : (
            <span className="sync-hint">Later keeps both versions as they are. Recording waits until you apply.</span>
          )}
          <span className="grow" />
          <button type="button" className="btn ghost" disabled={busy} onClick={close}>
            Later
          </button>
          {analysis &&
            (view.kind === "result" ? (
              <button type="button" className="btn primary" disabled={busy || !preview} onClick={() => preview && void run("Applying, indexing, and sharing", () => api.applySync(plan.id, selected, preview.token))}>
                Apply &amp; share
              </button>
            ) : (
              <button type="button" className="btn primary" disabled={!complete} onClick={() => setView({ kind: "result" })}>
                Check result <ChevronRight size={15} strokeWidth={2} />
              </button>
            ))}
        </footer>
      </>
    );
  } else if (plan) {
    const recovery = RECOVERY[plan.phase] ?? RECOVERY.applying;
    const rejected = /rejected|fetch first|non-fast-forward/i.test(plan.message ?? "");
    body = (
      <div className="sync-center">
        <div className="sync-column">
          {errorBanner}
          <span className="sync-big-icon warn">
            <CloudUpload size={22} strokeWidth={1.9} />
          </span>
          <h1 className="sync-title">{recovery.title}</h1>
          <p className="sync-lead">{recovery.lead(upstream)}</p>
          {plan.message && (
            <div className="sync-error-box">
              <div className="sync-error-head">Details</div>
              <pre role="status">{plan.message}</pre>
            </div>
          )}
          {plan.phase === "push_pending" && (
            <p className="sync-why">{rejected ? "Another PC shared changes in the meantime. Sync again to bring them in. If both sides changed the same notes, Lore asks you to compare them again." : "Check your network connection and Git sign-in, then retry."}</p>
          )}
          <div className="sync-card-actions">
            <button type="button" className="btn primary" disabled={busy} onClick={() => void (plan.phase === "push_pending" ? run("Checking remote", api.prepareSync) : run("Resuming sync", api.resumeSync))}>
              {busy ? pending ?? "Syncing…" : recovery.primary}
            </button>
            {recovery.secondary && (
              <button type="button" className="btn ghost" disabled={busy} onClick={() => void run("Resuming sync", api.resumeSync)}>
                {recovery.secondary}
              </button>
            )}
          </div>
          <p className="sync-safe">
            <CircleCheck size={16} strokeWidth={2} />
            {recovery.safe}
          </p>
        </div>
      </div>
    );
  } else {
    const failed = snapshot.summary.phase === "error";
    const last = snapshot.summary.lastSyncedAt;
    body = (
      <div className="sync-center">
        <div className="sync-column">
          {errorBanner}
          {notice && (
            <div className="sync-notice" role="status">
              {notice}
            </div>
          )}
          <span className={`sync-big-icon ${failed ? "warn" : last ? "ok" : ""}`}>{failed ? <TriangleAlert size={22} strokeWidth={1.9} /> : <Check size={22} strokeWidth={2} />}</span>
          <h1 className="sync-title">{failed ? "Sync needs attention" : last ? "Your PCs are in sync" : "Not synced yet"}</h1>
          <p className="sync-lead">
            {failed
              ? snapshot.summary.message ?? "The saved sync state could not be read."
              : last
                ? `Last synced ${relativePhrase(last, now)} with ${upstream}.`
                : snapshot.tracking
                  ? `Share this PC’s memory with ${upstream}.`
                  : "This memory folder has no upstream branch yet. Set one with Git to share memory between PCs."}
          </p>
          <div className="sync-group">
            <div className="sync-row">
              <div className="sync-row-main">
                <div className="sync-row-title">Auto-sync</div>
                <div className="sync-row-sub">{snapshot.enabled ? "On, every 5 minutes" : "Off"}</div>
              </div>
              <button type="button" className="link-btn" onClick={() => void api.openSettings()}>
                Change in Settings
              </button>
            </div>
            {snapshot.review ? (
              <div className="sync-row">
                <div className="sync-row-main">
                  <div className="sync-row-title">Not committed on this PC</div>
                  <div className="sync-row-sub">{plural(snapshot.review, "note")} changed. They are shared after you review and commit them.</div>
                </div>
                <button type="button" className="btn" onClick={() => void api.openReview()}>
                  Review &amp; commit
                </button>
              </div>
            ) : null}
          </div>
          <div className="sync-card-actions">
            <button type="button" className="btn primary" disabled={busy || !snapshot.tracking} onClick={() => void (failed ? run("Checking remote", api.prepareSync) : run("Syncing", api.syncNow))}>
              {busy ? (
                <>
                  <LoaderCircle size={15} className="spin" />
                  {pending ?? "Syncing…"}
                </>
              ) : failed ? (
                "Check remote again"
              ) : (
                "Sync now"
              )}
            </button>
          </div>
        </div>
      </div>
    );
  }

  return (
    <main className="sync-window">
      <header className="sync-head" data-tauri-drag-region>
        <span className="sync-brand" data-tauri-drag-region>
          <BrandMark size={22} />
        </span>
        <span className="sync-head-title" data-tauri-drag-region>
          Memory sync
        </span>
        {snapshot.tracking && (
          <span className="sync-head-sub mono" data-tauri-drag-region title={plan ? `Local ${plan.local.slice(0, 8)} · Remote ${plan.remote.slice(0, 8)}` : undefined}>
            {snapshot.tracking}
          </span>
        )}
        <span className={`sync-state ${state.tone}`}>{state.label}</span>
        <span className="grow" data-tauri-drag-region />
        <WindowClose onClose={close} disabled={busy} />
      </header>
      {body}
    </main>
  );
}
