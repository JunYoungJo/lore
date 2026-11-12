import { act, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { EditableSettings, Job, Snapshot } from "@/ipc/contract";

const api = vi.hoisted(() => ({
  getSnapshot: vi.fn(),
  getQueue: vi.fn(),
  setPaused: vi.fn(),
  retryJob: vi.fn(),
  discardJob: vi.fn(),
  saveSettings: vi.fn(),
  setAutostart: vi.fn(),
  setSyncEnabled: vi.fn(),
  syncNow: vi.fn(),
  clearActivity: vi.fn(),
  getCleanupFolders: vi.fn(),
  startCleanup: vi.fn(),
  openReview: vi.fn(),
  openSync: vi.fn(),
  openNote: vi.fn(),
  openFolder: vi.fn(),
  hideWindow: vi.fn(),
  openSetup: vi.fn(),
  openIntegrations: vi.fn(),
  checkAppUpdate: vi.fn(),
  installAppUpdate: vi.fn(),
  onStateChanged: vi.fn(() => Promise.resolve(() => {})),
  onPopoverShown: vi.fn(() => Promise.resolve(() => {})),
  onShowSettings: vi.fn((_handler: () => void) => Promise.resolve(() => {})),
}));

vi.mock("@/ipc/api", () => api);

import { App } from "@/App";
import { useAppStore } from "@/store/appStore";
import { setThemePreference } from "@/theme";

const CLI = "C:/app/lore-cli.exe";
const NOW = 1_800_000_600;

function snapshot(overrides: Partial<Snapshot> = {}): Snapshot {
  return {
    state: "idle",
    current: null,
    cleanup: null,
    counts: { ready: 1, waiting: 2, failed: 1, done: 5, skipped: 0 },
    recent: [
      { id: 4, at: 1_800_000_000, jobId: "a", provider: "claude", project: "demo-app", kind: "recorded", summary: "로그 전용으로 변경", notes: ["projects/demo-app/bg-policy.md", "projects/demo-app/log-only.md"], error: null },
      { id: 3, at: 1_799_999_500, jobId: "b", provider: "codex", project: "lore", kind: "no_change", summary: "새로 남길 결정이 없습니다.", notes: [], error: null },
      { id: 2, at: 1_799_999_000, jobId: "c", provider: "codex", project: null, kind: "skipped", summary: "작업자 세션", notes: [], error: null },
      { id: 1, at: 1_799_998_000, jobId: "d", provider: "claude", project: "sample-console", kind: "failed", summary: null, notes: [], error: "Transcript file not found" },
    ],
    attention: [
      { id: "f1", provider: "codex", transcript: "C:/s.jsonl", status: "failed", cursorLine: 0, cursorHash: "", project: "demoengine", sessionId: null, attempts: 3, nextAttemptAt: 0, lastError: "The evidence location or quotation does not match the provided source.", updatedAt: 1 },
    ],
    problem: null,
    settings: {
      knowledgeRoot: "D:/doc/knowledge",
      backend: "codex",
      fallback: false,
      models: { codex: null, claude: null },
      efforts: { codex: null, claude: null },
      modelTimeoutSecs: 600,
      qmdTimeoutSecs: 900,
      claudeProjectsDir: "C:/u/.claude/projects",
      codexSessionsDir: "C:/u/.codex/sessions",
      idleSecs: 600,
      since: 1,
      paused: false,
      syncEnabled: true,
      maxChars: 30000,
      notesBudget: 60000,
      qmdIndex: "index",
      commands: { codex: ["node", "codex.js"], claude: ["claude.exe"], qmd: ["node", "qmd"] },
    },
    autostart: false,
    hooks: [
      { provider: "claude", state: "connected", config: "C:/u/.claude/settings.json", command: `${CLI} hook claude`, found: `${CLI} hook claude`, error: null },
      { provider: "codex", state: "missing", config: "C:/u/.codex/hooks.json", command: `${CLI} hook codex`, found: null, error: null },
    ],
    connections: [
      { provider: "claude", state: "connected" },
      { provider: "codex", state: "missing" },
    ],
    review: 4,
    reviewing: false,
    version: "0.1.0",
    update: { phase: "idle", version: null, notes: null, downloaded: 0, total: null, error: null },
    sync: { phase: "idle", message: null, items: 0, comparing: false,lastSyncedAt: null },
    syncing: false,
    catalog: { codex: null, claude: null },
    defaultModels: { codex: "gpt-6.1-sol", claude: "haiku" },
    defaultEfforts: { codex: "low", claude: null },
    cliAvailable: { codex: true, claude: true },
    ...overrides,
  };
}

const CODEX_LIST = [
  { value: "gpt-a", label: "GPT-A", resolved: null, efforts: ["low", "ultra"] },
  { value: "gpt-b", label: "GPT-B", resolved: null, efforts: ["low", "max"] },
];
const CLAUDE_LIST = [
  { value: "haiku", label: "Haiku 5.5", resolved: "claude-haiku-5-5", efforts: ["low", "medium", "max"] },
  { value: "claude-haiku-4-5-20251001", label: "Haiku 4.5", resolved: "claude-haiku-4-5-20251001", efforts: [] },
];
const listed = (settings: Partial<Snapshot["settings"]> = {}) =>
  snapshot({ catalog: { codex: CODEX_LIST, claude: CLAUDE_LIST }, defaultModels: { codex: "gpt-a", claude: "haiku" }, settings: { ...snapshot().settings, ...settings } });

const calm = (overrides: Partial<Snapshot> = {}) =>
  snapshot({ hooks: snapshot().hooks.map((hook) => ({ ...hook, state: "connected" as const })), review: 0, attention: [], ...overrides });

function serve(initial: Snapshot) {
  let current = initial;
  api.getSnapshot.mockImplementation(async () => current);
  api.saveSettings.mockImplementation(async (settings: EditableSettings) => {
    current = { ...current, settings: { ...current.settings, ...settings } };
  });
}

async function findFooter() {
  const folder = await screen.findByRole("button", { name: "Memory folder" });
  return folder.closest("footer") as HTMLElement;
}

beforeEach(() => {
  Object.values(api).forEach((mock) => mock.mockReset());
  api.onStateChanged.mockImplementation(() => Promise.resolve(() => {}));
  api.onPopoverShown.mockImplementation(() => Promise.resolve(() => {}));
  api.onShowSettings.mockImplementation(() => Promise.resolve(() => {}));
  useAppStore.setState({ snapshot: null, view: "status", error: null, queue: null, queueFocus: "ready" });
  setThemePreference("system");
});

afterEach(() => {
  vi.useRealTimers();
});

describe("status view", () => {
  it("keeps the header to the app name, pause and settings", async () => {
    api.getSnapshot.mockResolvedValue(snapshot());
    render(<App />);
    const header = (await screen.findByText("Lore")).closest("header") as HTMLElement;
    expect(within(header).getAllByRole("button").map((button) => button.getAttribute("aria-label"))).toEqual(["Pause", "Settings"]);
    expect(header).not.toHaveTextContent("pending");
  });

  it("lists what needs attention in one place and acts on each item", async () => {
    api.getSnapshot.mockResolvedValue(snapshot());
    api.getQueue.mockResolvedValue([]);
    render(<App />);
    const attention = await screen.findByRole("region", { name: "Needs attention" });
    const titles = within(attention).getAllByText(/hooks$|changed notes$|failed$/).map((element) => element.textContent);
    expect(titles).toEqual(["Connect Codex hooks", "Review 4 changed notes", "1 recording failed"]);
    const detail = "demoengine · The evidence location or quotation does not match the provided source.";
    expect(within(attention).getByText(detail)).toHaveAttribute("title", detail);

    await userEvent.click(within(attention).getByRole("button", { name: "Connect" }));
    expect(api.openIntegrations).toHaveBeenCalledWith();
    await userEvent.click(within(attention).getByRole("button", { name: "Review" }));
    expect(api.openReview).toHaveBeenCalledTimes(1);
    await userEvent.click(within(attention).getByRole("button", { name: "View" }));
    expect(await screen.findByRole("region", { name: "Failed" })).toBeInTheDocument();
  });

  it("hides Needs attention when nothing needs it", async () => {
    api.getSnapshot.mockResolvedValue(calm());
    render(<App />);
    await screen.findByRole("region", { name: "Recent activity" });
    expect(screen.queryByRole("region", { name: "Needs attention" })).not.toBeInTheDocument();
  });

  it("reports a saved recording model that the CLI no longer lists and opens Settings", async () => {
    api.getSnapshot.mockResolvedValue(calm({ catalog: { codex: CODEX_LIST, claude: null }, settings: { ...snapshot().settings, models: { codex: "gpt-retired", claude: null } } }));
    render(<App />);
    const attention = await screen.findByRole("region", { name: "Needs attention" });
    expect(attention).toHaveTextContent("Recording model not available");
    expect(attention).toHaveTextContent("gpt-retired is not in Codex’s model list for this account.");
    await userEvent.click(within(attention).getByRole("button", { name: "Settings" }));
    expect(await screen.findByLabelText("Model name")).toHaveValue("gpt-retired");
  });

  it("does not report the model while no list is available or the model is listed", async () => {
    for (const state of [calm({ settings: { ...snapshot().settings, models: { codex: "gpt-retired", claude: null } } }), calm({ catalog: { codex: CODEX_LIST, claude: null }, settings: { ...snapshot().settings, models: { codex: "gpt-b", claude: null } } })]) {
      api.getSnapshot.mockResolvedValue(state);
      useAppStore.setState({ snapshot: null });
      const { unmount } = render(<App />);
      await screen.findByRole("region", { name: "Recent activity" });
      expect(screen.queryByRole("region", { name: "Needs attention" })).not.toBeInTheDocument();
      unmount();
    }
  });

  it("reports interrupted recordings and memory not yet shared", async () => {
    const interrupted: Job = { ...snapshot().attention[0], id: "i1", status: "index_pending", project: "sample-app", attempts: 1, lastError: null };
    api.getSnapshot.mockResolvedValue(calm({ attention: [interrupted], sync: { phase: "failing", message: null, items: 0, comparing: false,lastSyncedAt: null } }));
    api.getQueue.mockResolvedValue([]);
    render(<App />);
    const attention = await screen.findByRole("region", { name: "Needs attention" });
    expect(attention).toHaveTextContent("1 recording interrupted");
    expect(attention).toHaveTextContent("sample-app · Index pending");
    expect(attention).toHaveTextContent("Memory not shared yet");
    await userEvent.click(within(attention).getByRole("button", { name: "Sync" }));
    expect(api.syncNow).toHaveBeenCalledTimes(1);
    await userEvent.click(within(attention).getByRole("button", { name: "View" }));
    expect(await screen.findByRole("region", { name: "Pending" })).toBeInTheDocument();
  });

  it("gives every recent row one line under its title and hides skipped sessions", async () => {
    api.getSnapshot.mockResolvedValue(snapshot());
    render(<App />);
    const recent = await screen.findByRole("region", { name: "Recent activity" });
    expect(within(recent).getByText("bg-policy.md · log-only.md")).toHaveAttribute("title", "projects/demo-app/bg-policy.md\nprojects/demo-app/log-only.md");
    expect(within(recent).getByText("새로 남길 결정이 없습니다.")).toHaveAttribute("title", "새로 남길 결정이 없습니다.");
    expect(within(recent).getByText("Transcript file not found")).toHaveAttribute("title", "Transcript file not found");
    expect(within(recent).getByText("demo-app")).toHaveAttribute("title", "demo-app");
    expect(within(recent).queryByText("작업자 세션")).not.toBeInTheDocument();
    expect(within(recent).queryByRole("button", { name: /^Open / })).not.toBeInTheDocument();
  });

  it("opens the updated notes in a card over the list", async () => {
    api.getSnapshot.mockResolvedValue(snapshot());
    render(<App />);
    const recent = await screen.findByRole("region", { name: "Recent activity" });
    const trigger = within(recent).getByRole("button", { name: "2 notes updated" });
    await userEvent.click(trigger);
    const card = screen.getByRole("dialog", { name: "Updated notes in demo-app" });
    expect(recent).not.toContainElement(card);
    expect(trigger).toHaveAttribute("aria-expanded", "true");
    expect(card).toHaveTextContent("로그 전용으로 변경");
    expect(within(card).getByRole("button", { name: "Open bg-policy.md" })).toHaveFocus();
    await userEvent.click(within(card).getByRole("button", { name: "Open log-only.md" }));
    expect(api.openNote).toHaveBeenCalledWith("projects/demo-app/log-only.md");
    await userEvent.click(within(card).getByRole("button", { name: "Review changes" }));
    expect(api.openReview).toHaveBeenCalledTimes(1);
    expect(screen.queryByRole("dialog", { name: "Updated notes in demo-app" })).not.toBeInTheDocument();
  });

  it("offers no review from the notes card when nothing waits for review", async () => {
    api.getSnapshot.mockResolvedValue(calm());
    render(<App />);
    await userEvent.click(await screen.findByRole("button", { name: "2 notes updated" }));
    expect(within(screen.getByRole("dialog", { name: "Updated notes in demo-app" })).queryByRole("button", { name: "Review changes" })).not.toBeInTheDocument();
  });

  it("closes a card with Escape without hiding the popover, and by clicking elsewhere", async () => {
    api.getSnapshot.mockResolvedValue(snapshot());
    render(<App />);
    await userEvent.click(await screen.findByRole("button", { name: "2 notes updated" }));
    await userEvent.keyboard("{Escape}");
    expect(screen.queryByRole("dialog", { name: "Updated notes in demo-app" })).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: "2 notes updated" })).toHaveFocus();
    expect(api.hideWindow).not.toHaveBeenCalled();

    await userEvent.click(screen.getByRole("button", { name: "Clear history" }));
    expect(screen.getByRole("alertdialog", { name: "Clear history confirmation" })).toBeInTheDocument();
    await userEvent.click(screen.getByText("demo-app"));
    expect(screen.queryByRole("alertdialog", { name: "Clear history confirmation" })).not.toBeInTheDocument();
    expect(api.clearActivity).not.toHaveBeenCalled();

    await userEvent.keyboard("{Escape}");
    expect(api.hideWindow).toHaveBeenCalledTimes(1);
  });

  it("links the waiting and retrying sessions to the queue", async () => {
    api.getSnapshot.mockResolvedValue(snapshot());
    api.getQueue.mockResolvedValue([]);
    render(<App />);
    const now = await screen.findByRole("region", { name: "Current activity" });
    await userEvent.click(within(now).getByRole("button", { name: "1 session waiting · 2 retrying" }));
    expect(await screen.findByRole("region", { name: "Pending" })).toBeInTheDocument();
  });

  it("shows no queue link when nothing is waiting", async () => {
    api.getSnapshot.mockResolvedValue(calm({ counts: { ready: 0, waiting: 0, failed: 0, done: 5, skipped: 0 } }));
    render(<App />);
    const now = await screen.findByRole("region", { name: "Current activity" });
    expect(within(now).queryByRole("button")).not.toBeInTheDocument();
  });

  it("says recording waits while the review window is open", async () => {
    api.getSnapshot.mockResolvedValue(snapshot({ state: "reviewing", reviewing: true }));
    render(<App />);
    const now = await screen.findByRole("region", { name: "Current activity" });
    expect(now).toHaveTextContent("Reviewing");
    expect(now).toHaveTextContent("New recordings wait while the review window is open.");
  });

  it("lists synced notes to check without holding recording, and opens the sync window", async () => {
    api.getSnapshot.mockResolvedValue(calm({ sync: { phase: "idle", message: null, items: 2, comparing: false, lastSyncedAt: NOW - 60 } }));
    render(<App />);
    const now = await screen.findByRole("region", { name: "Current activity" });
    expect(now).toHaveTextContent("Waiting for the next session");
    const attention = screen.getByRole("region", { name: "Needs attention" });
    expect(attention).toHaveTextContent("2 synced notes to check");
    await userEvent.click(within(attention).getByRole("button", { name: "Open" }));
    expect(api.openSync).toHaveBeenCalledTimes(1);
  });

  it("shows the session being recorded", async () => {
    api.getSnapshot.mockResolvedValue(snapshot({ state: "working", current: { jobId: "a", provider: "claude", project: "sample-app", startedAt: 1_800_000_000 } }));
    render(<App />);
    const now = await screen.findByRole("region", { name: "Current activity" });
    expect(now).toHaveTextContent("Recording sample-app");
    expect(now).toHaveTextContent("Claude session · Codex");
  });

  it("shows the folder being cleaned up once the cleanup has started", async () => {
    api.getSnapshot.mockResolvedValue(snapshot({ state: "working", cleanup: { folder: "lore", absorb: [], startedAt: 1_800_000_000 } }));
    render(<App />);
    const now = await screen.findByRole("region", { name: "Current activity" });
    expect(now).toHaveTextContent("Cleaning up lore");
    expect(now).toHaveTextContent(/Codex · (Just started|\d+ min elapsed)/);
  });

  it("keeps showing the usual state while a cleanup still waits for its turn", async () => {
    api.getSnapshot.mockResolvedValue(calm({ cleanup: { folder: "lore", absorb: [], startedAt: null } }));
    render(<App />);
    expect(await screen.findByRole("region", { name: "Current activity" })).toHaveTextContent("Waiting for the next session");
  });

  const cleaned = (overrides: Partial<Snapshot["recent"][number]> = {}): Snapshot["recent"][number] => ({
    id: 9,
    at: 1_800_000_300,
    jobId: "cleanup",
    provider: "claude",
    project: "lore",
    kind: "cleaned",
    summary: "중복 노트를 합쳤습니다.\nMoved projects/qmd-memory/sync.md to projects/lore/sync.md\nRemoved projects/lore/old.md: 대상이 없어졌습니다.",
    notes: ["projects/lore/kept.md", "projects/lore/sync.md", "projects/lore/old.md", "projects/qmd-memory/sync.md"],
    error: null,
    ...overrides,
  });

  it("lists a cleanup by what it changed and removed, and offers to open only the notes that remain", async () => {
    api.getSnapshot.mockResolvedValue(snapshot({ recent: [cleaned()] }));
    render(<App />);
    const recent = await screen.findByRole("region", { name: "Recent activity" });
    expect(within(recent).getByText("kept.md · sync.md · old.md · sync.md")).toBeInTheDocument();
    await userEvent.click(within(recent).getByRole("button", { name: "2 changed · 2 removed" }));
    const card = screen.getByRole("dialog", { name: "Updated notes in lore" });
    expect(card).toHaveTextContent("중복 노트를 합쳤습니다.");
    expect(card).not.toHaveTextContent("Removed projects/lore/old.md");
    expect(within(card).getByRole("button", { name: "Open kept.md" })).toHaveFocus();
    expect(within(card).getByRole("button", { name: "Open sync.md" })).toBeInTheDocument();
    expect(within(card).queryByRole("button", { name: "Open old.md" })).not.toBeInTheDocument();
    expect(within(card).getByTitle("대상이 없어졌습니다.")).toHaveTextContent("old.md");
    expect(within(card).getByTitle("Moved to projects/lore/sync.md")).toHaveTextContent("sync.md");
  });

  it("says so when a cleanup found nothing to change, stopped early or failed", async () => {
    api.getSnapshot.mockResolvedValue(
      snapshot({
        recent: [
          cleaned({ id: 12, summary: "정리할 것이 없습니다.", notes: [] }),
          cleaned({ id: 11, project: "demo", error: "usage limit reached" }),
          cleaned({ id: 10, project: "wevrel", kind: "cleanup_failed", summary: null, notes: [], error: "Invalid memory path: x" }),
        ],
      }),
    );
    render(<App />);
    const recent = await screen.findByRole("region", { name: "Recent activity" });
    expect(within(recent).getByText("Nothing to clean up")).toBeInTheDocument();
    expect(within(recent).getByText("정리할 것이 없습니다.")).toBeInTheDocument();
    expect(within(recent).getByText("Cleanup failed")).toBeInTheDocument();
    expect(within(recent).getByText("Invalid memory path: x")).toBeInTheDocument();
    await userEvent.click(within(recent).getByRole("button", { name: "2 changed · 2 removed" }));
    expect(screen.getByRole("dialog", { name: "Updated notes in demo" })).toHaveTextContent("Stopped early: usage limit reached");
  });

  it("keeps elapsed and relative times moving while the popover stays open", async () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date(1_800_000_130 * 1000));
    api.getSnapshot.mockResolvedValue(snapshot({ state: "working", current: { jobId: "a", provider: "claude", project: "sample-app", startedAt: 1_800_000_000 } }));
    render(<App />);
    await act(async () => {
      await Promise.resolve();
    });
    const now = screen.getByRole("region", { name: "Current activity" });
    const recent = screen.getByRole("region", { name: "Recent activity" });
    expect(now).toHaveTextContent("2 min elapsed");
    expect(recent).toHaveTextContent("2m ago");
    await act(async () => {
      vi.advanceTimersByTime(60_000);
    });
    expect(now).toHaveTextContent("3 min elapsed");
    expect(recent).toHaveTextContent("3m ago");
    expect(api.getSnapshot).toHaveBeenCalledTimes(1);
  });

  it("clears the recent records only after a second confirmation", async () => {
    api.getSnapshot.mockResolvedValue(snapshot());
    api.clearActivity.mockResolvedValue(2);
    render(<App />);
    const trigger = await screen.findByRole("button", { name: "Clear history" });
    await userEvent.click(trigger);
    const confirm = screen.getByRole("alertdialog", { name: "Clear history confirmation" });
    expect(confirm).toHaveTextContent("Clear recent activity?");
    expect(screen.getByRole("region", { name: "Recent activity" })).not.toContainElement(confirm);
    await userEvent.click(within(confirm).getByRole("button", { name: "Cancel" }));
    expect(api.clearActivity).not.toHaveBeenCalled();
    expect(trigger).toHaveFocus();

    await userEvent.click(trigger);
    await userEvent.click(within(screen.getByRole("alertdialog", { name: "Clear history confirmation" })).getByRole("button", { name: "Clear all" }));
    expect(api.clearActivity).toHaveBeenCalledTimes(1);
    expect(api.getSnapshot).toHaveBeenCalledTimes(2);
  });

  it("shows an empty state and offers no clearing before anything is recorded", async () => {
    api.getSnapshot.mockResolvedValue(snapshot({ recent: [] }));
    render(<App />);
    expect(await screen.findByText("No sessions recorded yet")).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Clear history" })).not.toBeInTheDocument();
  });

  it("pauses from the header and offers to resume while paused", async () => {
    api.getSnapshot.mockResolvedValue(snapshot());
    const { unmount } = render(<App />);
    await userEvent.click(await screen.findByRole("button", { name: "Pause" }));
    expect(api.setPaused).toHaveBeenCalledWith(true);
    unmount();

    useAppStore.setState({ snapshot: null });
    api.getSnapshot.mockResolvedValue(snapshot({ state: "paused", settings: { ...snapshot().settings, paused: true } }));
    render(<App />);
    const now = await screen.findByRole("region", { name: "Current activity" });
    expect(now).toHaveTextContent("Recording paused");
    await userEvent.click(within(now).getByRole("button", { name: "Resume" }));
    expect(api.setPaused).toHaveBeenLastCalledWith(false);
  });
});

describe("footer", () => {
  it("shows when memory was last synced and syncs on request", async () => {
    vi.useFakeTimers({ toFake: ["Date"] });
    vi.setSystemTime(new Date(NOW * 1000));
    api.getSnapshot.mockResolvedValue(calm({ sync: { phase: "idle", message: null, items: 0, comparing: false,lastSyncedAt: NOW - 300 } }));
    render(<App />);
    const footer = await findFooter();
    expect(footer).toHaveTextContent("Synced 5m ago");
    await userEvent.click(within(footer).getByRole("button", { name: "Sync now" }));
    expect(api.syncNow).toHaveBeenCalledTimes(1);
  });

  it("marks automatic sync as off next to the last sync", async () => {
    vi.useFakeTimers({ toFake: ["Date"] });
    vi.setSystemTime(new Date(NOW * 1000));
    api.getSnapshot.mockResolvedValue(calm({ settings: { ...snapshot().settings, syncEnabled: false }, sync: { phase: "idle", message: null, items: 0, comparing: false,lastSyncedAt: NOW - 30 } }));
    render(<App />);
    expect(await findFooter()).toHaveTextContent("Synced just now · auto off");
  });

  it("opens the sync window when synced notes are waiting to be checked", async () => {
    api.getSnapshot.mockResolvedValue(snapshot({ sync: { phase: "idle", message: null, items: 1, comparing: false, lastSyncedAt: NOW } }));
    render(<App />);
    const footer = await findFooter();
    expect(footer).toHaveTextContent("1 to check");
    await userEvent.click(within(footer).getByRole("button", { name: "Open sync" }));
    expect(api.openSync).toHaveBeenCalledTimes(1);
    expect(api.syncNow).not.toHaveBeenCalled();
  });

  it("says when sync has never run, when it keeps failing, and a sync in progress", async () => {
    api.getSnapshot.mockResolvedValue(calm({ settings: { ...snapshot().settings, syncEnabled: false } }));
    const { unmount } = render(<App />);
    expect(await findFooter()).toHaveTextContent("Sync off");
    unmount();

    useAppStore.setState({ snapshot: null });
    api.getSnapshot.mockResolvedValue(calm({ sync: { phase: "failing", message: "authentication failed", items: 0, comparing: false, lastSyncedAt: null } }));
    const second = render(<App />);
    expect(await findFooter()).toHaveTextContent("Sync needs attention");
    second.unmount();

    useAppStore.setState({ snapshot: null });
    api.getSnapshot.mockResolvedValue(calm({ sync: { phase: "retrying", message: "network unreachable", items: 0, comparing: false, lastSyncedAt: Math.floor(Date.now() / 1000) } }));
    const third = render(<App />);
    expect(await findFooter()).toHaveTextContent("Synced just now");
    third.unmount();

    useAppStore.setState({ snapshot: null });
    api.getSnapshot.mockResolvedValue(calm({ syncing: true }));
    render(<App />);
    const footer = await findFooter();
    expect(footer).toHaveTextContent("Syncing…");
    expect(within(footer).getByRole("button", { name: "Sync now" })).toBeDisabled();
  });

  it("opens the memory folder and keeps its path in the tooltip", async () => {
    api.getSnapshot.mockResolvedValue(calm());
    render(<App />);
    const folder = await screen.findByRole("button", { name: "Memory folder" });
    expect(folder).toHaveAttribute("title", "D:/doc/knowledge");
    await userEvent.click(folder);
    expect(api.openFolder).toHaveBeenCalledWith("knowledge");
  });
});

describe("queue view", () => {
  const job = (overrides: Partial<Job>): Job => ({
    id: "j",
    provider: "claude",
    transcript: "C:/u/.claude/projects/D--src-x/s.jsonl",
    status: "pending",
    cursorLine: 0,
    cursorHash: "",
    project: null,
    sessionId: null,
    attempts: 0,
    nextAttemptAt: 0,
    lastError: null,
    updatedAt: 1_800_000_000,
    ...overrides,
  });
  const queued = [
    job({ id: "rec", project: "sample-app", sessionId: "rec-session" }),
    job({ id: "next", project: "ae-recipe-wizard", sessionId: "1f2e3d4c-aaaa-bbbb-cccc-000000000001", cursorLine: 120 }),
    job({ id: "unread", provider: "codex", transcript: "C:\\u\\.codex\\sessions\\rollout-x-0aa1.jsonl" }),
    job({ id: "retry", project: "demoengine", sessionId: "retry-session", attempts: 2, nextAttemptAt: 4_000_000_000, lastError: "Model timeout" }),
    job({ id: "f1", provider: "codex", status: "failed", project: "sample-viewer", sessionId: "failed-session", attempts: 3, lastError: "Transcript file not found" }),
  ];
  const recording = { state: "working" as const, current: { jobId: "rec", provider: "claude" as const, project: "sample-app", startedAt: 1_800_000_000 } };

  it("shows each session's project, session id and state in its group", async () => {
    api.getSnapshot.mockResolvedValue(snapshot(recording));
    api.getQueue.mockResolvedValue(queued);
    render(<App />);
    await userEvent.click(await screen.findByRole("button", { name: "1 session waiting · 2 retrying" }));

    const now = await screen.findByRole("region", { name: "Recording" });
    expect(now).toHaveTextContent("sample-app");
    expect(now).toHaveTextContent("rec-session");
    const ready = screen.getByRole("region", { name: "Pending" });
    expect(within(ready).getByText("ae-recipe-wizard")).toBeInTheDocument();
    expect(within(ready).getByText("1f2e3d4c-aaaa-bbbb-cccc-000000000001")).toBeInTheDocument();
    expect(within(ready).getByText("Resume recording")).toBeInTheDocument();
    expect(within(ready).getByText("rollout-x-0aa1.jsonl")).toHaveAttribute("title", "Session rollout-x-0aa1.jsonl\nC:\\u\\.codex\\sessions\\rollout-x-0aa1.jsonl");
    expect(within(ready).getByText("Codex", { selector: ".name" })).toBeInTheDocument();
    const waiting = screen.getByRole("region", { name: "Retrying" });
    expect(waiting).toHaveTextContent("demoengine");
    expect(waiting).toHaveTextContent("2 failed attempts · Model timeout");
    const failed = screen.getByRole("region", { name: "Failed" });
    expect(failed).toHaveTextContent("Transcript file not found");

    await userEvent.click(screen.getByRole("button", { name: "Back" }));
    expect(await screen.findByRole("region", { name: "Recent activity" })).toBeInTheDocument();
  });

  it("says a group is empty instead of hiding it", async () => {
    api.getSnapshot.mockResolvedValue(snapshot({ counts: { ready: 0, waiting: 2, failed: 0, done: 0, skipped: 0 } }));
    api.getQueue.mockResolvedValue([]);
    render(<App />);
    await userEvent.click(await screen.findByRole("button", { name: "2 retrying" }));
    expect(await screen.findByRole("region", { name: "Retrying" })).toHaveTextContent("No sessions are waiting for a retry.");
    expect(screen.getByRole("region", { name: "Recording" })).toHaveTextContent("No sessions are being recorded.");
  });

  it("scrolls to the group that was opened once, not again on later updates", async () => {
    const scroll = vi.spyOn(Element.prototype, "scrollIntoView");
    try {
      api.getSnapshot.mockResolvedValue(snapshot());
      api.getQueue.mockResolvedValue(queued);
      render(<App />);
      const attention = await screen.findByRole("region", { name: "Needs attention" });
      await userEvent.click(within(attention).getByRole("button", { name: "View" }));
      const failed = await screen.findByRole("region", { name: "Failed" });
      expect(scroll.mock.contexts).toEqual([failed]);

      const [[stateChanged]] = api.onStateChanged.mock.calls as unknown as [[() => void]];
      await act(async () => stateChanged());
      expect(api.getQueue).toHaveBeenCalledTimes(2);
      expect(scroll).toHaveBeenCalledTimes(1);
    } finally {
      scroll.mockRestore();
    }
  });

  async function openFailed() {
    api.getSnapshot.mockResolvedValue(snapshot());
    api.getQueue.mockResolvedValue(queued);
    render(<App />);
    const attention = await screen.findByRole("region", { name: "Needs attention" });
    await userEvent.click(within(attention).getByRole("button", { name: "View" }));
    return screen.findByRole("region", { name: "Failed" });
  }

  it("retries a failed job", async () => {
    api.retryJob.mockResolvedValue(true);
    const failed = await openFailed();
    await userEvent.click(within(failed).getByRole("button", { name: "Retry" }));
    expect(api.retryJob).toHaveBeenCalledWith("f1");
  });

  it("skips a failed job only after a second confirmation in a card that leaves the row as it was", async () => {
    api.discardJob.mockResolvedValue(true);
    const failed = await openFailed();
    const trigger = within(failed).getByRole("button", { name: "Skip" });
    await userEvent.click(trigger);
    const confirm = screen.getByRole("alertdialog", { name: "Skip failed segment confirmation" });
    expect(confirm).toHaveTextContent("Skip the failed segment?");
    expect(failed).not.toContainElement(confirm);
    expect(trigger).toHaveAttribute("aria-expanded", "true");
    await userEvent.click(within(confirm).getByRole("button", { name: "Cancel" }));
    expect(api.discardJob).not.toHaveBeenCalled();
    expect(trigger).toHaveFocus();

    await userEvent.click(trigger);
    await userEvent.click(within(screen.getByRole("alertdialog", { name: "Skip failed segment confirmation" })).getByRole("button", { name: "Skip" }));
    expect(api.discardJob).toHaveBeenCalledWith("f1");
  });

  it("shows why skipping failed", async () => {
    api.discardJob.mockRejectedValue("File error: the file is in use by another process.");
    const failed = await openFailed();
    await userEvent.click(within(failed).getByRole("button", { name: "Skip" }));
    await userEvent.click(within(screen.getByRole("alertdialog", { name: "Skip failed segment confirmation" })).getByRole("button", { name: "Skip" }));
    expect(await screen.findByText("File error: the file is in use by another process.")).toBeInTheDocument();
  });
});

describe("settings view", () => {
  async function openSettings(initial: Snapshot = snapshot()) {
    serve(initial);
    render(<App />);
    const [header] = await screen.findAllByRole("button", { name: "Settings" });
    await userEvent.click(header);
  }

  const cli = (name: "Claude" | "Codex") => screen.getByRole("group", { name });
  const cliOrder = () => within(screen.getByRole("region", { name: "Recording" })).getAllByRole("group").map((group) => group.getAttribute("aria-label"));

  it("lists each recording CLI with its own model, turns the second one on at once and has no save button", async () => {
    await openSettings(snapshot({ settings: { ...snapshot().settings, backend: "claude", models: { codex: null, claude: "opus" } } }));
    expect(screen.queryByRole("button", { name: "Save" })).not.toBeInTheDocument();
    expect(cliOrder()).toEqual(["Claude", "Codex"]);
    expect(within(cli("Claude")).getByText("Records every session.")).toBeInTheDocument();
    expect(within(cli("Claude")).getByLabelText("Model")).toHaveValue("opus");
    expect(within(cli("Claude")).queryByRole("switch")).not.toBeInTheDocument();
    expect(within(cli("Codex")).getByText("Not used for recording.")).toBeInTheDocument();
    expect(within(cli("Codex")).queryByLabelText("Model")).not.toBeInTheDocument();
    expect(screen.getByText("Each recording runs claude --print and uses your signed-in account’s allowance.")).toBeInTheDocument();

    await userEvent.click(within(cli("Codex")).getByRole("switch", { name: "Record with Codex" }));
    expect(api.saveSettings).toHaveBeenLastCalledWith(expect.objectContaining({ backend: "claude", fallback: true, models: { codex: null, claude: "opus" } }));
    expect(await within(cli("Codex")).findByLabelText("Model")).toHaveValue("");
    expect(within(cli("Codex")).getByText("Used when Claude cannot run, for example at its usage limit.")).toBeInTheDocument();
    expect(within(cli("Claude")).getByText("Used first.")).toBeInTheDocument();
    expect(within(cli("Claude")).getByLabelText("Model")).toHaveValue("opus");
    expect(screen.getByText("Lore records with the first CLI that can run. Each uses its own signed-in account’s allowance.")).toBeInTheDocument();
  });

  it("moves the second CLI to the front and stops using the first", async () => {
    await openSettings(snapshot({ settings: { ...snapshot().settings, backend: "claude", fallback: true, models: { codex: "gpt-6.1-sol", claude: "opus" } } }));
    await userEvent.click(within(cli("Codex")).getByRole("button", { name: "Use first" }));
    expect(api.saveSettings).toHaveBeenLastCalledWith(expect.objectContaining({ backend: "codex", fallback: true }));
    await waitFor(() => expect(cliOrder()).toEqual(["Codex", "Claude"]));
    expect(within(cli("Codex")).queryByRole("button", { name: "Use first" })).not.toBeInTheDocument();
    expect(within(cli("Codex")).getByLabelText("Model")).toHaveValue("gpt-6.1-sol");

    await userEvent.click(within(cli("Codex")).getByRole("switch", { name: "Record with Codex" }));
    expect(api.saveSettings).toHaveBeenLastCalledWith(expect.objectContaining({ backend: "claude", fallback: false }));
    await waitFor(() => expect(cliOrder()).toEqual(["Claude", "Codex"]));
    expect(within(cli("Claude")).getByText("Records every session.")).toBeInTheDocument();
    expect(within(cli("Codex")).getByRole("switch", { name: "Record with Codex" })).not.toBeChecked();
  });

  it("does not turn on a CLI that is not on this PC and opens its setup", async () => {
    await openSettings(snapshot({ cliAvailable: { codex: true, claude: false } }));
    expect(within(cli("Claude")).getByText("Not found on this PC.")).toBeInTheDocument();
    expect(within(cli("Claude")).getByRole("switch", { name: "Record with Claude" })).toBeDisabled();
    expect(within(cli("Codex")).queryByRole("button", { name: "Set up…" })).not.toBeInTheDocument();
    await userEvent.click(within(cli("Claude")).getByRole("button", { name: "Set up…" }));
    expect(api.openIntegrations).toHaveBeenLastCalledWith("tools");
    expect(api.saveSettings).not.toHaveBeenCalled();
  });

  it("saves a model when the field is left and says it was saved", async () => {
    await openSettings();
    await userEvent.type(screen.getByLabelText("Model"), "gpt-6.1-sol{Enter}");
    expect(api.saveSettings).toHaveBeenCalledWith({
      knowledgeRoot: "D:/doc/knowledge",
      backend: "codex",
      fallback: false,
      models: { codex: "gpt-6.1-sol", claude: null },
      efforts: { codex: null, claude: null },
      modelTimeoutSecs: 600,
      qmdTimeoutSecs: 900,
      maxChars: 30000,
      notesBudget: 60000,
    });
    expect(await screen.findByText("Saved")).toBeInTheDocument();
    await userEvent.type(screen.getByLabelText("Model"), "x");
    expect(screen.queryByText("Saved")).not.toBeInTheDocument();
  });

  it("does not save a field that was left unchanged", async () => {
    await openSettings();
    await userEvent.click(screen.getByLabelText("Model"));
    await userEvent.tab();
    expect(api.saveSettings).not.toHaveBeenCalled();
  });

  it("stores a blank model as the CLI default", async () => {
    await openSettings(snapshot({ settings: { ...snapshot().settings, models: { codex: "gpt-6.1-sol", claude: null } } }));
    await userEvent.clear(screen.getByLabelText("Model"));
    await userEvent.type(screen.getByLabelText("Model"), "   {Enter}");
    expect(api.saveSettings).toHaveBeenLastCalledWith(expect.objectContaining({ models: { codex: null, claude: null } }));
  });

  it("keeps a model typed just before turning the second CLI on", async () => {
    await openSettings();
    await userEvent.type(screen.getByLabelText("Model"), "gpt-6.1-sol");
    await userEvent.click(within(cli("Claude")).getByRole("switch", { name: "Record with Claude" }));
    await waitFor(() => expect(api.saveSettings).toHaveBeenCalledTimes(2));
    expect(api.saveSettings).toHaveBeenNthCalledWith(1, expect.objectContaining({ fallback: false, models: { codex: "gpt-6.1-sol", claude: null } }));
    expect(api.saveSettings).toHaveBeenLastCalledWith(expect.objectContaining({ backend: "codex", fallback: true, models: { codex: "gpt-6.1-sol", claude: null } }));
  });

  it("lists the models the CLI reported, names the default and saves the chosen one", async () => {
    await openSettings(listed());
    const model = screen.getByLabelText("Model");
    expect(within(model).getAllByRole("option").map((option) => option.textContent)).toEqual(["Default · GPT-A", "GPT-A", "GPT-B", "Other…"]);
    expect(model).toHaveValue("");
    await userEvent.selectOptions(model, "gpt-b");
    expect(api.saveSettings).toHaveBeenLastCalledWith(expect.objectContaining({ models: { codex: "gpt-b", claude: null }, efforts: { codex: null, claude: null } }));
    await userEvent.selectOptions(screen.getByLabelText("Model"), "");
    expect(api.saveSettings).toHaveBeenLastCalledWith(expect.objectContaining({ models: { codex: null, claude: null } }));
  });

  it("offers only the efforts the model supports and drops one the next model lacks", async () => {
    await openSettings(listed({ models: { codex: "gpt-a", claude: null }, efforts: { codex: "ultra", claude: null } }));
    const effort = screen.getByLabelText("Effort");
    expect(effort).toHaveValue("ultra");
    expect(within(effort).getAllByRole("option").map((option) => option.textContent)).toEqual(["Default · low", "low", "ultra"]);
    await userEvent.selectOptions(screen.getByLabelText("Model"), "gpt-b");
    expect(api.saveSettings).toHaveBeenLastCalledWith(expect.objectContaining({ models: { codex: "gpt-b", claude: null }, efforts: { codex: null, claude: null } }));
    expect(within(await screen.findByLabelText("Effort")).getAllByRole("option").map((option) => option.textContent)).toEqual(["Default · low", "low", "max"]);
  });

  it("keeps an effort the next model also supports and warns about the heavier levels", async () => {
    await openSettings(listed({ models: { codex: "gpt-a", claude: null }, efforts: { codex: "low", claude: null } }));
    expect(screen.queryByText(/Higher effort makes each recording slower/)).not.toBeInTheDocument();
    await userEvent.selectOptions(screen.getByLabelText("Model"), "gpt-b");
    expect(api.saveSettings).toHaveBeenLastCalledWith(expect.objectContaining({ efforts: { codex: "low", claude: null } }));
    await userEvent.selectOptions(await screen.findByLabelText("Effort"), "max");
    expect(api.saveSettings).toHaveBeenLastCalledWith(expect.objectContaining({ efforts: { codex: "max", claude: null } }));
    expect(await screen.findByText(/Higher effort makes each recording slower/)).toBeInTheDocument();
  });

  it("hides Effort for a model without levels and shows Claude's default without a level", async () => {
    await openSettings(listed({ backend: "claude" }));
    expect(within(screen.getByLabelText("Effort")).getAllByRole("option")[0]).toHaveTextContent(/^Default$/);
    await userEvent.selectOptions(screen.getByLabelText("Model"), "claude-haiku-4-5-20251001");
    expect(api.saveSettings).toHaveBeenLastCalledWith(expect.objectContaining({ models: { codex: null, claude: "claude-haiku-4-5-20251001" } }));
    await waitFor(() => expect(screen.queryByLabelText("Effort")).not.toBeInTheDocument());
  });

  it("types a model that is not listed under Other and resets its effort", async () => {
    await openSettings(listed({ efforts: { codex: "ultra", claude: null } }));
    await userEvent.selectOptions(screen.getByLabelText("Model"), "__other");
    expect(api.saveSettings).not.toHaveBeenCalled();
    await userEvent.type(screen.getByLabelText("Model name"), "gpt-custom{Enter}");
    expect(api.saveSettings).toHaveBeenLastCalledWith(expect.objectContaining({ models: { codex: "gpt-custom", claude: null }, efforts: { codex: null, claude: null } }));
  });

  it("shows a saved model that is not listed under Other with its name", async () => {
    await openSettings(listed({ models: { codex: "gpt-old", claude: null } }));
    expect(screen.getByLabelText("Model")).toHaveValue("__other");
    expect(screen.getByLabelText("Model name")).toHaveValue("gpt-old");
    expect(screen.queryByLabelText("Effort")).not.toBeInTheDocument();
  });

  it("matches a full Claude model name to its listed alias", async () => {
    await openSettings(listed({ backend: "claude", models: { codex: null, claude: "claude-haiku-5-5" } }));
    expect(screen.getByLabelText("Model")).toHaveValue("haiku");
    expect(screen.getByLabelText("Effort")).toBeInTheDocument();
  });

  it("shows the processing limits under Advanced and saves the one that changed", async () => {
    await openSettings();
    expect(screen.queryByLabelText("Recording timeout")).not.toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Show" }));
    expect(screen.getByLabelText("Recording timeout")).toHaveValue(600);
    expect(screen.getByLabelText("qmd timeout")).toHaveValue(900);
    expect(screen.getByLabelText("New conversation per recording")).toHaveValue(30000);
    expect(screen.getByLabelText("Existing notes per recording")).toHaveValue(60000);
    await userEvent.clear(screen.getByLabelText("Recording timeout"));
    await userEvent.type(screen.getByLabelText("Recording timeout"), "1200{Enter}");
    expect(api.saveSettings).toHaveBeenCalledWith({
      knowledgeRoot: "D:/doc/knowledge",
      backend: "codex",
      fallback: false,
      models: { codex: null, claude: null },
      efforts: { codex: null, claude: null },
      modelTimeoutSecs: 1200,
      qmdTimeoutSecs: 900,
      maxChars: 30000,
      notesBudget: 60000,
    });
  });

  it("shows why an emptied limit was rejected and puts the saved value back", async () => {
    await openSettings();
    api.saveSettings.mockRejectedValue("Timeouts must be at least one second.");
    await userEvent.click(screen.getByRole("button", { name: "Show" }));
    await userEvent.clear(screen.getByLabelText("qmd timeout"));
    await userEvent.tab();
    expect(api.saveSettings).toHaveBeenCalledWith(expect.objectContaining({ qmdTimeoutSecs: 0 }));
    expect(await screen.findByText("Timeouts must be at least one second.")).toBeInTheDocument();
    expect(screen.getByLabelText("qmd timeout")).toHaveValue(900);
  });

  it("shows each connection and opens the integrations window to repair it or change the folder", async () => {
    await openSettings(snapshot({ connections: [{ provider: "claude", state: "connected" }, { provider: "codex", state: "partial" }] }));
    expect(screen.getByLabelText("Claude Code connection")).toHaveTextContent("Connected");
    expect(screen.getByLabelText("Codex connection")).toHaveTextContent("Needs repair");
    await userEvent.click(screen.getByRole("button", { name: "Manage connections" }));
    expect(api.openIntegrations).toHaveBeenLastCalledWith();
    expect(screen.getByText("D:/doc/knowledge")).toHaveAttribute("title", "D:/doc/knowledge");
    await userEvent.click(screen.getByRole("button", { name: "Change…" }));
    expect(api.openIntegrations).toHaveBeenLastCalledWith("folder");
  });

  it("toggles launch at login and auto-sync, and syncs from the sync row", async () => {
    await openSettings();
    await userEvent.click(screen.getByRole("switch", { name: "Launch at login" }));
    expect(api.setAutostart).toHaveBeenCalledWith(true);
    await userEvent.click(screen.getByRole("switch", { name: "Auto-sync" }));
    expect(api.setSyncEnabled).toHaveBeenCalledWith(false);
    const sync = screen.getByRole("region", { name: "Memory and sync" });
    expect(sync).toHaveTextContent("Not synced yet");
    await userEvent.click(within(sync).getByRole("button", { name: "Sync now" }));
    expect(api.syncNow).toHaveBeenCalledTimes(1);
  });

  const FOLDERS = [
    { name: "common", notes: 10 },
    { name: "lore", notes: 39 },
    { name: "qmd-memory", notes: 5 },
  ];
  const options = (select: HTMLElement) => within(select).getAllByRole("option").map((option) => option.textContent);

  it("cleans up the chosen folder and can merge another project folder into it", async () => {
    api.getCleanupFolders.mockResolvedValue(FOLDERS);
    await openSettings();
    const memory = screen.getByRole("region", { name: "Memory and sync" });
    expect(screen.queryByRole("combobox", { name: "Folder to clean up" })).not.toBeInTheDocument();
    await userEvent.click(within(memory).getByRole("button", { name: "Clean up…" }));
    const folder = await screen.findByRole("combobox", { name: "Folder to clean up" });
    await waitFor(() => expect(options(folder)).toEqual(["Choose a folder", "common · 10 notes", "lore · 39 notes", "qmd-memory · 5 notes"]));
    const start = screen.getByRole("button", { name: "Start cleanup" });
    expect(start).toBeDisabled();
    expect(screen.queryByRole("combobox", { name: "Merge another folder into it" })).not.toBeInTheDocument();

    await userEvent.selectOptions(folder, "lore");
    const merge = screen.getByRole("combobox", { name: "Merge another folder into it" });
    expect(options(merge)).toEqual(["None", "qmd-memory · 5 notes"]);
    await userEvent.selectOptions(merge, "qmd-memory");
    await userEvent.click(start);
    expect(api.startCleanup).toHaveBeenCalledWith("lore", ["qmd-memory"]);
  });

  it("merges nothing into the shared folder and starts a cleanup without a merge", async () => {
    api.getCleanupFolders.mockResolvedValue(FOLDERS);
    await openSettings();
    await userEvent.click(screen.getByRole("button", { name: "Clean up…" }));
    const folder = await screen.findByRole("combobox", { name: "Folder to clean up" });
    await waitFor(() => expect(options(folder)).toHaveLength(4));
    await userEvent.selectOptions(folder, "lore");
    await userEvent.selectOptions(screen.getByRole("combobox", { name: "Merge another folder into it" }), "qmd-memory");
    await userEvent.selectOptions(folder, "common");
    expect(screen.queryByRole("combobox", { name: "Merge another folder into it" })).not.toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Start cleanup" }));
    expect(api.startCleanup).toHaveBeenCalledWith("common", []);
  });

  it("says which folder is being cleaned up and starts no second cleanup meanwhile", async () => {
    await openSettings(snapshot({ cleanup: { folder: "lore", absorb: ["qmd-memory"], startedAt: 1_800_000_000 } }));
    const memory = screen.getByRole("region", { name: "Memory and sync" });
    expect(memory).toHaveTextContent("Cleaning up lore…");
    expect(within(memory).getByRole("button", { name: "Clean up…" })).toBeDisabled();
    expect(api.getCleanupFolders).not.toHaveBeenCalled();
  });

  it("says a cleanup is waiting until the current work is done", async () => {
    await openSettings(snapshot({ cleanup: { folder: "lore", absorb: [], startedAt: null } }));
    expect(screen.getByRole("region", { name: "Memory and sync" })).toHaveTextContent("lore cleanup starts when the current work is done.");
  });

  it("says a waiting cleanup needs recording to be resumed while recording is paused", async () => {
    await openSettings(snapshot({ cleanup: { folder: "lore", absorb: [], startedAt: null }, settings: { ...snapshot().settings, paused: true } }));
    expect(screen.getByRole("region", { name: "Memory and sync" })).toHaveTextContent("lore cleanup starts when recording is resumed.");
  });

  it("follows the chosen appearance", async () => {
    await openSettings();
    await userEvent.click(screen.getByRole("button", { name: "Dark" }));
    expect(document.documentElement).toHaveAttribute("data-theme", "dark");
    expect(localStorage.getItem("lore.theme")).toBe("dark");
    await userEvent.click(screen.getByRole("button", { name: "Light" }));
    expect(document.documentElement).toHaveAttribute("data-theme", "light");
  });

  it("runs the setup guide and opens the app data folder from Advanced", async () => {
    await openSettings();
    await userEvent.click(screen.getByRole("button", { name: "Show" }));
    await userEvent.click(screen.getByRole("button", { name: "Run setup guide again" }));
    expect(api.openSetup).toHaveBeenCalledTimes(1);
    await userEvent.click(screen.getByRole("button", { name: "Open app data folder" }));
    expect(api.openFolder).toHaveBeenCalledWith("data");
  });
});

describe("app updates", () => {
  const available = () => ({ phase: "available" as const, version: "0.2.0", notes: "A sample release.", downloaded: 0, total: null, error: null });

  it("opens an available release from the tray and only installs after an explicit click", async () => {
    serve(calm({ update: available() }));
    render(<App />);
    await userEvent.click(await screen.findByRole("button", { name: /Lore 0.2.0 available/ }));
    const updates = screen.getByRole("region", { name: "App updates" });
    expect(within(updates).getByText("A sample release.")).toBeInTheDocument();
    expect(api.installAppUpdate).not.toHaveBeenCalled();
    await userEvent.click(within(updates).getByRole("button", { name: "Install & restart" }));
    expect(api.installAppUpdate).toHaveBeenCalledTimes(1);
  });

  it("checks manually and reports the current version", async () => {
    const initial = calm();
    serve(initial);
    api.checkAppUpdate.mockImplementation(async () => {
      api.getSnapshot.mockResolvedValue({ ...initial, update: { ...initial.update, phase: "current" } });
    });
    render(<App />);
    await userEvent.click(await screen.findByRole("button", { name: "Settings" }));
    await userEvent.click(screen.getByRole("button", { name: "Check now" }));
    expect(await screen.findByText("You’re up to date.")).toBeInTheDocument();
    expect(api.installAppUpdate).not.toHaveBeenCalled();
  });

  it.each([
    { current: { jobId: "recording", provider: "codex" as const, project: "demo-app", startedAt: NOW } },
    { syncing: true },
    { reviewing: true },
  ])("prevents install while existing work is active: %j", async (work) => {
    serve(calm({ ...work, update: available() }));
    render(<App />);
    await userEvent.click(await screen.findByRole("button", { name: "Settings" }));
    const button = screen.getByRole("button", { name: "Install & restart" });
    expect(button).toBeDisabled();
    await userEvent.click(button);
    expect(api.installAppUpdate).not.toHaveBeenCalled();
  });

  it("shows a failed verification and allows retry without discarding the available release", async () => {
    serve(calm({ update: available() }));
    api.installAppUpdate.mockRejectedValue(new Error("Could not download or verify the update."));
    render(<App />);
    await userEvent.click(await screen.findByRole("button", { name: "Settings" }));
    await userEvent.click(screen.getByRole("button", { name: "Install & restart" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("Could not download or verify the update.");
    expect(screen.getByRole("button", { name: "Install & restart" })).toBeEnabled();
    expect(screen.getByText("Version 0.2.0 available")).toBeInTheDocument();
  });
});
