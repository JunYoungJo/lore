import { create } from "zustand";

import * as api from "@/ipc/api";
import type { Provider, Settings, Snapshot } from "@/ipc/contract";

export type View = "status" | "settings";

interface AppStore {
  snapshot: Snapshot | null;
  view: View;
  draft: Settings | null;
  error: string | null;
  saving: boolean;
  refresh: () => Promise<void>;
  showSettings: () => void;
  showStatus: () => void;
  editDraft: (patch: Partial<Settings>) => void;
  saveDraft: () => Promise<void>;
  togglePause: () => Promise<void>;
  retry: (id: string) => Promise<void>;
  setAutostart: (enabled: boolean) => Promise<void>;
  connecting: Provider | null;
  connectHook: (provider: Provider) => Promise<void>;
  disconnectHook: (provider: Provider) => Promise<void>;
}

const message = (error: unknown) => (error instanceof Error ? error.message : String(error));

let refreshing: Promise<void> | null = null;

export const useAppStore = create<AppStore>((set, get) => ({
  snapshot: null,
  view: "status",
  draft: null,
  error: null,
  saving: false,

  refresh: () => {
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

  showSettings: () => {
    const settings = get().snapshot?.settings ?? null;
    set({ view: "settings", draft: settings ? structuredClone(settings) : null, error: null });
  },

  showStatus: () => set({ view: "status", draft: null, error: null }),

  editDraft: (patch) => {
    const draft = get().draft;
    if (draft) set({ draft: { ...draft, ...patch } });
  },

  saveDraft: async () => {
    const draft = get().draft;
    if (!draft) return;
    set({ saving: true, error: null });
    try {
      await api.saveSettings({ ...draft, model: draft.model?.trim() ? draft.model.trim() : null });
      set({ view: "status", draft: null });
      await get().refresh();
    } catch (error) {
      set({ error: message(error) });
    } finally {
      set({ saving: false });
    }
  },

  togglePause: async () => {
    const paused = get().snapshot?.settings.paused ?? false;
    try {
      await api.setPaused(!paused);
      await get().refresh();
    } catch (error) {
      set({ error: message(error) });
    }
  },

  retry: async (id) => {
    try {
      await api.retryJob(id);
      await get().refresh();
    } catch (error) {
      set({ error: message(error) });
    }
  },

  setAutostart: async (enabled) => {
    try {
      await api.setAutostart(enabled);
      await get().refresh();
    } catch (error) {
      set({ error: message(error) });
    }
  },

  connecting: null,
  connectHook: async (provider) => {
    set({ connecting: provider, error: null });
    try {
      await api.connectHook(provider);
      await get().refresh();
    } catch (error) {
      set({ error: message(error) });
    } finally {
      set({ connecting: null });
    }
  },

  disconnectHook: async (provider) => {
    set({ connecting: provider, error: null });
    try {
      await api.disconnectHook(provider);
      await get().refresh();
    } catch (error) {
      set({ error: message(error) });
    } finally {
      set({ connecting: null });
    }
  },
}));
