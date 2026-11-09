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

  it("keeps the full project, summary and error in a hover tooltip where the row cuts them off", async () => {
    api.getSnapshot.mockResolvedValue(snapshot());
    render(<App />);
    const attention = await screen.findByRole("region", { name: "확인 필요" });
    expect(within(attention).getByText("demoengine")).toHaveAttribute("title", "demoengine");
    expect(within(attention).getByText("근거 위치 또는 인용문이 제공된 원문과 일치하지 않습니다.")).toHaveAttribute("title", "근거 위치 또는 인용문이 제공된 원문과 일치하지 않습니다.");
    const recent = screen.getByRole("region", { name: "최근 기록" });
    expect(within(recent).getByText("demo-app")).toHaveAttribute("title", "demo-app");
    expect(within(recent).getByText("로그 전용으로 변경")).toHaveAttribute("title", "로그 전용으로 변경");
    expect(screen.getByText("대기 1건")).toHaveAttribute("title", "대기 1건");
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

  it("discards a failed job only after a second confirmation in a card that leaves the row as it was", async () => {
    api.getSnapshot.mockResolvedValue(snapshot());
    api.discardJob.mockResolvedValue(true);
    render(<App />);
    const attention = await screen.findByRole("region", { name: "확인 필요" });
    const trigger = within(attention).getByRole("button", { name: "버리기" });
    await userEvent.click(trigger);
    const confirm = screen.getByRole("alertdialog", { name: "실패 버리기 확인" });
    expect(confirm).toHaveTextContent("실패한 구간을 기록하지 않고 넘어갈까요?");
    expect(attention).not.toContainElement(confirm);
    expect(trigger).toHaveAttribute("aria-expanded", "true");
    expect(within(attention).getByRole("button", { name: "다시 시도" })).toBeInTheDocument();
    await userEvent.click(within(confirm).getByRole("button", { name: "취소" }));
    expect(api.discardJob).not.toHaveBeenCalled();
    expect(screen.queryByRole("alertdialog", { name: "실패 버리기 확인" })).not.toBeInTheDocument();
    expect(trigger).toHaveFocus();

    await userEvent.click(trigger);
    await userEvent.click(within(screen.getByRole("alertdialog", { name: "실패 버리기 확인" })).getByRole("button", { name: "버리기" }));
    expect(api.discardJob).toHaveBeenCalledWith("f1");
    expect(api.getSnapshot).toHaveBeenCalledTimes(2);
  });

  it("closes the confirmation card with Escape without hiding the popover, and by clicking elsewhere", async () => {
    api.getSnapshot.mockResolvedValue(snapshot());
    render(<App />);
    const attention = await screen.findByRole("region", { name: "확인 필요" });
    await userEvent.click(within(attention).getByRole("button", { name: "버리기" }));
    await userEvent.keyboard("{Escape}");
    expect(screen.queryByRole("alertdialog", { name: "실패 버리기 확인" })).not.toBeInTheDocument();
    expect(api.hideWindow).not.toHaveBeenCalled();

    await userEvent.click(screen.getByRole("button", { name: "최근 기록 지우기" }));
    expect(screen.getByRole("alertdialog", { name: "최근 기록 지우기 확인" })).toBeInTheDocument();
    await userEvent.click(screen.getByText("demo-app"));
    expect(screen.queryByRole("alertdialog", { name: "최근 기록 지우기 확인" })).not.toBeInTheDocument();
    expect(api.clearActivity).not.toHaveBeenCalled();

    await userEvent.keyboard("{Escape}");
    expect(api.hideWindow).toHaveBeenCalledTimes(1);
  });

  it("shows why discarding failed", async () => {
    api.getSnapshot.mockResolvedValue(snapshot());
    api.discardJob.mockRejectedValue("파일 오류: 다른 프로세스가 파일을 사용 중입니다.");
    render(<App />);
    const attention = await screen.findByRole("region", { name: "확인 필요" });
    await userEvent.click(within(attention).getByRole("button", { name: "버리기" }));
    await userEvent.click(within(screen.getByRole("alertdialog", { name: "실패 버리기 확인" })).getByRole("button", { name: "버리기" }));
    expect(await screen.findByText("파일 오류: 다른 프로세스가 파일을 사용 중입니다.")).toBeInTheDocument();
  });

  it("shows how many notes wait for review and opens the review window from that tile", async () => {
    api.getSnapshot.mockResolvedValue(snapshot());
    render(<App />);
    const tile = await screen.findByRole("button", { name: "검토 대기 4" });
    expect(screen.queryByText("완료")).not.toBeInTheDocument();
    await userEvent.click(tile);
    expect(api.openReview).toHaveBeenCalledTimes(1);
  });

  it("shows a dash for review when the memory store is not a git repository", async () => {
    api.getSnapshot.mockResolvedValue(snapshot({ review: null }));
    render(<App />);
    expect(await screen.findByRole("button", { name: "검토 대기 –" })).toBeInTheDocument();
  });

  it("says recording waits while the review window is open", async () => {
    api.getSnapshot.mockResolvedValue(snapshot({ state: "reviewing", reviewing: true }));
    render(<App />);
    expect(await screen.findByText("검토 중")).toBeInTheDocument();
    expect(screen.getByRole("region", { name: "진행 중" })).toHaveTextContent("검토 창이 열려 있는 동안에는 새 기록을 시작하지 않습니다");
  });

  it("clears the recent records only after a second confirmation", async () => {
    api.getSnapshot.mockResolvedValue(snapshot());
    api.clearActivity.mockResolvedValue(2);
    render(<App />);
    const trigger = await screen.findByRole("button", { name: "최근 기록 지우기" });
    await userEvent.click(trigger);
    const confirm = screen.getByRole("alertdialog", { name: "최근 기록 지우기 확인" });
    expect(confirm).toHaveTextContent("최근 기록을 모두 지울까요?");
    expect(screen.getByRole("region", { name: "최근 기록" })).not.toContainElement(confirm);
    expect(trigger).toBeInTheDocument();
    await userEvent.click(within(confirm).getByRole("button", { name: "취소" }));
    expect(api.clearActivity).not.toHaveBeenCalled();
    expect(screen.queryByRole("alertdialog", { name: "최근 기록 지우기 확인" })).not.toBeInTheDocument();

    await userEvent.click(trigger);
    await userEvent.click(within(screen.getByRole("alertdialog", { name: "최근 기록 지우기 확인" })).getByRole("button", { name: "모두 지우기" }));
    expect(api.clearActivity).toHaveBeenCalledTimes(1);
    expect(api.getSnapshot).toHaveBeenCalledTimes(2);
  });

  it("offers no clearing when there is nothing to clear", async () => {
    api.getSnapshot.mockResolvedValue(snapshot({ recent: [] }));
    render(<App />);
    await screen.findByText("아직 기록한 세션이 없습니다");
    expect(screen.queryByRole("button", { name: "최근 기록 지우기" })).not.toBeInTheDocument();
  });

  it("shows the session being recorded", async () => {
    api.getSnapshot.mockResolvedValue(snapshot({ state: "working", current: { jobId: "a", provider: "claude", project: "sample-app", startedAt: 1_800_000_000 } }));
    render(<App />);
    expect(await screen.findByText("기록 중 · sample-app")).toBeInTheDocument();
    expect(screen.getByRole("region", { name: "진행 중" })).toHaveTextContent("sample-app 세션을 정리하고 있습니다");
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
      const now = screen.getByRole("region", { name: "진행 중" });
      const recent = screen.getByRole("region", { name: "최근 기록" });
      expect(now).toHaveTextContent("2분째");
      expect(recent).toHaveTextContent("2분 전");
      await act(async () => {
        vi.advanceTimersByTime(60_000);
      });
      expect(now).toHaveTextContent("3분째");
      expect(recent).toHaveTextContent("3분 전");
      expect(api.getSnapshot).toHaveBeenCalledTimes(1);
    } finally {
      vi.useRealTimers();
    }
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
    job({ id: "retry", project: "demoengine", sessionId: "retry-session", attempts: 2, nextAttemptAt: 4_000_000_000, lastError: "모델 시간 초과" }),
    job({ id: "f1", provider: "codex", status: "failed", project: "sample-viewer", sessionId: "failed-session", attempts: 3, lastError: "원문 파일이 없습니다" }),
  ];
  const recording = { state: "working" as const, current: { jobId: "rec", provider: "claude" as const, project: "sample-app", startedAt: 1_800_000_000 } };

  it("opens from a count tile and shows each session's project, session id and state in the tiles' groups", async () => {
    api.getSnapshot.mockResolvedValue(snapshot(recording));
    api.getQueue.mockResolvedValue(queued);
    render(<App />);
    await userEvent.click(await screen.findByRole("button", { name: "대기 1" }));

    const now = await screen.findByRole("region", { name: "기록 중" });
    expect(now).toHaveTextContent("sample-app");
    expect(now).toHaveTextContent("rec-session");
    const ready = screen.getByRole("region", { name: "대기" });
    expect(within(ready).getByText("ae-recipe-wizard")).toBeInTheDocument();
    expect(within(ready).getByText("1f2e3d4c-aaaa-bbbb-cccc-000000000001")).toBeInTheDocument();
    expect(within(ready).getByText("이어서 기록")).toBeInTheDocument();
    expect(within(ready).getByText("rollout-x-0aa1.jsonl")).toHaveAttribute("title", "세션 rollout-x-0aa1.jsonl\nC:\\u\\.codex\\sessions\\rollout-x-0aa1.jsonl");
    expect(within(ready).getByText("Codex", { selector: ".name" })).toBeInTheDocument();
    const waiting = screen.getByRole("region", { name: "재시도 대기" });
    expect(waiting).toHaveTextContent("demoengine");
    expect(waiting).toHaveTextContent("2번 실패 · 모델 시간 초과");
    const failed = screen.getByRole("region", { name: "실패" });
    expect(failed).toHaveTextContent("원문 파일이 없습니다");
    expect(within(failed).getByRole("button", { name: "다시 시도" })).toBeInTheDocument();

    await userEvent.click(screen.getByRole("button", { name: "뒤로" }));
    expect(await screen.findByRole("region", { name: "큐 상태" })).toBeInTheDocument();
  });

  it("says a group is empty instead of hiding it", async () => {
    api.getSnapshot.mockResolvedValue(snapshot());
    api.getQueue.mockResolvedValue([]);
    render(<App />);
    await userEvent.click(await screen.findByRole("button", { name: "재시도 대기 2" }));
    expect(await screen.findByRole("region", { name: "재시도 대기" })).toHaveTextContent("다시 시도를 기다리는 세션이 없습니다.");
    expect(screen.getByRole("region", { name: "기록 중" })).toHaveTextContent("지금 기록하는 세션이 없습니다.");
  });

  it("scrolls to the group of the clicked tile once, not again on later updates", async () => {
    const scroll = vi.spyOn(Element.prototype, "scrollIntoView");
    try {
      api.getSnapshot.mockResolvedValue(snapshot());
      api.getQueue.mockResolvedValue(queued);
      render(<App />);
      await userEvent.click(await screen.findByRole("button", { name: "실패 1" }));
      const failed = await screen.findByRole("region", { name: "실패" });
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
    await userEvent.click(await screen.findByRole("button", { name: "설정" }));
    expect(screen.getByRole("button", { name: "저장" })).toBeDisabled();
    await userEvent.click(screen.getByRole("button", { name: "Claude" }));
    await userEvent.type(screen.getByLabelText("모델"), "   ");
    await userEvent.click(screen.getByRole("button", { name: "저장" }));
    expect(api.saveSettings).toHaveBeenCalledWith(expect.objectContaining({ backend: "claude", models: { codex: null, claude: null }, knowledgeRoot: "D:/doc/knowledge" }));
    expect(await screen.findByRole("button", { name: "설정" })).toBeInTheDocument();
  });

  it("keeps the model typed for each CLI while switching between them", async () => {
    api.getSnapshot.mockResolvedValue(snapshot({ settings: { ...snapshot().settings, backend: "claude", models: { codex: null, claude: "opus" } } }));
    api.saveSettings.mockResolvedValue(undefined);
    render(<App />);
    await userEvent.click(await screen.findByRole("button", { name: "설정" }));
    expect(screen.getByLabelText("모델")).toHaveValue("opus");
    await userEvent.click(screen.getByRole("button", { name: "Claude" }));
    expect(screen.getByLabelText("모델")).toHaveValue("opus");
    expect(screen.getByRole("button", { name: "저장" })).toBeDisabled();
    await userEvent.click(screen.getByRole("button", { name: "Codex" }));
    expect(screen.getByLabelText("모델")).toHaveValue("");
    await userEvent.type(screen.getByLabelText("모델"), "gpt-6.1-sol");
    await userEvent.click(screen.getByRole("button", { name: "Claude" }));
    expect(screen.getByLabelText("모델")).toHaveValue("opus");
    await userEvent.click(screen.getByRole("button", { name: "Codex" }));
    expect(screen.getByLabelText("모델")).toHaveValue("gpt-6.1-sol");
    await userEvent.click(screen.getByRole("button", { name: "저장" }));
    expect(api.saveSettings).toHaveBeenCalledWith(expect.objectContaining({ backend: "codex", models: { codex: "gpt-6.1-sol", claude: "opus" } }));
  });

  it("shows the processing limits and saves only the values the screen edits", async () => {
    api.getSnapshot.mockResolvedValue(snapshot());
    api.saveSettings.mockResolvedValue(undefined);
    render(<App />);
    await userEvent.click(await screen.findByRole("button", { name: "설정" }));
    expect(screen.getByLabelText("기록 모델 시간 제한(초)")).toHaveValue(600);
    expect(screen.getByLabelText("qmd 색인 시간 제한(초)")).toHaveValue(900);
    expect(screen.getByLabelText("한 번에 보내는 새 대화(자)")).toHaveValue(30000);
    expect(screen.getByLabelText("한 번에 보내는 기존 문서(자)")).toHaveValue(60000);
    await userEvent.clear(screen.getByLabelText("기록 모델 시간 제한(초)"));
    await userEvent.type(screen.getByLabelText("기록 모델 시간 제한(초)"), "1200");
    await userEvent.click(screen.getByRole("button", { name: "저장" }));
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
    api.saveSettings.mockRejectedValue("시간 제한은 1초 이상이어야 합니다.");
    render(<App />);
    await userEvent.click(await screen.findByRole("button", { name: "설정" }));
    expect(screen.getByRole("button", { name: "저장" })).toBeDisabled();
    await userEvent.clear(screen.getByLabelText("qmd 색인 시간 제한(초)"));
    expect(screen.getByRole("button", { name: "저장" })).toBeEnabled();
    await userEvent.click(screen.getByRole("button", { name: "저장" }));
    expect(api.saveSettings).toHaveBeenCalledWith(expect.objectContaining({ qmdTimeoutSecs: 0 }));
    expect(await screen.findByText("시간 제한은 1초 이상이어야 합니다.")).toBeInTheDocument();
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
