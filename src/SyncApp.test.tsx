import { act, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { SyncPlan, SyncSnapshot } from "@/ipc/contract";

const api = vi.hoisted(() => ({ getSync: vi.fn(), setSyncEnabled: vi.fn(), prepareSync: vi.fn(), analyzeSync: vi.fn(), previewSync: vi.fn(), applySync: vi.fn(), resumeSync: vi.fn(), deferSync: vi.fn(), openReview: vi.fn(), onStateChanged: vi.fn(() => Promise.resolve(() => {})) }));
vi.mock("@/ipc/api", () => api);
import { SyncApp } from "@/SyncApp";

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
function snapshot(overrides: Partial<SyncSnapshot> = {}): SyncSnapshot { return { root: "/knowledge", enabled: false, busy: false, summary: { phase: "review", message: null, blocked: true, issues: 1 }, plan: plan(), ...overrides }; }

beforeEach(() => {
  Object.values(api).forEach((mock) => mock.mockReset());
  api.onStateChanged.mockResolvedValue(() => {});
  api.getSync.mockResolvedValue(snapshot());
  api.previewSync.mockImplementation((_id: string, choices: { option: number; customText: string | null }[]) => Promise.resolve({ files: { [note]: choices[0]?.customText ?? (choices[0]?.option === 0 ? "최종: 로그만" : "최종: 팝업") }, token: "confirmed-preview" }));
  api.applySync.mockResolvedValue(undefined);
});

describe("model-assisted synchronization review", () => {
  it("shows provenance and requires a choice and server preview before applying", async () => {
    render(<SyncApp />);
    await screen.findByRole("region", { name: "오류 알림 방침" });
    expect(screen.getByRole("button", { name: "Apply & share" })).toBeDisabled();
    expect(api.applySync).not.toHaveBeenCalled();
    expect(screen.getByText("팝업은 빼고 로그만 남겨.")).toBeInTheDocument();
    await userEvent.click(screen.getAllByText("View source conversation", { selector: "summary" })[0]);
    expect(screen.getByText("사용자: 팝업은 빼고 로그만 남겨.")).toBeInTheDocument();
    await userEvent.click(screen.getByRole("radio", { name: /로그 전용/ }));
    const preview = await screen.findByRole("region", { name: "Final document preview" });
    expect(within(preview).getByText("최종: 로그만")).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Apply & share" }));
    expect(api.applySync).toHaveBeenCalledWith("comparison-1", [{ issue: 0, option: 0, customText: null }], "confirmed-preview");
  });

  it("lets the user edit the replacement text while preserving the target span", async () => {
    render(<SyncApp />);
    await screen.findByRole("radio", { name: /로그 전용/ });
    await userEvent.click(screen.getByRole("radio", { name: /로그 전용/ }));
    await userEvent.click(screen.getByRole("checkbox", { name: "Edit the selected wording" }));
    const input = screen.getByRole("textbox", { name: "Final wording" });
    await userEvent.clear(input); await userEvent.type(input, "운영은 로그만, 점검 중 팝업");
    await waitFor(() => expect(api.previewSync).toHaveBeenLastCalledWith("comparison-1", [{ issue: 0, option: 0, customText: "운영은 로그만, 점검 중 팝업" }]));
    await screen.findByText("운영은 로그만, 점검 중 팝업", { selector: "pre" });
    expect(api.applySync).not.toHaveBeenCalled();
  });

  it("does not silently apply after a preview validation failure", async () => {
    api.previewSync.mockRejectedValue(new Error("Evidence 링크가 빠졌습니다"));
    render(<SyncApp />);
    await userEvent.click(await screen.findByRole("radio", { name: /로그 전용/ }));
    expect(await screen.findByRole("alert")).toHaveTextContent("Evidence 링크가 빠졌습니다");
    expect(screen.getByRole("button", { name: "Apply & share" })).toBeDisabled();
  });

  it("discards a slow preview when the choice changes", async () => {
    let resolveOld: ((value: { files: Record<string, string>; token: string }) => void) | undefined;
    api.previewSync.mockImplementationOnce(() => new Promise((resolve) => { resolveOld = resolve; }));
    render(<SyncApp />);
    await userEvent.click(await screen.findByRole("radio", { name: /로그 전용/ }));
    await waitFor(() => expect(api.previewSync).toHaveBeenCalledTimes(1));
    await userEvent.click(screen.getByRole("radio", { name: /팝업 사용/ }));
    await screen.findByText("최종: 팝업");
    await act(async () => { resolveOld?.({ files: { [note]: "잘못된 오래된 미리보기" }, token: "old" }); });
    expect(screen.queryByText("잘못된 오래된 미리보기")).not.toBeInTheDocument();
  });

  it("defers the review without applying or deleting the plan", async () => {
    api.deferSync.mockResolvedValue(undefined);
    render(<SyncApp />);
    await screen.findByRole("region", { name: "오류 알림 방침" });
    await userEvent.click(screen.getByRole("button", { name: "Later" }));
    expect(api.deferSync).toHaveBeenCalledOnce(); expect(api.applySync).not.toHaveBeenCalled();
  });

  it("can analyze an unprocessed plan and refreshes the model result", async () => {
    api.getSync.mockResolvedValueOnce(snapshot({ plan: { ...plan(), analysis: null } })).mockResolvedValue(snapshot());
    api.analyzeSync.mockResolvedValue(undefined);
    render(<SyncApp />);
    await userEvent.click(await screen.findByRole("button", { name: "Compare with model" }));
    expect(api.analyzeSync).toHaveBeenCalledOnce();
    expect(await screen.findByRole("region", { name: "오류 알림 방침" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Apply & share" })).toBeDisabled();
  });

  it("reports a pending push separately and resumes without rerunning the model", async () => {
    api.getSync.mockResolvedValue(snapshot({ plan: { ...plan(), phase: "push_pending", message: "오프라인" }, summary: { phase: "push_pending", blocked: false, issues: 1, message: "오프라인" } }));
    api.resumeSync.mockResolvedValue(undefined);
    render(<SyncApp />);
    expect(await screen.findByRole("status")).toHaveTextContent("오프라인");
    await userEvent.click(screen.getByRole("button", { name: "Retry push" }));
    expect(api.resumeSync).toHaveBeenCalledOnce(); expect(api.analyzeSync).not.toHaveBeenCalled();
  });
});
