import { create } from "zustand";

import * as api from "@/ipc/api";
import type { Backend, EditableSettings, Job, Settings, Snapshot } from "@/ipc/contract";

export type View = "status" | "settings" | "queue";

export type QueueGroup = "recording" | "ready" | "waiting" | "failed";

export function editable({ knowledgeRoot, backend, models, efforts, modelTimeoutSecs, qmdTimeoutSecs, maxChars, notesBudget }: Settings): EditableSettings {
  return { knowledgeRoot, backend, models: { ...models }, efforts: { ...efforts }, modelTimeoutSecs, qmdTimeoutSecs, maxChars, notesBudget };
}

const cleanValue = (value: string | null) => (value?.trim() ? value.trim() : null);

const cleanPerCli = (values: Record<Backend, string | null>) => ({ codex: cleanValue(values.codex), claude: cleanValue(values.claude) });

interface AppStore {
  snapshot: Snapshot | null;
  view: View;
  error: string | null;
  queue: Job[] | null;
  queueFocus: QueueGroup;
  refresh: () => Promise<void>;
  loadQueue: () => Promise<void>;
  showSettings: () => void;
  showStatus: () => void;
  showQueue: (focus: QueueGroup) => void;
  saveSettings: (patch: Partial<EditableSettings>) => Promise<boolean>;
  togglePause: () => Promise<void>;
  retry: (id: string) => Promise<void>;
  discard: (id: string) => Promise<void>;
  clearActivity: () => Promise<void>;
  setAutostart: (enabled: boolean) => Promise<void>;
  setSyncEnabled: (enabled: boolean) => Promise<void>;
  syncNow: () => Promise<void>;
}

const message = (error: unknown) => (error instanceof Error ? error.message : String(error));

let refreshing: Promise<void> | null = null;
let loadingQueue: Promise<void> | null = null;
let saving: Promise<boolean> = Promise.resolve(true);

export const useAppStore = create<AppStore>((set, get) => {
  const attempt = async (action: () => Promise<unknown>) => {
    try {
      await action();
      await get().refresh();
    } catch (error) {
      set({ error: message(error) });
    }
  };

  return {
    snapshot: null,
    view: "status",
    error: null,
    queue: null,
    queueFocus: "ready",

    refresh: () => {
      if (get().view === "queue") void get().loadQueue();
      if (refreshing) return refreshing;
      refreshing = api
        .getSnapshot()
        .then((snapshot) => set({ snapshot, error: null }))
        .catch((error) => set({ error: message(error) }))
        .finally(() => {
          refreshing = null;
        });
      return refreshing;
    },

    loadQueue: () => {
      if (loadingQueue) return loadingQueue;
      loadingQueue = api
        .getQueue()
        .then((queue) => set({ queue }))
        .catch((error) => set({ error: message(error) }))
        .finally(() => {
          loadingQueue = null;
        });
      return loadingQueue;
    },

    showQueue: (focus) => {
      set({ view: "queue", queueFocus: focus, queue: null, error: null });
      void get().loadQueue();
    },

    showSettings: () => set({ view: "settings", error: null }),

    showStatus: () => set({ view: "status", error: null }),

    // Saves run in order and each starts from the last saved values: leaving a field saves it, and the click that left it often saves another setting.
    saveSettings: (patch) => {
      saving = saving.then(async () => {
        const settings = get().snapshot?.settings;
        if (!settings) return false;
        const next = { ...editable(settings), ...patch };
        const sent = { ...next, models: cleanPerCli(next.models), efforts: cleanPerCli(next.efforts) };
        set({ error: null });
        try {
          await api.saveSettings(sent);
          const current = get().snapshot;
          if (current) set({ snapshot: { ...current, settings: { ...current.settings, ...sent } } });
          void get().refresh();
          return true;
        } catch (error) {
          set({ error: message(error) });
          return false;
        }
      });
      return saving;
    },

    togglePause: () => attempt(() => api.setPaused(!(get().snapshot?.settings.paused ?? false))),
    retry: (id) => attempt(() => api.retryJob(id)),
    discard: (id) => attempt(() => api.discardJob(id)),
    clearActivity: () => attempt(() => api.clearActivity()),
    setAutostart: (enabled) => attempt(() => api.setAutostart(enabled)),
    setSyncEnabled: (enabled) => attempt(() => api.setSyncEnabled(enabled)),
    syncNow: () => attempt(() => api.syncNow()),
  };
});
