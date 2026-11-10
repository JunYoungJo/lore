import { useEffect, useState } from "react";
import { ChevronLeft, FolderOpen, Moon, Pause, Play, Settings as SettingsIcon, Sun, TriangleAlert } from "lucide-react";

import * as api from "@/ipc/api";
import type { Snapshot } from "@/ipc/contract";
import { QueueView } from "@/components/QueueView";
import { SettingsFooter, SettingsView } from "@/components/SettingsView";
import { StatusView } from "@/components/StatusView";
import { useAppStore } from "@/store/appStore";
import { useTheme } from "@/theme";
import { BrandMark } from "@/ui/BrandMark";

function statusLine(snapshot: Snapshot): string {
  if (snapshot.current) return `Recording · ${snapshot.current.project}`;
  if (snapshot.syncing) return "Syncing";
  if (snapshot.sync?.blocked) return "Sync review needed · Recording on hold";
  if (snapshot.reviewing) return "Recording paused for review";
  if (snapshot.settings.paused) return "Paused";
  if (snapshot.counts.ready > 0) return `${snapshot.counts.ready} pending`;
  return "Idle";
}

export function App() {
  const snapshot = useAppStore((store) => store.snapshot);
  const view = useAppStore((store) => store.view);
  const error = useAppStore((store) => store.error);
  const refresh = useAppStore((store) => store.refresh);
  const togglePause = useAppStore((store) => store.togglePause);
  const showSettings = useAppStore((store) => store.showSettings);
  const showStatus = useAppStore((store) => store.showStatus);
  const [theme, toggleTheme] = useTheme();
  const [shown, setShown] = useState(0);

  useEffect(() => {
    void refresh();
    const unlisten = [
      api.onStateChanged(() => void refresh()),
      api.onPopoverShown(() => {
        setShown((count) => count + 1);
        void refresh();
      }),
    ];
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape") void api.hideWindow();
    };
    window.addEventListener("keydown", onKey);
    return () => {
      window.removeEventListener("keydown", onKey);
      unlisten.forEach((pending) => void pending.then((stop) => stop()));
    };
  }, [refresh]);

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
        {view === "settings" ? (
          <>
            <button type="button" className="icon-btn" title="Back" aria-label="Back" onClick={showStatus}>
              <ChevronLeft size={18} strokeWidth={1.9} />
            </button>
            <div className="head-text">
              <div className="head-title">Settings</div>
              <div className="head-sub">Lore {snapshot.version}</div>
            </div>
          </>
        ) : view === "queue" ? (
          <>
            <button type="button" className="icon-btn" title="Back" aria-label="Back" onClick={showStatus}>
              <ChevronLeft size={18} strokeWidth={1.9} />
            </button>
            <div className="head-text">
              <div className="head-title">Queue</div>
              <div className="head-sub" title={status}>
                <span className={dotClass} />
                {status}
              </div>
            </div>
          </>
        ) : (
          <>
            <BrandMark />
            <div className="head-text">
              <div className="head-title">Lore</div>
              <div className="head-sub" title={status}>
                <span className={dotClass} />
                {status}
              </div>
            </div>
            <button type="button" className="icon-btn" title={paused ? "Resume" : "Pause"} aria-label={paused ? "Resume" : "Pause"} onClick={() => void togglePause()}>
              {paused ? <Play size={16} strokeWidth={1.9} /> : <Pause size={16} strokeWidth={1.9} />}
            </button>
          </>
        )}
        <button type="button" className="icon-btn" title="Theme" aria-label="Toggle theme" onClick={toggleTheme}>
          {theme === "dark" ? <Sun size={16} strokeWidth={1.9} /> : <Moon size={16} strokeWidth={1.9} />}
        </button>
        {view === "status" && (
          <button type="button" className="icon-btn" title="Settings" aria-label="Settings" onClick={showSettings}>
            <SettingsIcon size={16} strokeWidth={1.9} />
          </button>
        )}
      </header>

      {snapshot.problem && (
        <div className="banner" role="alert">
          <TriangleAlert size={15} strokeWidth={1.9} style={{ flex: "none", marginTop: 1 }} />
          <span>{snapshot.problem}</span>
        </div>
      )}
      {error && view !== "settings" && (
        <div className="banner" role="alert">
          <TriangleAlert size={15} strokeWidth={1.9} style={{ flex: "none", marginTop: 1 }} />
          <span>{error}</span>
        </div>
      )}

      {view === "settings" ? <SettingsView snapshot={snapshot} /> : view === "queue" ? <QueueView snapshot={snapshot} /> : <StatusView snapshot={snapshot} />}

      {view === "settings" ? (
        <SettingsFooter snapshot={snapshot} />
      ) : (
        <footer className="foot">
          <button type="button" className="foot-btn" onClick={() => void api.openFolder("knowledge")}>
            <FolderOpen size={14} strokeWidth={1.9} />
            Memory folder
          </button>
          <span className="foot-path mono" title={snapshot.settings.knowledgeRoot}>
            {snapshot.settings.knowledgeRoot}
          </span>
        </footer>
      )}
    </main>
  );
}
