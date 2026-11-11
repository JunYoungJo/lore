import { useEffect, useState } from "react";
import { ChevronLeft, FolderOpen, Pause, Play, RefreshCw, Settings as SettingsIcon, TriangleAlert } from "lucide-react";

import * as api from "@/ipc/api";
import type { Snapshot } from "@/ipc/contract";
import { QueueView } from "@/components/QueueView";
import { SettingsView } from "@/components/SettingsView";
import { StatusView } from "@/components/StatusView";
import { useAppStore } from "@/store/appStore";
import { syncState } from "@/syncState";
import { useTheme } from "@/theme";
import { BrandMark } from "@/ui/BrandMark";
import { useNow } from "@/useNow";

function statusLine(snapshot: Snapshot): string {
  if (snapshot.current) return `Recording · ${snapshot.current.project}`;
  if (snapshot.syncing) return "Syncing";
  if (snapshot.sync?.blocked) return "Sync review needed · Recording on hold";
  if (snapshot.reviewing) return "Recording paused for review";
  if (snapshot.settings.paused) return "Paused";
  if (snapshot.counts.ready > 0) return `${snapshot.counts.ready} pending`;
  return "Idle";
}

function SyncStatus({ snapshot }: { snapshot: Snapshot }) {
  const now = useNow(15_000);
  const syncNow = useAppStore((store) => store.syncNow);
  const state = syncState(snapshot, now);
  const label = state.opensWindow ? "Open sync" : "Sync now";
  return (
    <>
      <span className={`foot-sync ${state.tone}`}>
        <span className="sync-dot" aria-hidden="true" />
        {state.text}
      </span>
      <button type="button" className="icon-btn" title={label} aria-label={label} disabled={snapshot.syncing} onClick={() => void (state.opensWindow ? api.openSync() : syncNow())}>
        <RefreshCw size={15} strokeWidth={1.9} className={snapshot.syncing ? "spin" : undefined} />
      </button>
    </>
  );
}

export function App() {
  const snapshot = useAppStore((store) => store.snapshot);
  const view = useAppStore((store) => store.view);
  const error = useAppStore((store) => store.error);
  const refresh = useAppStore((store) => store.refresh);
  const togglePause = useAppStore((store) => store.togglePause);
  const showSettings = useAppStore((store) => store.showSettings);
  const showStatus = useAppStore((store) => store.showStatus);
  const [shown, setShown] = useState(0);
  useTheme();

  useEffect(() => {
    void refresh();
    const unlisten = [
      api.onStateChanged(() => void refresh()),
      api.onPopoverShown(() => {
        setShown((count) => count + 1);
        void refresh();
      }),
      api.onShowSettings(showSettings),
    ];
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape") void api.hideWindow();
    };
    window.addEventListener("keydown", onKey);
    return () => {
      window.removeEventListener("keydown", onKey);
      unlisten.forEach((pending) => void pending.then((stop) => stop()));
    };
  }, [refresh, showSettings]);

  if (!snapshot) {
    return (
      <main className="pop">
        <div className="empty">{error ?? "Loading status…"}</div>
      </main>
    );
  }

  const paused = snapshot.settings.paused;
  const dotClass = snapshot.current ? "dot working" : paused || snapshot.reviewing ? "dot paused" : "dot";
  const status = statusLine(snapshot);

  return (
    <main key={shown} className="pop anim-traypop">
      <header className="head">
        {view === "status" ? (
          <>
            <BrandMark />
            <div className="head-text">
              <div className="head-title">Lore</div>
            </div>
            <button type="button" className="icon-btn" title={paused ? "Resume" : "Pause"} aria-label={paused ? "Resume" : "Pause"} onClick={() => void togglePause()}>
              {paused ? <Play size={16} strokeWidth={1.9} /> : <Pause size={16} strokeWidth={1.9} />}
            </button>
            <button type="button" className="icon-btn" title="Settings" aria-label="Settings" onClick={showSettings}>
              <SettingsIcon size={16} strokeWidth={1.9} />
            </button>
          </>
        ) : (
          <>
            <button type="button" className="icon-btn" title="Back" aria-label="Back" onClick={showStatus}>
              <ChevronLeft size={18} strokeWidth={1.9} />
            </button>
            <div className="head-text">
              <div className="head-title">{view === "settings" ? "Settings" : "Queue"}</div>
              {view === "settings" ? (
                <div className="head-sub">Lore {snapshot.version}</div>
              ) : (
                <div className="head-sub" title={status}>
                  <span className={dotClass} />
                  {status}
                </div>
              )}
            </div>
          </>
        )}
      </header>

      {snapshot.problem && (
        <div className="banner" role="alert">
          <TriangleAlert size={15} strokeWidth={1.9} style={{ flex: "none", marginTop: 1 }} />
          <span>{snapshot.problem}</span>
        </div>
      )}
      {error && (
        <div className="banner" role="alert">
          <TriangleAlert size={15} strokeWidth={1.9} style={{ flex: "none", marginTop: 1 }} />
          <span>{error}</span>
        </div>
      )}

      {view === "settings" ? <SettingsView snapshot={snapshot} /> : view === "queue" ? <QueueView snapshot={snapshot} /> : <StatusView snapshot={snapshot} />}

      {view !== "settings" && (
        <footer className="foot">
          <button type="button" className="foot-btn" title={snapshot.settings.knowledgeRoot} onClick={() => void api.openFolder("knowledge")}>
            <FolderOpen size={14} strokeWidth={1.9} />
            Memory folder
          </button>
          <SyncStatus snapshot={snapshot} />
        </footer>
      )}
    </main>
  );
}
