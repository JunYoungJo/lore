import { useState } from "react";

import * as api from "@/ipc/api";
import type { Snapshot } from "@/ipc/contract";
import { useAppStore } from "@/store/appStore";

export function AppUpdates({ snapshot }: { snapshot: Snapshot }) {
  const refresh = useAppStore((store) => store.refresh);
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const update = snapshot.update;
  const busy = pending || ["checking", "downloading", "installing"].includes(update.phase);
  const blocked = Boolean(snapshot.current || snapshot.syncing || snapshot.reviewing);
  const installing = update.phase === "downloading" || update.phase === "installing";
  const text = update.phase === "checking" ? "Checking for updates…"
    : update.phase === "downloading" ? `Downloading${update.total ? ` · ${Math.min(100, Math.round(update.downloaded / update.total * 100))}%` : "…"}`
    : update.phase === "installing" ? "Installing and restarting…"
    : update.version ? `Version ${update.version} available`
    : update.phase === "current" ? "You’re up to date."
    : "Check for a new version on GitHub.";

  const run = async (action: () => Promise<void>) => {
    setPending(true); setError(null);
    try { await action(); }
    catch (reason) { setError(reason instanceof Error ? reason.message : String(reason)); }
    finally { setPending(false); await refresh(); }
  };

  return <>
    <div className="eyebrow">App updates</div>
    <section className="group" aria-label="App updates">
      <div className="set-row">
        <div className="set-main">
          <div className="set-title">Lore {snapshot.version}</div>
          <div className="set-sub" role="status">{text}</div>
        </div>
        {!update.version && <button type="button" className="pill ghost" disabled={busy} onClick={() => void run(api.checkAppUpdate)}>Check now</button>}
      </div>
      {update.version && !installing && <>
        {update.notes && <div className="update-notes">{update.notes}</div>}
        <div className="set-hint">{blocked ? "Finish recording or sync and close the memory review window before updating." : "Installing will close Lore and restart it. Your memory and settings are kept."}</div>
        <div className="update-actions">
          <button type="button" className="pill" disabled={busy || blocked} onClick={() => void run(api.installAppUpdate)}>Install &amp; restart</button>
          <button type="button" className="pill ghost" disabled={busy} onClick={() => void run(api.checkAppUpdate)}>Check again</button>
        </div>
      </>}
      {(error || update.error) && <div className="set-hint update-error" role="alert">{error || update.error}</div>}
    </section>
  </>;
}
