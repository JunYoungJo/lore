import { act, render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

import type { Job, Snapshot } from "@/ipc/contract";

const api = vi.hoisted(() => ({
  getSnapshot: vi.fn(),
  getQueue: vi.fn(),
  setPaused: vi.fn(),
  retryJob: vi.fn(),
  discardJob: vi.fn(),
  saveSettings: vi.fn(),
  setAutostart: vi.fn(),
  connectHook: vi.fn(),
  disconnectHook: vi.fn(),
  clearActivity: vi.fn(),
  openReview: vi.fn(),
  openNote: vi.fn(),
  openFolder: vi.fn(),
  hideWindow: vi.fn(),
  onStateChanged: vi.fn(() => Promise.resolve(() => {})),
  onPopoverShown: vi.fn(() => Promise.resolve(() => {})),
}));

vi.mock("@/ipc/api", () => api);

import { App } from "@/App";
import { useAppStore } from "@/store/appStore";

const CLI = "C:/app/lore-cli.exe";

function snapshot(overrides: Partial<Snapshot> = {}): Snapshot {
  return {
    state: "idle",
    current: null,
    counts: { ready: 1, waiting: 2, failed: 1, done: 5, skipped: 0 },
    recent: [
      { id: 2, at: 1_800_000_000, jobId: "a", provider: "claude", project: "demo-app", kind: "recorded", summary: "로그 전용으로 변경", notes: ["projects/demo-app/bg-policy.md"], error: null },
      { id: 1, at: 1_799_999_000, jobId: "b", provider: "codex", project: null, kind: "skipped", summary: "작업자 세션", notes: [], error: null },
    ],
    attention: [
      { id: "f1", provider: "codex", transcript: "C:/s.jsonl", status: "failed", cursorLine: 0, cursorHash: "", project: "demoengine", sessionId: null, attempts: 3, nextAttemptAt: 0, lastError: "The evidence location or quotation does not match the provided source.", updatedAt: 1 },
    ],
    problem: null,
    settings: {
      knowledgeRoot: "D:/doc/knowledge",
      backend: "codex",
      models: { codex: null, claude: null },
      modelTimeoutSecs: 600,
      qmdTimeoutSecs: 900,
      claudeProjectsDir: "C:/u/.claude/projects",
      codexSessionsDir: "C:/u/.codex/sessions",
      idleSecs: 600,
      since: 1,
      paused: false,
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
    review: 4,
    reviewing: false,
    version: "0.1.0",
    ...overrides,
  };
}

beforeEach(() => {
  Object.values(api).forEach((mock) => mock.mockClear());
  useAppStore.setState({ snapshot: null, view: "status", draft: null, error: null, saving: false, connecting: null, queue: null, queueFocus: "ready" });
});

describe("status view", () => {
  it("shows queue counts, the failed job and recorded notes but hides skipped sessions", async () => {
    api.getSnapshot.mockResolvedValue(snapshot());
    render(<App />);
    expect(await screen.findByText("1 pending")).toBeInTheDocument();
    const counts = screen.getByRole("region", { name: "Queue status" });
    expect(within(within(counts).getByText("Retrying").parentElement!).getByText("2")).toBeInTheDocument();
    const attention = screen.getByRole("region", { name: "Needs attention" });
    expect(within(attention).getByText("demoengine")).toBeInTheDocument();
    expect(within(attention).getByText("The evidence location or quotation does not match the provided source.")).toBeInTheDocument();
    const recent = screen.getByRole("region", { name: "Recent activity" });
    expect(within(recent).getByText("1 note updated")).toBeInTheDocument();
    expect(within(recent).queryByText("작업자 세션")).not.toBeInTheDocument();
  });

  it("keeps the full project, summary and error in a hover tooltip where the row cuts them off", async () => {
    api.getSnapshot.mockResolvedValue(snapshot());
    render(<App />);
    const attention = await screen.findByRole("region", { name: "Needs attention" });
    expect(within(attention).getByText("demoengine")).toHaveAttribute("title", "demoengine");
    expect(within(attention).getByText("The evidence location or quotation does not match the provided source.")).toHaveAttribute("title", "The evidence location or quotation does not match the provided source.");
    const recent = screen.getByRole("region", { name: "Recent activity" });
    expect(within(recent).getByText("demo-app")).toHaveAttribute("title", "demo-app");
    expect(within(recent).getByText("로그 전용으로 변경")).toHaveAttribute("title", "로그 전용으로 변경");
    expect(screen.getByText("1 pending")).toHaveAttribute("title", "1 pending");
  });

  it("retries a failed job and opens a changed note", async () => {
    api.getSnapshot.mockResolvedValue(snapshot());
    api.retryJob.mockResolvedValue(true);
    render(<App />);
    await userEvent.click(await screen.findByRole("button", { name: "Retry" }));
    expect(api.retryJob).toHaveBeenCalledWith("f1");
    await userEvent.click(screen.getByRole("button", { name: "Open bg-policy.md" }));
    expect(api.openNote).toHaveBeenCalledWith("projects/demo-app/bg-policy.md");
  });

  it("discards a failed job only after a second confirmation in a card that leaves the row as it was", async () => {
    api.getSnapshot.mockResolvedValue(snapshot());
    api.discardJob.mockResolvedValue(true);
    render(<App />);
    const attention = await screen.findByRole("region", { name: "Needs attention" });
    const trigger = within(attention).getByRole("button", { name: "Skip" });
    await userEvent.click(trigger);
    const confirm = screen.getByRole("alertdialog", { name: "Skip failed segment confirmation" });
    expect(confirm).toHaveTextContent("Skip the failed segment?");
    expect(attention).not.toContainElement(confirm);
    expect(trigger).toHaveAttribute("aria-expanded", "true");
    expect(within(attention).getByRole("button", { name: "Retry" })).toBeInTheDocument();
    await userEvent.click(within(confirm).getByRole("button", { name: "Cancel" }));
    expect(api.discardJob).not.toHaveBeenCalled();
    expect(screen.queryByRole("alertdialog", { name: "Skip failed segment confirmation" })).not.toBeInTheDocument();
    expect(trigger).toHaveFocus();

    await userEvent.click(trigger);
    await userEvent.click(within(screen.getByRole("alertdialog", { name: "Skip failed segment confirmation" })).getByRole("button", { name: "Skip" }));
    expect(api.discardJob).toHaveBeenCalledWith("f1");
    expect(api.getSnapshot).toHaveBeenCalledTimes(2);
  });

  it("closes the confirmation card with Escape without hiding the popover, and by clicking elsewhere", async () => {
    api.getSnapshot.mockResolvedValue(snapshot());
    render(<App />);
    const attention = await screen.findByRole("region", { name: "Needs attention" });
    await userEvent.click(within(attention).getByRole("button", { name: "Skip" }));
    await userEvent.keyboard("{Escape}");
    expect(screen.queryByRole("alertdialog", { name: "Skip failed segment confirmation" })).not.toBeInTheDocument();
    expect(api.hideWindow).not.toHaveBeenCalled();

    await userEvent.click(screen.getByRole("button", { name: "Clear history" }));
    expect(screen.getByRole("alertdialog", { name: "Clear history confirmation" })).toBeInTheDocument();
    await userEvent.click(screen.getByText("demo-app"));
    expect(screen.queryByRole("alertdialog", { name: "Clear history confirmation" })).not.toBeInTheDocument();
    expect(api.clearActivity).not.toHaveBeenCalled();

    await userEvent.keyboard("{Escape}");
    expect(api.hideWindow).toHaveBeenCalledTimes(1);
  });

  it("shows why discarding failed", async () => {
    api.getSnapshot.mockResolvedValue(snapshot());
    api.discardJob.mockRejectedValue("File error: the file is in use by another process.");
    render(<App />);
    const attention = await screen.findByRole("region", { name: "Needs attention" });
    await userEvent.click(within(attention).getByRole("button", { name: "Skip" }));
    await userEvent.click(within(screen.getByRole("alertdialog", { name: "Skip failed segment confirmation" })).getByRole("button", { name: "Skip" }));
    expect(await screen.findByText("File error: the file is in use by another process.")).toBeInTheDocument();
  });

  it("shows how many notes wait for review and opens the review window from that tile", async () => {
    api.getSnapshot.mockResolvedValue(snapshot());
    render(<App />);
    const tile = await screen.findByRole("button", { name: "To review 4" });
    expect(screen.queryByText("완료")).not.toBeInTheDocument();
    await userEvent.click(tile);
    expect(api.openReview).toHaveBeenCalledTimes(1);
  });

  it("shows a dash for review when the memory store is not a git repository", async () => {
    api.getSnapshot.mockResolvedValue(snapshot({ review: null }));
    render(<App />);
    expect(await screen.findByRole("button", { name: "To review –" })).toBeInTheDocument();
  });

  it("says recording waits while the review window is open", async () => {
    api.getSnapshot.mockResolvedValue(snapshot({ state: "reviewing", reviewing: true }));
    render(<App />);
    expect(await screen.findByText("Reviewing")).toBeInTheDocument();
    expect(screen.getByRole("region", { name: "Current activity" })).toHaveTextContent("New recordings wait while the review window is open.");
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
    expect(trigger).toBeInTheDocument();
    await userEvent.click(within(confirm).getByRole("button", { name: "Cancel" }));
    expect(api.clearActivity).not.toHaveBeenCalled();
    expect(screen.queryByRole("alertdialog", { name: "Clear history confirmation" })).not.toBeInTheDocument();

    await userEvent.click(trigger);
    await userEvent.click(within(screen.getByRole("alertdialog", { name: "Clear history confirmation" })).getByRole("button", { name: "Clear all" }));
    expect(api.clearActivity).toHaveBeenCalledTimes(1);
    expect(api.getSnapshot).toHaveBeenCalledTimes(2);
  });

  it("offers no clearing when there is nothing to clear", async () => {
    api.getSnapshot.mockResolvedValue(snapshot({ recent: [] }));
    render(<App />);
    await screen.findByText("No sessions recorded yet");
    expect(screen.queryByRole("button", { name: "Clear history" })).not.toBeInTheDocument();
  });

  it("shows the session being recorded", async () => {
    api.getSnapshot.mockResolvedValue(snapshot({ state: "working", current: { jobId: "a", provider: "claude", project: "sample-app", startedAt: 1_800_000_000 } }));
    render(<App />);
    expect(await screen.findByText("Recording · sample-app")).toBeInTheDocument();
    expect(screen.getByRole("region", { name: "Current activity" })).toHaveTextContent("Recording sample-app");
  });

  it("keeps elapsed and relative times moving while the popover stays open", async () => {
    vi.useFakeTimers();
    try {
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
    } finally {
      vi.useRealTimers();
    }
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

  it("shows an empty state before anything is recorded and asks to connect missing hooks", async () => {
    api.getSnapshot.mockResolvedValue(snapshot({ recent: [], attention: [] }));
    render(<App />);
    expect(await screen.findByText("No sessions recorded yet")).toBeInTheDocument();
    const callout = screen.getByRole("region", { name: "Hooks need connecting" });
    expect(callout).toHaveTextContent("Connect Codex hooks");
    await userEvent.click(within(callout).getByRole("button", { name: "Connect" }));
    expect(screen.getByText("Recording model")).toBeInTheDocument();
  });

  it("does not ask to connect hooks when both are connected", async () => {
    const hooks = snapshot().hooks.map((hook) => ({ ...hook, state: "connected" as const }));
    api.getSnapshot.mockResolvedValue(snapshot({ hooks }));
    render(<App />);
    await screen.findByText("1 pending");
    expect(screen.queryByRole("region", { name: "Hooks need connecting" })).not.toBeInTheDocument();
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

  it("opens from a count tile and shows each session's project, session id and state in the tiles' groups", async () => {
    api.getSnapshot.mockResolvedValue(snapshot(recording));
    api.getQueue.mockResolvedValue(queued);
    render(<App />);
    await userEvent.click(await screen.findByRole("button", { name: "Pending 1" }));

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
    expect(within(failed).getByRole("button", { name: "Retry" })).toBeInTheDocument();

    await userEvent.click(screen.getByRole("button", { name: "Back" }));
    expect(await screen.findByRole("region", { name: "Queue status" })).toBeInTheDocument();
  });

  it("says a group is empty instead of hiding it", async () => {
    api.getSnapshot.mockResolvedValue(snapshot());
    api.getQueue.mockResolvedValue([]);
    render(<App />);
    await userEvent.click(await screen.findByRole("button", { name: "Retrying 2" }));
    expect(await screen.findByRole("region", { name: "Retrying" })).toHaveTextContent("No sessions are waiting for a retry.");
    expect(screen.getByRole("region", { name: "Recording" })).toHaveTextContent("No sessions are being recorded.");
  });

  it("scrolls to the group of the clicked tile once, not again on later updates", async () => {
    const scroll = vi.spyOn(Element.prototype, "scrollIntoView");
    try {
      api.getSnapshot.mockResolvedValue(snapshot());
      api.getQueue.mockResolvedValue(queued);
      render(<App />);
      await userEvent.click(await screen.findByRole("button", { name: "Failed 1" }));
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
});

describe("settings view", () => {
  it("saves a backend change with an empty model as the default and returns to status", async () => {
    api.getSnapshot.mockResolvedValue(snapshot());
    api.saveSettings.mockResolvedValue(undefined);
    render(<App />);
    await userEvent.click(await screen.findByRole("button", { name: "Settings" }));
    expect(screen.getByRole("button", { name: "Save" })).toBeDisabled();
    await userEvent.click(screen.getByRole("button", { name: "Claude" }));
    await userEvent.type(screen.getByLabelText("Model"), "   ");
    await userEvent.click(screen.getByRole("button", { name: "Save" }));
    expect(api.saveSettings).toHaveBeenCalledWith(expect.objectContaining({ backend: "claude", models: { codex: null, claude: null }, knowledgeRoot: "D:/doc/knowledge" }));
    expect(await screen.findByRole("button", { name: "Settings" })).toBeInTheDocument();
  });

  it("keeps the model typed for each CLI while switching between them", async () => {
    api.getSnapshot.mockResolvedValue(snapshot({ settings: { ...snapshot().settings, backend: "claude", models: { codex: null, claude: "opus" } } }));
    api.saveSettings.mockResolvedValue(undefined);
    render(<App />);
    await userEvent.click(await screen.findByRole("button", { name: "Settings" }));
    expect(screen.getByLabelText("Model")).toHaveValue("opus");
    await userEvent.click(screen.getByRole("button", { name: "Claude" }));
    expect(screen.getByLabelText("Model")).toHaveValue("opus");
    expect(screen.getByRole("button", { name: "Save" })).toBeDisabled();
    await userEvent.click(screen.getByRole("button", { name: "Codex" }));
    expect(screen.getByLabelText("Model")).toHaveValue("");
    await userEvent.type(screen.getByLabelText("Model"), "gpt-6.1-sol");
    await userEvent.click(screen.getByRole("button", { name: "Claude" }));
    expect(screen.getByLabelText("Model")).toHaveValue("opus");
    await userEvent.click(screen.getByRole("button", { name: "Codex" }));
    expect(screen.getByLabelText("Model")).toHaveValue("gpt-6.1-sol");
    await userEvent.click(screen.getByRole("button", { name: "Save" }));
    expect(api.saveSettings).toHaveBeenCalledWith(expect.objectContaining({ backend: "codex", models: { codex: "gpt-6.1-sol", claude: "opus" } }));
  });

  it("shows the processing limits and saves only the values the screen edits", async () => {
    api.getSnapshot.mockResolvedValue(snapshot());
    api.saveSettings.mockResolvedValue(undefined);
    render(<App />);
    await userEvent.click(await screen.findByRole("button", { name: "Settings" }));
    expect(screen.getByLabelText("Recording timeout (seconds)")).toHaveValue(600);
    expect(screen.getByLabelText("qmd timeout (seconds)")).toHaveValue(900);
    expect(screen.getByLabelText("New conversation per request (characters)")).toHaveValue(30000);
    expect(screen.getByLabelText("Existing notes per request (characters)")).toHaveValue(60000);
    await userEvent.clear(screen.getByLabelText("Recording timeout (seconds)"));
    await userEvent.type(screen.getByLabelText("Recording timeout (seconds)"), "1200");
    await userEvent.click(screen.getByRole("button", { name: "Save" }));
    expect(api.saveSettings).toHaveBeenCalledWith({
      knowledgeRoot: "D:/doc/knowledge",
      backend: "codex",
      models: { codex: null, claude: null },
      modelTimeoutSecs: 1200,
      qmdTimeoutSecs: 900,
      maxChars: 30000,
      notesBudget: 60000,
    });
  });

  it("enables saving only after a limit changes and sends an emptied limit as zero for the app to reject", async () => {
    api.getSnapshot.mockResolvedValue(snapshot());
    api.saveSettings.mockRejectedValue("Timeouts must be at least one second.");
    render(<App />);
    await userEvent.click(await screen.findByRole("button", { name: "Settings" }));
    expect(screen.getByRole("button", { name: "Save" })).toBeDisabled();
    await userEvent.clear(screen.getByLabelText("qmd timeout (seconds)"));
    expect(screen.getByRole("button", { name: "Save" })).toBeEnabled();
    await userEvent.click(screen.getByRole("button", { name: "Save" }));
    expect(api.saveSettings).toHaveBeenCalledWith(expect.objectContaining({ qmdTimeoutSecs: 0 }));
    expect(await screen.findByText("Timeouts must be at least one second.")).toBeInTheDocument();
  });

  it("keeps the settings open and shows the error when saving fails", async () => {
    api.getSnapshot.mockResolvedValue(snapshot());
    api.saveSettings.mockRejectedValue("The memory folder path is empty.");
    render(<App />);
    await userEvent.click(await screen.findByRole("button", { name: "Settings" }));
    await userEvent.clear(screen.getByLabelText("Location"));
    await userEvent.click(screen.getByRole("button", { name: "Save" }));
    expect(await screen.findByText("The memory folder path is empty.")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Save" })).toBeInTheDocument();
  });

  it("offers connect for a missing hook, disconnect for a connected one, and toggles autostart", async () => {
    api.getSnapshot.mockResolvedValue(snapshot());
    api.connectHook.mockResolvedValue(snapshot().hooks[1]);
    api.disconnectHook.mockResolvedValue(snapshot().hooks[0]);
    render(<App />);
    await userEvent.click(await screen.findByRole("button", { name: "Settings" }));
    const claude = screen.getByLabelText("Claude hooks");
    expect(within(claude).getByText("Connected")).toBeInTheDocument();
    expect(within(claude).queryByRole("button", { name: "Connect" })).not.toBeInTheDocument();
    const codex = screen.getByLabelText("Codex hooks");
    expect(within(codex).queryByRole("button", { name: "Disconnect" })).not.toBeInTheDocument();
    await userEvent.click(within(codex).getByRole("button", { name: "Connect" }));
    expect(api.connectHook).toHaveBeenCalledWith("codex");
    await userEvent.click(within(screen.getByLabelText("Claude hooks")).getByRole("button", { name: "Disconnect" }));
    expect(api.disconnectHook).toHaveBeenCalledWith("claude");
    await userEvent.click(screen.getByRole("switch", { name: "Launch at login" }));
    expect(api.setAutostart).toHaveBeenCalledWith(true);
  });
});
