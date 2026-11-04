import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

import type { HookStatus, Provider, Settings, Snapshot } from "./contract";

export type { Activity, Counts, Current, HookStatus, Job, Settings, Snapshot } from "./contract";

export const getSnapshot = (): Promise<Snapshot> => invoke("get_snapshot");
export const setPaused = (paused: boolean): Promise<void> => invoke("set_paused", { paused });
export const retryJob = (id: string): Promise<boolean> => invoke("retry_job", { id });
export const saveSettings = (settings: Settings): Promise<void> => invoke("save_settings", { settings });
export const setAutostart = (enabled: boolean): Promise<void> => invoke("set_autostart", { enabled });
export const connectHook = (provider: Provider): Promise<HookStatus> => invoke("connect_hook", { provider });
export const disconnectHook = (provider: Provider): Promise<HookStatus> => invoke("disconnect_hook", { provider });
export const openNote = (path: string): Promise<void> => invoke("open_note", { path });
export const openFolder = (kind: "knowledge" | "data"): Promise<void> => invoke("open_folder", { kind });
export const hideWindow = (): Promise<void> => invoke("hide_window");

export const onStateChanged = (handler: () => void): Promise<UnlistenFn> => listen("state-changed", handler);
export const onPopoverShown = (handler: () => void): Promise<UnlistenFn> => listen("popover-shown", handler);
