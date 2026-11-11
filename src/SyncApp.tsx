import { useCallback, useEffect, useRef, useState } from "react";
import { Check, CircleCheck, FileText, GitCompare, LoaderCircle, Sparkles, TriangleAlert } from "lucide-react";

import * as api from "@/ipc/api";
import type { DiffRow, SyncItem, SyncItemFile, SyncItemView, SyncSnapshot } from "@/ipc/contract";
import { DiffView } from "@/components/DiffView";
import { fileName, relativePhrase } from "@/format";
import { useTheme } from "@/theme";
import { BrandMark } from "@/ui/BrandMark";
import { WindowClose } from "@/ui/WindowClose";
import { useNow } from "@/useNow";

const SIDE: Record<string, string> = { base: "Base", local: "This PC", remote: "Other PCs", merged: "Merged note" };
const KIND: Record<SyncItem["kind"], string> = { merged: "Both PCs’ lines kept", kept: "Edited note kept", decision: "Notes disagree" };

const plural = (count: number, one: string, many = `${one}s`) => `${count} ${count === 1 ? one : many}`;
const upstreamOf = (tracking: string | null) => tracking?.split("↔ ")[1] ?? "the upstream branch";

function headerState(snapshot: SyncSnapshot, busy: boolean): { label: string; tone: "" | "ok" | "warn" } {
  if (busy) return { label: "Syncing…", tone: "" };
  if (snapshot.summary.phase === "failing") return { label: "Needs attention", tone: "warn" };
  if (snapshot.summary.phase === "retrying") return { label: "Retrying", tone: "" };
  if (snapshot.items.length > 0) return { label: `${snapshot.items.length} to check`, tone: "warn" };
  return snapshot.summary.lastSyncedAt ? { label: "Up to date", tone: "ok" } : { label: "Not synced yet", tone: "" };
}

function Versions({ files }: { files: SyncItemFile[] }) {
  return (
    <>
      {files.map((file) => (
        <section key={file.path} className="sync-docs">
          <div className="sync-path mono">{file.path}</div>
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

interface Choice {
  option: number;
  customText: string | null;
}

function DecisionView({ view, choice, preview, previewError, disabled, onChange }: { view: SyncItemView; choice: Choice | null; preview: Record<string, DiffRow[]> | null; previewError: string | null; disabled: boolean; onChange: (choice: Choice) => void }) {
  const issue = view.issue!;
  const option = choice ? issue.options[choice.option] : undefined;
  const target = issue.options[0]?.edits[0]?.path;
  return (
    <section className="sync-pane-inner" aria-label={issue.title}>
      <span className="sync-over">{KIND.decision}</span>
      <h1 className="sync-title">{issue.title}</h1>
      <p className="sync-lead">{issue.explanation}</p>
      <div className="sync-two sync-evidence-grid">
        {issue.evidence.map((evidence, index) => {
          const source = view.sources.find((item) => item.id === evidence.source);
          return (
            <div className="sync-ev" key={`${evidence.source}-${index}`}>
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
        {issue.options.map((item, index) => (
          <label key={index} className={`sync-opt ${choice?.option === index ? "on" : ""}`}>
            <input type="radio" name={`sync-item-${view.item.id}`} checked={choice?.option === index} onChange={() => onChange({ option: index, customText: null })} />
            <span>
              <strong>{item.label}</strong>
              <small>{item.description}</small>
            </span>
          </label>
        ))}
      </fieldset>
      {choice && option?.edits.length === 1 && (
        <label className="sync-check">
          <input type="checkbox" disabled={disabled} checked={choice.customText !== null} onChange={(event) => onChange({ ...choice, customText: event.target.checked ? option.edits[0].text : null })} />
          Edit the selected wording
        </label>
      )}
      {choice && choice.customText !== null && (
        <label className="sync-custom">
          Final wording
          <textarea className="input" rows={5} value={choice.customText} disabled={disabled} onChange={(event) => onChange({ ...choice, customText: event.target.value })} />
        </label>
      )}
      {previewError && (
        <div className="banner" role="alert">
          <TriangleAlert size={15} />
          <span>{previewError}</span>
        </div>
      )}
      {preview && Object.entries(preview).map(([path, rows]) => <DiffView key={path} path={path} rows={rows} labels={["Note now", "After applying"]} />)}
      <details className="sync-compare">
        <summary>Compare base, this PC and other PCs</summary>
        <Versions files={view.files} />
      </details>
    </section>
  );
}

function MergedView({ view }: { view: SyncItemView }) {
  const file = view.files[0];
  return (
    <section className="sync-pane-inner" aria-label={fileName(view.item.path)}>
      <span className="sync-over">{KIND[view.item.kind]}</span>
      <h1 className="sync-title">{fileName(view.item.path)}</h1>
      <p className="sync-lead">{view.item.detail}</p>
      <p className="sync-note">
        <CircleCheck size={15} strokeWidth={1.9} />
        The merged note is already saved and shared. To change it, edit the note; the edit syncs like any other.
      </p>
      {file && <DiffView path={file.path} rows={file.rows} labels={["This PC before sync", "Merged note"]} />}
      <details className="sync-compare">
        <summary>Compare base, this PC and other PCs</summary>
        <Versions files={view.files} />
      </details>
    </section>
  );
}

export function SyncApp() {
  useTheme();
  const now = useNow(15_000);
  const [snapshot, setSnapshot] = useState<SyncSnapshot | null>(null);
  const [pending, setPending] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [selected, setSelected] = useState<string | null>(null);
  const [view, setView] = useState<SyncItemView | null>(null);
  const [choice, setChoice] = useState<Choice | null>(null);
  const [preview, setPreview] = useState<Record<string, DiffRow[]> | null>(null);
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
    const stopProgress = api.onSyncComparisonProgress((comparison) => {
      if (alive.current) setSnapshot((current) => (current ? { ...current, comparison } : current));
    });
    return () => {
      alive.current = false;
      void stop.then((unlisten) => unlisten());
      void stopProgress.then((unlisten) => unlisten());
    };
  }, [load]);

  const items = snapshot?.items ?? [];
  const current = items.find((item) => item.id === selected)?.id ?? items[0]?.id ?? null;
  useEffect(() => {
    setView(null);
    setChoice(null);
    if (!current) return;
    let active = true;
    api.getSyncItem(current).then(
      (next) => active && setView(next),
      (failure) => active && setError(String(failure)),
    );
    return () => {
      active = false;
    };
  }, [current]);

  const option = choice?.option;
  const customText = choice?.customText ?? null;
  useEffect(() => {
    setPreview(null);
    setPreviewError(null);
    if (!current || option === undefined) return;
    let active = true;
    const timer = window.setTimeout(() => {
      api.previewSyncItem(current, option, customText).then(
        (next) => active && setPreview(next),
        (failure) => active && setPreviewError(String(failure)),
      );
    }, 200);
    return () => {
      active = false;
      window.clearTimeout(timer);
    };
  }, [current, option, customText]);

  const run = async (label: string, action: () => Promise<unknown>) => {
    setPending(label);
    setError(null);
    try {
      await action();
      await load();
    } catch (failure) {
      if (alive.current) setError(String(failure));
    } finally {
      if (alive.current) setPending(null);
    }
  };
  const close = () => void api.deferSync().catch((failure) => setError(String(failure)));

  if (!snapshot) {
    return (
      <main className="sync-window">
        <div className="empty">{error ?? "Loading…"}</div>
      </main>
    );
  }

  const busy = pending !== null || snapshot.busy;
  const state = headerState(snapshot, busy);
  const upstream = upstreamOf(snapshot.tracking);
  const errorBanner = error && (
    <div className="banner" role="alert">
      <TriangleAlert size={15} />
      <span>{error}</span>
    </div>
  );
  const comparison = snapshot.comparison;
  const comparing = snapshot.summary.comparing && (
    <p className="sync-busy" role="status">
      <LoaderCircle size={15} className="spin" />
      {snapshot.model} is comparing the notes both PCs changed{comparison ? (comparison.stage === "checking" ? ` · checking ${comparison.total} groups together` : ` · ${comparison.completed} of ${plural(comparison.total, "group")}`) : ""}
    </p>
  );
  const comparisonFailed = snapshot.comparisonError && (
    <div className="sync-error-box">
      <div className="sync-error-head">The comparison could not finish. Your notes are synced either way.</div>
      <pre>{snapshot.comparisonError}</pre>
      <div className="sync-card-actions">
        <button type="button" className="btn" disabled={busy} onClick={() => void run("Starting the comparison", api.retrySyncComparison)}>
          Try again
        </button>
        <button type="button" className="btn ghost" disabled={busy} onClick={() => void run("Skipping the comparison", api.skipSyncComparison)}>
          Skip
        </button>
      </div>
    </div>
  );

  let body;
  if (items.length > 0) {
    const item = items.find((entry) => entry.id === current)!;
    body = (
      <>
        <div className="sync-main">
          <aside className="sync-rail" aria-label="Notes to check">
            <div className="sync-rail-label">
              <span>To check</span>
              <span>{items.length}</span>
            </div>
            {items.map((entry) => (
              <button key={entry.id} type="button" className={`sync-item ${entry.id === current ? "on" : ""}`} onClick={() => setSelected(entry.id)}>
                {entry.kind === "decision" ? <GitCompare size={16} strokeWidth={1.9} /> : <FileText size={16} strokeWidth={1.9} />}
                <span className="sync-item-main">
                  <span className="sync-item-title">{entry.kind === "decision" ? entry.title : fileName(entry.path)}</span>
                  <span className="sync-item-sub">
                    {KIND[entry.kind]}
                    {entry.kind === "decision" ? ` · ${fileName(entry.path)}` : ""}
                  </span>
                </span>
              </button>
            ))}
            <div className="sync-rail-foot">
              {comparing}
              {comparisonFailed}
              <button type="button" className="link-btn" disabled={busy || !snapshot.tracking} onClick={() => void run("Syncing", api.syncNow)}>
                Sync now
              </button>
              <p>{snapshot.summary.lastSyncedAt ? `Last synced ${relativePhrase(snapshot.summary.lastSyncedAt, now)}.` : "Not synced yet."}</p>
            </div>
          </aside>
          <div className="sync-pane">
            {errorBanner}
            {!view || view.item.id !== current ? (
              <div className="sync-progress" role="status">
                <LoaderCircle size={16} className="spin" />
                Loading…
              </div>
            ) : view.issue ? (
              <DecisionView key={current} view={view} choice={choice} preview={preview} previewError={previewError} disabled={busy} onChange={setChoice} />
            ) : (
              <MergedView key={current} view={view} />
            )}
          </div>
        </div>
        <footer className="sync-actionbar">
          {busy ? (
            <span className="sync-busy" role="status">
              <LoaderCircle size={15} className="spin" />
              {pending ?? "Syncing…"}
            </span>
          ) : (
            <span className="sync-hint">Nothing waits on these. Recording and sync continue.</span>
          )}
          <span className="grow" />
          <button type="button" className="btn ghost" onClick={close}>
            Later
          </button>
          {item.kind === "decision" ? (
            <>
              <button type="button" className="btn" disabled={busy} onClick={() => void run("Removing the item", () => api.dismissSyncItem(item.id))}>
                Keep the note as it is
              </button>
              <button type="button" className="btn primary" disabled={busy || !choice || !preview} onClick={() => choice && void run("Applying and sharing", () => api.resolveSyncItem(item.id, choice.option, choice.customText))}>
                Apply
              </button>
            </>
          ) : (
            <>
              <button type="button" className="btn" onClick={() => void api.openNote(item.path).catch((failure) => setError(String(failure)))}>
                Open note
              </button>
              <button type="button" className="btn primary" disabled={busy} onClick={() => void run("Removing the item", () => api.dismissSyncItem(item.id))}>
                <Check size={15} strokeWidth={2} /> Mark as checked
              </button>
            </>
          )}
        </footer>
      </>
    );
  } else {
    const failing = snapshot.summary.phase === "failing";
    const last = snapshot.summary.lastSyncedAt;
    body = (
      <div className="sync-center">
        <div className="sync-column">
          {errorBanner}
          <span className={`sync-big-icon ${failing ? "warn" : last ? "ok" : ""}`}>{failing ? <TriangleAlert size={22} strokeWidth={1.9} /> : <Check size={22} strokeWidth={2} />}</span>
          <h1 className="sync-title">{failing ? "Sync is not getting through" : last ? "Your PCs are in sync" : "Not synced yet"}</h1>
          <p className="sync-lead">
            {failing
              ? "Recording continues. Lore keeps retrying in the background."
              : last
                ? `Last synced ${relativePhrase(last, now)} with ${upstream}.`
                : snapshot.tracking
                  ? `Share this PC’s memory with ${upstream}.`
                  : "This memory folder has no upstream branch yet. Set one with Git to share memory between PCs."}
          </p>
          {failing && snapshot.summary.message && (
            <div className="sync-error-box">
              <div className="sync-error-head">Details</div>
              <pre>{snapshot.summary.message}</pre>
            </div>
          )}
          {comparing}
          {comparisonFailed}
          <div className="sync-group">
            <div className="sync-row">
              <div className="sync-row-main">
                <div className="sync-row-title">Auto-sync</div>
                <div className="sync-row-sub">{snapshot.enabled ? "On. Notes are committed and shared after each recording." : "Off. Notes are committed after each recording and shared when you sync."}</div>
              </div>
              <button type="button" className="link-btn" onClick={() => void api.openSettings()}>
                Change in Settings
              </button>
            </div>
            {snapshot.review ? (
              <div className="sync-row">
                <div className="sync-row-main">
                  <div className="sync-row-title">Review</div>
                  <div className="sync-row-sub">{plural(snapshot.review, "note")} recorded since your last review. They are already shared; review them when you want.</div>
                </div>
                <button type="button" className="btn" onClick={() => void api.openReview()}>
                  Review
                </button>
              </div>
            ) : null}
            {snapshot.indexPending && (
              <div className="sync-row">
                <div className="sync-row-main">
                  <div className="sync-row-title">Search not updated</div>
                  <div className="sync-row-sub">Notes from other PCs are saved, but search on this PC has not picked them up. The next sync tries again.</div>
                </div>
              </div>
            )}
          </div>
          <div className="sync-card-actions">
            <button type="button" className="btn primary" disabled={busy || !snapshot.tracking} onClick={() => void run("Syncing", api.syncNow)}>
              {busy ? (
                <>
                  <LoaderCircle size={15} className="spin" />
                  {pending ?? "Syncing…"}
                </>
              ) : failing ? (
                "Try again"
              ) : (
                <>
                  <Sparkles size={15} strokeWidth={1.9} /> Sync now
                </>
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
          <span className="sync-head-sub mono" data-tauri-drag-region>
            {snapshot.tracking}
          </span>
        )}
        <span className={`sync-state ${state.tone}`}>{state.label}</span>
        <span className="grow" data-tauri-drag-region />
        <WindowClose onClose={close} />
      </header>
      {body}
    </main>
  );
}
