import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

import type { DiffRow, EditableSettings, HookStatus, Job, Provider, ReviewSnapshot, Snapshot } from "./contract";

export type { Activity, Counts, Current, DiffRow, EditableSettings, HookStatus, Job, ReviewSnapshot, Settings, Snapshot } from "./contract";

export const getSnapshot = (): Promise<Snapshot> => invoke("get_snapshot");
export const getQueue = (): Promise<Job[]> => invoke("get_queue");
export const setPaused = (paused: boolean): Promise<void> => invoke("set_paused", { paused });
export const retryJob = (id: string): Promise<boolean> => invoke("retry_job", { id });
export const discardJob = (id: string): Promise<boolean> => invoke("discard_job", { id });
export const clearActivity = (): Promise<number> => invoke("clear_activity");
export const openReview = (): Promise<void> => invoke("open_review");
export const beginReview = (): Promise<void> => invoke("begin_review");
export const endReview = (): Promise<void> => invoke("end_review");
export const getReview = (): Promise<ReviewSnapshot> => invoke("get_review");
export const reviewDiff = (path: string): Promise<DiffRow[]> => invoke("review_diff", { path });
export const commitReview = (message: string, notes: string[]): Promise<string> => invoke("commit_review", { message, notes });
export const saveSettings = (settings: EditableSettings): Promise<void> => invoke("save_settings", { settings });
export const setAutostart = (enabled: boolean): Promise<void> => invoke("set_autostart", { enabled });
export const connectHook = (provider: Provider): Promise<HookStatus> => invoke("connect_hook", { provider });
export const disconnectHook = (provider: Provider): Promise<HookStatus> => invoke("disconnect_hook", { provider });
export const openNote = (path: string): Promise<void> => invoke("open_note", { path });
export const openFolder = (kind: "knowledge" | "data"): Promise<void> => invoke("open_folder", { kind });
export const hideWindow = (): Promise<void> => invoke("hide_window");

export const onStateChanged = (handler: () => void): Promise<UnlistenFn> => listen("state-changed", handler);
export const onPopoverShown = (handler: () => void): Promise<UnlistenFn> => listen("popover-shown", handler);
