import { act, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { DiffRow, SyncPlan, SyncSnapshot } from "@/ipc/contract";

const api = vi.hoisted(() => ({
  getSync: vi.fn(), setSyncEnabled: vi.fn(), prepareSync: vi.fn(), analyzeSync: vi.fn(), previewSync: vi.fn(), applySync: vi.fn(), resumeSync: vi.fn(), deferSync: vi.fn(),
  openReview: vi.fn(), openSettings: vi.fn(), syncNow: vi.fn(), onStateChanged: vi.fn(() => Promise.resolve(() => {})), onSyncComparisonProgress: vi.fn((_handler: (progress: unknown) => void) => Promise.resolve(() => {})),
}));
vi.mock("@/ipc/api", () => api);
import { SyncApp } from "@/SyncApp";

const NOW = 1_800_000_600;
const note = "projects/demo/policy.md";
function plan(): SyncPlan {
  return { id: "comparison-1", root: "/knowledge", branch: "main", remoteName: "origin", remoteRef: "refs/heads/main", local: "aaaa1111", remote: "bbbb2222", base: "cccc3333", tree: "dddd4444", phase: "review", approved: null, message: null,
    files: [{ path: note, base: "미정", local: "로그만", remote: "팝업", merged: "원래 문구", textConflict: false }],
    sources: [{ id: "local:source", side: "local", path: "sources/claude/a.md", text: "사용자: 팝업은 빼고 로그만 남겨." }, { id: "remote:source", side: "remote", path: "sources/codex/b.md", text: "사용자: 오류 팝업을 표시해." }],
    unsupported: [], otherFiles: ["sources/claude/a.md"],
    analysis: { summary: "알림 방침이 서로 다릅니다. 사용자의 최종 확인이 필요합니다.", issues: [{ title: "오류 알림 방침", explanation: "양쪽 문서에서 반대되는 방침을 기록했습니다.", evidence: [{ source: "local:source", quote: "팝업은 빼고 로그만 남겨." }, { source: "remote:source", quote: "오류 팝업을 표시해." }], options: [
      { label: "로그 전용", description: "팝업 없이 로그만 남깁니다.", edits: [{ path: note, find: "원래 문구", text: "로그만" }] },
      { label: "팝업 사용", description: "오류 팝업을 표시합니다.", edits: [{ path: note, find: "원래 문구", text: "팝업" }] },
    ] }] },
  };
}
function snapshot(overrides: Partial<SyncSnapshot> = {}): SyncSnapshot {
  return { root: "/knowledge", enabled: true, busy: false, summary: { phase: "review", message: null, blocked: true, issues: 1, lastSyncedAt: null }, plan: plan(),
    tracking: "main ↔ origin/main", model: "Claude opus", review: null, changes: [{ path: note, local: { added: 1, removed: 1 }, remote: { added: 2, removed: 1 } }], ...overrides };
}
const idle = (overrides: Partial<SyncSnapshot> = {}) => snapshot({ plan: null, changes: [], summary: { phase: "idle", message: null, blocked: false, issues: 0, lastSyncedAt: null }, ...overrides });
const diff = (text: string): DiffRow[] => [{ kind: "changed", leftNo: 1, left: [{ kind: "removed", text: "로그만" }], rightNo: 1, right: [{ kind: "added", text }] }];

beforeEach(() => {
  Object.values(api).forEach((mock) => mock.mockReset());
  api.onStateChanged.mockResolvedValue(() => {});
  api.onSyncComparisonProgress.mockResolvedValue(() => {});
  api.getSync.mockResolvedValue(snapshot());
  api.previewSync.mockImplementation((_id: string, choices: { option: number; customText: string | null }[]) => {
    const text = choices[0]?.customText ?? (choices[0]?.option === 0 ? "최종: 로그만" : "최종: 팝업");
    return Promise.resolve({ files: { [note]: text }, diffs: { [note]: diff(text) }, token: "confirmed-preview" });
  });
  api.applySync.mockResolvedValue(undefined);
  api.deferSync.mockResolvedValue(undefined);
});

afterEach(() => {
  vi.useRealTimers();
});

describe("sync review", () => {
  it("shows provenance, then requires every choice and a server preview before applying", async () => {
    render(<SyncApp />);
    const issue = await screen.findByRole("region", { name: "오류 알림 방침" });
    expect(screen.getByText("Review needed")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /Check result/ })).toBeDisabled();
    expect(within(issue).getByText("팝업은 빼고 로그만 남겨.")).toBeInTheDocument();
    await userEvent.click(within(issue).getAllByText("View source conversation", { selector: "summary" })[0]);
    expect(screen.getByText("사용자: 팝업은 빼고 로그만 남겨.")).toBeInTheDocument();
    await userEvent.click(screen.getByRole("radio", { name: /로그 전용/ }));
    expect(screen.getByRole("button", { name: /오류 알림 방침/ })).toHaveTextContent("로그 전용");
    expect(screen.getByText("1 of 1 decided")).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: /Check result/ }));
    const result = screen.getByRole("region", { name: "Final check" });
    expect(await within(result).findByText("최종: 로그만")).toBeInTheDocument();
    expect(within(result).getByText("This PC now")).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Apply & share" }));
    expect(api.applySync).toHaveBeenCalledWith("comparison-1", [{ issue: 0, option: 0, customText: null }], "confirmed-preview");
  });

  it("lets the user edit the replacement text while preserving the target span", async () => {
    render(<SyncApp />);
    await userEvent.click(await screen.findByRole("radio", { name: /로그 전용/ }));
    await userEvent.click(screen.getByRole("checkbox", { name: "Edit the selected wording" }));
    const input = screen.getByRole("textbox", { name: "Final wording" });
    await userEvent.clear(input);
    await userEvent.type(input, "운영은 로그만, 점검 중 팝업");
    await waitFor(() => expect(api.previewSync).toHaveBeenLastCalledWith("comparison-1", [{ issue: 0, option: 0, customText: "운영은 로그만, 점검 중 팝업" }]));
    await userEvent.click(screen.getByRole("button", { name: /Check result/ }));
    expect(await screen.findByText("운영은 로그만, 점검 중 팝업", { selector: "ins" })).toBeInTheDocument();
    expect(api.applySync).not.toHaveBeenCalled();
  });

  it("does not apply after a preview validation failure", async () => {
    api.previewSync.mockRejectedValue(new Error("Evidence 링크가 빠졌습니다"));
    render(<SyncApp />);
    await userEvent.click(await screen.findByRole("radio", { name: /로그 전용/ }));
    await userEvent.click(screen.getByRole("button", { name: /Check result/ }));
    expect(await screen.findByRole("alert")).toHaveTextContent("Evidence 링크가 빠졌습니다");
    expect(screen.getByRole("button", { name: "Apply & share" })).toBeDisabled();
  });

  it("discards a slow preview when the choice changes", async () => {
    let resolveOld: ((value: { files: Record<string, string>; diffs: Record<string, DiffRow[]>; token: string }) => void) | undefined;
    api.previewSync.mockImplementationOnce(() => new Promise((resolve) => { resolveOld = resolve; }));
    render(<SyncApp />);
    await userEvent.click(await screen.findByRole("radio", { name: /로그 전용/ }));
    await waitFor(() => expect(api.previewSync).toHaveBeenCalledTimes(1));
    await userEvent.click(screen.getByRole("radio", { name: /팝업 사용/ }));
    await userEvent.click(screen.getByRole("button", { name: /Check result/ }));
    await screen.findByText("최종: 팝업");
    await act(async () => { resolveOld?.({ files: { [note]: "잘못된 오래된 미리보기" }, diffs: { [note]: diff("잘못된 오래된 미리보기") }, token: "old" }); });
    expect(screen.queryByText("잘못된 오래된 미리보기")).not.toBeInTheDocument();
  });

  it("defers the review from Later and from the header without applying", async () => {
    render(<SyncApp />);
    await screen.findByRole("region", { name: "오류 알림 방침" });
    await userEvent.click(screen.getByRole("button", { name: "Later" }));
    await userEvent.click(screen.getByRole("button", { name: /Close/ }));
    expect(api.deferSync).toHaveBeenCalledTimes(2);
    expect(api.applySync).not.toHaveBeenCalled();
  });

  it("explains the comparison before running the model, then shows the decisions", async () => {
    api.getSync.mockResolvedValueOnce(snapshot({ plan: { ...plan(), analysis: null } })).mockResolvedValue(snapshot());
    api.analyzeSync.mockResolvedValue(undefined);
    render(<SyncApp />);
    const overview = await screen.findByRole("region", { name: "Sync review" });
    expect(overview).toHaveTextContent("Claude opus reads both versions");
    expect(screen.getByRole("button", { name: /policy\.md/ })).toHaveTextContent("demo · This PC +1 −1 · Other PCs +2 −1");
    await userEvent.click(within(overview).getByRole("button", { name: "Compare with model" }));
    expect(api.analyzeSync).toHaveBeenCalledOnce();
    expect(await screen.findByRole("region", { name: "오류 알림 방침" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /Check result/ })).toBeDisabled();
  });

  it("shows the raw differences of a note before comparing", async () => {
    api.getSync.mockResolvedValue(snapshot({ plan: { ...plan(), analysis: null } }));
    render(<SyncApp />);
    await userEvent.click(await screen.findByRole("button", { name: "View raw differences" }));
    const raw = screen.getByRole("region", { name: `Differences in ${note}` });
    expect(within(raw).getByText("로그만")).toBeInTheDocument();
    expect(within(raw).getByText("팝업")).toBeInTheDocument();
  });

  it("resumes a saved comparison and shows its previous failure", async () => {
    api.getSync.mockResolvedValueOnce(snapshot({ plan: { ...plan(), analysis: null }, comparison: { completed: 2, total: 6, stage: "paused", reads: 5, error: "Provider unavailable" } })).mockResolvedValue(snapshot());
    api.analyzeSync.mockResolvedValue(undefined);
    render(<SyncApp />);
    expect(await screen.findByRole("status")).toHaveTextContent("2 of 6 groups compared");
    expect(screen.getByRole("alert")).toHaveTextContent("Provider unavailable");
    await userEvent.click(screen.getByRole("button", { name: "Resume comparison" }));
    expect(api.analyzeSync).toHaveBeenCalledOnce();
    expect(await screen.findByRole("region", { name: "오류 알림 방침" })).toBeInTheDocument();
    expect(api.applySync).not.toHaveBeenCalled();
  });

  it("updates comparison progress without downloading the whole review again", async () => {
    api.getSync.mockResolvedValue(snapshot({ plan: { ...plan(), analysis: null } }));
    api.analyzeSync.mockImplementation(() => new Promise(() => {}));
    render(<SyncApp />);
    await userEvent.click(await screen.findByRole("button", { name: "Compare with model" }));
    const update = api.onSyncComparisonProgress.mock.calls[0][0] as (value: unknown) => void;
    await act(async () => update({ completed: 2, total: 6, stage: "comparing", reads: 8, error: null }));
    expect(screen.getAllByRole("status").some((node) => node.textContent?.includes("2 of 6 groups compared"))).toBe(true);
    await act(async () => update({ completed: 6, total: 6, stage: "checking", reads: 9, error: null }));
    expect(screen.getAllByRole("status").some((node) => node.textContent?.includes("Checking consistency across 6 groups"))).toBe(true);
    expect(api.getSync).toHaveBeenCalledOnce();
    expect(api.applySync).not.toHaveBeenCalled();
  });

  it("blocks the comparison when a conflict must be resolved outside Lore and offers to check again", async () => {
    api.getSync.mockResolvedValue(snapshot({ plan: { ...plan(), analysis: null, unsupported: ["sources/claude/a.md"] } }));
    api.prepareSync.mockResolvedValue(undefined);
    render(<SyncApp />);
    expect(await screen.findByRole("alert")).toHaveTextContent("sources/claude/a.md");
    expect(screen.getByRole("button", { name: "Compare with model" })).toBeDisabled();
    await userEvent.click(screen.getByRole("button", { name: "Check remote again" }));
    expect(api.prepareSync).toHaveBeenCalledOnce();
  });
});

describe("sync recovery", () => {
  it("reports an unshared result and retries sending without rerunning the model", async () => {
    api.getSync.mockResolvedValue(snapshot({ plan: { ...plan(), phase: "push_pending", message: "오프라인" }, summary: { phase: "push_pending", blocked: false, issues: 1, message: "오프라인", lastSyncedAt: null } }));
    api.resumeSync.mockResolvedValue(undefined);
    api.prepareSync.mockResolvedValue(undefined);
    render(<SyncApp />);
    expect(await screen.findByRole("status")).toHaveTextContent("오프라인");
    expect(screen.getByText("Not shared yet")).toBeInTheDocument();
    expect(screen.getByRole("heading", { name: "Applied on this PC, not shared yet" })).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Retry sending only" }));
    expect(api.resumeSync).toHaveBeenCalledOnce();
    await userEvent.click(screen.getByRole("button", { name: "Sync again" }));
    expect(api.prepareSync).toHaveBeenCalledOnce();
    expect(api.analyzeSync).not.toHaveBeenCalled();
  });

  it("retries a search update that failed after applying", async () => {
    api.getSync.mockResolvedValue(snapshot({ plan: { ...plan(), phase: "index_pending", message: "qmd was not found." }, summary: { phase: "index_pending", blocked: true, issues: 1, message: "qmd was not found.", lastSyncedAt: null } }));
    api.resumeSync.mockResolvedValue(undefined);
    render(<SyncApp />);
    expect(await screen.findByRole("heading", { name: "Applied on this PC, search not updated" })).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Retry search update" }));
    expect(api.resumeSync).toHaveBeenCalledOnce();
  });
});

describe("sync summary", () => {
  it("shows the last sync, syncs now, and points to settings and uncommitted notes", async () => {
    vi.useFakeTimers({ toFake: ["Date"] });
    vi.setSystemTime(new Date(NOW * 1000));
    api.getSync.mockResolvedValue(idle({ review: 2, summary: { phase: "idle", message: null, blocked: false, issues: 0, lastSyncedAt: NOW - 300 } }));
    api.syncNow.mockResolvedValue(undefined);
    render(<SyncApp />);
    expect(await screen.findByRole("heading", { name: "Your PCs are in sync" })).toBeInTheDocument();
    expect(screen.getByText("Up to date")).toBeInTheDocument();
    expect(screen.getByText("Last synced 5m ago with origin/main.")).toBeInTheDocument();
    expect(screen.getByText("On, every 5 minutes")).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Sync now" }));
    expect(api.syncNow).toHaveBeenCalledOnce();
    await userEvent.click(screen.getByRole("button", { name: "Change in Settings" }));
    expect(api.openSettings).toHaveBeenCalledOnce();
    await userEvent.click(screen.getByRole("button", { name: "Review & commit" }));
    expect(api.openReview).toHaveBeenCalledOnce();
  });

  it("explains a missing upstream and offers no sync", async () => {
    api.getSync.mockResolvedValue(idle({ tracking: null }));
    render(<SyncApp />);
    expect(await screen.findByRole("heading", { name: "Not synced yet" })).toBeInTheDocument();
    expect(screen.getByText(/has no upstream branch yet/)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Sync now" })).toBeDisabled();
    expect(screen.queryByRole("button", { name: "Review & commit" })).not.toBeInTheDocument();
  });
});
