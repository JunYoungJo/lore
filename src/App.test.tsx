import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

import type { Snapshot } from "@/ipc/contract";

const api = vi.hoisted(() => ({
  getSnapshot: vi.fn(),
  setPaused: vi.fn(),
  retryJob: vi.fn(),
  saveSettings: vi.fn(),
  setAutostart: vi.fn(),
  connectHook: vi.fn(),
  disconnectHook: vi.fn(),
  openNote: vi.fn(),
  openFolder: vi.fn(),
  hideWindow: vi.fn(),
  onStateChanged: vi.fn(() => Promise.resolve(() => {})),
  onPopoverShown: vi.fn(() => Promise.resolve(() => {})),
}));

vi.mock("@/ipc/api", () => api);

import { App } from "@/App";
import { useAppStore } from "@/store/appStore";

const CLI = "C:/app/qmd-memory-cli.exe";

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
      { id: "f1", provider: "codex", transcript: "C:/s.jsonl", status: "failed", cursorLine: 0, cursorHash: "", project: "demoengine", sessionId: null, attempts: 3, nextAttemptAt: 0, lastError: "근거 위치 또는 인용문이 제공된 원문과 일치하지 않습니다.", updatedAt: 1 },
    ],
    problem: null,
    settings: {
      knowledgeRoot: "D:/doc/knowledge",
      backend: "codex",
      model: null,
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
    version: "0.1.0",
    ...overrides,
  };
}

beforeEach(() => {
  Object.values(api).forEach((mock) => mock.mockClear());
  useAppStore.setState({ snapshot: null, view: "status", draft: null, error: null, saving: false, connecting: null });
});

describe("status view", () => {
  it("shows queue counts, the failed job and recorded notes but hides skipped sessions", async () => {
    api.getSnapshot.mockResolvedValue(snapshot());
    render(<App />);
    expect(await screen.findByText("대기 1건")).toBeInTheDocument();
    const counts = screen.getByRole("region", { name: "큐 상태" });
    expect(within(within(counts).getByText("재시도 대기").parentElement!).getByText("2")).toBeInTheDocument();
    const attention = screen.getByRole("region", { name: "확인 필요" });
    expect(within(attention).getByText("demoengine")).toBeInTheDocument();
    expect(within(attention).getByText("근거 위치 또는 인용문이 제공된 원문과 일치하지 않습니다.")).toBeInTheDocument();
    const recent = screen.getByRole("region", { name: "최근 기록" });
    expect(within(recent).getByText("문서 1개 고침")).toBeInTheDocument();
    expect(within(recent).queryByText("작업자 세션")).not.toBeInTheDocument();
  });

  it("retries a failed job and opens a changed note", async () => {
    api.getSnapshot.mockResolvedValue(snapshot());
    api.retryJob.mockResolvedValue(true);
    render(<App />);
    await userEvent.click(await screen.findByRole("button", { name: "다시 시도" }));
    expect(api.retryJob).toHaveBeenCalledWith("f1");
    await userEvent.click(screen.getByRole("button", { name: "bg-policy.md 열기" }));
    expect(api.openNote).toHaveBeenCalledWith("projects/demo-app/bg-policy.md");
  });

  it("shows the session being recorded", async () => {
    api.getSnapshot.mockResolvedValue(snapshot({ state: "working", current: { jobId: "a", provider: "claude", project: "sample-app", startedAt: 1_800_000_000 } }));
    render(<App />);
    expect(await screen.findByText("기록 중 · sample-app")).toBeInTheDocument();
    expect(screen.getByRole("region", { name: "진행 중" })).toHaveTextContent("sample-app 세션을 정리하고 있습니다");
  });

  it("pauses from the header and offers to resume while paused", async () => {
    api.getSnapshot.mockResolvedValue(snapshot());
    const { unmount } = render(<App />);
    await userEvent.click(await screen.findByRole("button", { name: "일시 중지" }));
    expect(api.setPaused).toHaveBeenCalledWith(true);
    unmount();

    useAppStore.setState({ snapshot: null });
    api.getSnapshot.mockResolvedValue(snapshot({ state: "paused", settings: { ...snapshot().settings, paused: true } }));
    render(<App />);
    const now = await screen.findByRole("region", { name: "진행 중" });
    expect(now).toHaveTextContent("기록을 멈췄습니다");
    await userEvent.click(within(now).getByRole("button", { name: "다시 시작" }));
    expect(api.setPaused).toHaveBeenLastCalledWith(false);
  });

  it("shows an empty state before anything is recorded and asks to connect missing hooks", async () => {
    api.getSnapshot.mockResolvedValue(snapshot({ recent: [], attention: [] }));
    render(<App />);
    expect(await screen.findByText("아직 기록한 세션이 없습니다")).toBeInTheDocument();
    const callout = screen.getByRole("region", { name: "훅 연결 필요" });
    expect(callout).toHaveTextContent("Codex 훅 연결 필요");
    await userEvent.click(within(callout).getByRole("button", { name: "연결" }));
    expect(screen.getByText("기록 모델")).toBeInTheDocument();
  });

  it("does not ask to connect hooks when both are connected", async () => {
    const hooks = snapshot().hooks.map((hook) => ({ ...hook, state: "connected" as const }));
    api.getSnapshot.mockResolvedValue(snapshot({ hooks }));
    render(<App />);
    await screen.findByText("대기 1건");
    expect(screen.queryByRole("region", { name: "훅 연결 필요" })).not.toBeInTheDocument();
  });
});

describe("settings view", () => {
  it("saves a backend change with an empty model as the default and returns to status", async () => {
    api.getSnapshot.mockResolvedValue(snapshot());
    api.saveSettings.mockResolvedValue(undefined);
    render(<App />);
    await userEvent.click(await screen.findByRole("button", { name: "설정" }));
    expect(screen.getByRole("button", { name: "저장" })).toBeDisabled();
    await userEvent.click(screen.getByRole("button", { name: "Claude" }));
    await userEvent.type(screen.getByLabelText("모델"), "   ");
    await userEvent.click(screen.getByRole("button", { name: "저장" }));
    expect(api.saveSettings).toHaveBeenCalledWith(expect.objectContaining({ backend: "claude", model: null, knowledgeRoot: "D:/doc/knowledge" }));
    expect(await screen.findByRole("button", { name: "설정" })).toBeInTheDocument();
  });

  it("keeps the settings open and shows the error when saving fails", async () => {
    api.getSnapshot.mockResolvedValue(snapshot());
    api.saveSettings.mockRejectedValue("기억 저장소 위치가 비어 있습니다.");
    render(<App />);
    await userEvent.click(await screen.findByRole("button", { name: "설정" }));
    await userEvent.clear(screen.getByLabelText("위치"));
    await userEvent.click(screen.getByRole("button", { name: "저장" }));
    expect(await screen.findByText("기억 저장소 위치가 비어 있습니다.")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "저장" })).toBeInTheDocument();
  });

  it("offers connect for a missing hook, disconnect for a connected one, and toggles autostart", async () => {
    api.getSnapshot.mockResolvedValue(snapshot());
    api.connectHook.mockResolvedValue(snapshot().hooks[1]);
    api.disconnectHook.mockResolvedValue(snapshot().hooks[0]);
    render(<App />);
    await userEvent.click(await screen.findByRole("button", { name: "설정" }));
    const claude = screen.getByLabelText("Claude 훅");
    expect(within(claude).getByText("연결됨")).toBeInTheDocument();
    expect(within(claude).queryByRole("button", { name: "연결" })).not.toBeInTheDocument();
    const codex = screen.getByLabelText("Codex 훅");
    expect(within(codex).queryByRole("button", { name: "해제" })).not.toBeInTheDocument();
    await userEvent.click(within(codex).getByRole("button", { name: "연결" }));
    expect(api.connectHook).toHaveBeenCalledWith("codex");
    await userEvent.click(within(screen.getByLabelText("Claude 훅")).getByRole("button", { name: "해제" }));
    expect(api.disconnectHook).toHaveBeenCalledWith("claude");
    await userEvent.click(screen.getByRole("switch", { name: "Windows 로그인 시 자동 실행" }));
    expect(api.setAutostart).toHaveBeenCalledWith(true);
  });
});
