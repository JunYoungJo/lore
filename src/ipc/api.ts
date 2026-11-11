import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

import type { DiffRow, EditableSettings, Job, Provider, ReviewSnapshot, Snapshot } from "./contract";

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
export const markReviewed = (notes: string[]): Promise<number> => invoke("mark_reviewed", { notes });
export const saveSettings = (settings: EditableSettings): Promise<void> => invoke("save_settings", { settings });
export const setAutostart = (enabled: boolean): Promise<void> => invoke("set_autostart", { enabled });
export const openNote = (path: string): Promise<void> => invoke("open_note", { path });
export const openFolder = (kind: "knowledge" | "data"): Promise<void> => invoke("open_folder", { kind });
export const hideWindow = (): Promise<void> => invoke("hide_window");
export const openSettings = (): Promise<void> => invoke("open_settings");
export const checkAppUpdate = (): Promise<void> => invoke("check_app_update");
export const installAppUpdate = (): Promise<void> => invoke("install_app_update");

export const onStateChanged = (handler: () => void): Promise<UnlistenFn> => listen("state-changed", handler);
export const onSyncComparisonProgress = (handler: (progress: import("./contract").SyncComparisonProgress) => void): Promise<UnlistenFn> => listen<import("./contract").SyncComparisonProgress>("sync-comparison-progress", (event) => handler(event.payload));
export const onPopoverShown = (handler: () => void): Promise<UnlistenFn> => listen("popover-shown", handler);
export const onShowSettings = (handler: () => void): Promise<UnlistenFn> => listen("show-settings", handler);

export const openSync = (): Promise<void> => invoke("open_sync");
export const getSync = (): Promise<import("./contract").SyncSnapshot> => invoke("get_sync");
export const setSyncEnabled = (enabled: boolean): Promise<void> => invoke("set_sync_enabled", { enabled });
export const getSyncItem = (id: string): Promise<import("./contract").SyncItemView> => invoke("get_sync_item", { id });
export const previewSyncItem = (id: string, option: number, customText: string | null): Promise<Record<string, import("./contract").DiffRow[]>> => invoke("preview_sync_item", { id, option, customText });
export const resolveSyncItem = (id: string, option: number, customText: string | null): Promise<void> => invoke("resolve_sync_item", { id, option, customText });
export const dismissSyncItem = (id: string): Promise<void> => invoke("dismiss_sync_item", { id });
export const retrySyncComparison = (): Promise<void> => invoke("retry_sync_comparison");
export const skipSyncComparison = (): Promise<void> => invoke("skip_sync_comparison");
export const syncNow = (): Promise<void> => invoke("sync_now");
export const deferSync = (): Promise<void> => invoke("defer_sync");

export const openSetup = (): Promise<void> => invoke("open_setup");
export const openIntegrations = (section?: "folder"): Promise<void> => invoke("open_integrations", { section: section ?? null });
export const onIntegrationsSection = (handler: (section: string) => void): Promise<UnlistenFn> => listen<string>("integrations-section", (event) => handler(event.payload));
export const setupCheckTools = (): Promise<import("./contract").ToolChecks> => invoke("setup_check_tools");
export const setupPickFolder = (): Promise<string | null> => invoke("setup_pick_folder");
export const getSetup = (): Promise<import("./contract").SetupSnapshot> => invoke("get_setup");
export const saveSetupProgress = (progress: import("./contract").SetupProgress): Promise<void> => invoke("save_setup_progress", { progress });
export const setupDetect = (): Promise<void> => invoke("setup_detect");
export const setupRepository = (path: string, create: boolean): Promise<void> => invoke("setup_repository", { path, create });
export const setupConnect = (provider: Provider, token: string): Promise<void> => invoke("setup_connect", { provider, token });
export const setupDisconnect = (provider: Provider): Promise<string[]> => invoke("setup_disconnect", { provider });
export const setupIndex = (): Promise<void> => invoke("setup_index");
export const setupEmbedModel = (model: string): Promise<void> => invoke("setup_embed_model", { model });
export const setupBeginTest = (): Promise<void> => invoke("setup_begin_test");
const commandArgs = ({ task, remote = "", path = "", query = "" }: import("./contract").SetupCommand) => ({ task, remote, path, query });
export const setupPrepare = (command: import("./contract").SetupCommand): Promise<import("./contract").PreparedCommand> => invoke("setup_prepare", commandArgs(command));
export const setupRun = (command: import("./contract").SetupCommand, token: string, cols: number, rows: number): Promise<void> => invoke("setup_run", { request: { ...commandArgs(command), token, cols, rows } });
export const setupOpenDocs = (topic: string): Promise<void> => invoke("setup_open_docs", { topic });
export const setupTerminalRead = (after: number): Promise<import("./contract").TerminalOutput> => invoke("setup_terminal_read", { after });
export const setupTerminalInput = (data: string): Promise<void> => invoke("setup_terminal_input", { data });
export const setupTerminalResize = (cols: number, rows: number): Promise<void> => invoke("setup_terminal_resize", { cols, rows });
export const setupTerminalStop = (): Promise<void> => invoke("setup_terminal_stop");
