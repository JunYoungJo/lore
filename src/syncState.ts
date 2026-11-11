import type { Snapshot } from "@/ipc/contract";
import { relativePhrase } from "@/format";

export interface SyncState {
  text: string;
  tone: "" | "ok" | "warn";
  opensWindow: boolean;
}

export function syncState(snapshot: Snapshot, now: number): SyncState {
  const { sync, settings } = snapshot;
  if (snapshot.syncing) return { text: "Syncing…", tone: "", opensWindow: false };
  if (sync?.phase === "failing") return { text: "Sync needs attention", tone: "warn", opensWindow: true };
  // Items never hold sync: the window only shows them, and sync keeps running.
  if (sync?.items) return { text: `${sync.items} to check`, tone: "warn", opensWindow: true };
  if (sync?.lastSyncedAt) return { text: `Synced ${relativePhrase(sync.lastSyncedAt, now)}${settings.syncEnabled ? "" : " · auto off"}`, tone: "ok", opensWindow: false };
  return { text: settings.syncEnabled ? "Not synced yet" : "Sync off", tone: "", opensWindow: false };
}
