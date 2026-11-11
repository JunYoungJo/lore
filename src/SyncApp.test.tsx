import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { DiffRow, SyncItem, SyncItemView, SyncSnapshot } from "@/ipc/contract";

const api = vi.hoisted(() => ({
  getSync: vi.fn(), getSyncItem: vi.fn(), previewSyncItem: vi.fn(), resolveSyncItem: vi.fn(), dismissSyncItem: vi.fn(), retrySyncComparison: vi.fn(), skipSyncComparison: vi.fn(), deferSync: vi.fn(),
  openReview: vi.fn(), openSettings: vi.fn(), openNote: vi.fn(), syncNow: vi.fn(), onStateChanged: vi.fn(() => Promise.resolve(() => {})), onSyncComparisonProgress: vi.fn((_handler: (progress: unknown) => void) => Promise.resolve(() => {})),
}));
vi.mock("@/ipc/api", () => api);
import { SyncApp } from "@/SyncApp";

const NOW = 1_800_000_600;
const note = "projects/demo/policy.md";
const merged: SyncItem = { id: "m1", kind: "merged", plan: "p1", path: note, title: "Both PCs changed the same lines", detail: "Lore kept the lines from both PCs. Check the note for repeated or disagreeing lines.", createdAt: NOW - 60, issue: null };
const decision: SyncItem = { id: "d1", kind: "decision", plan: "p1", path: note, title: "오류 알림 방침", detail: "양쪽 문서에서 반대되는 방침을 기록했습니다.", createdAt: NOW - 30, issue: 0 };
const diff = (text: string): DiffRow[] => [{ kind: "added", leftNo: null, left: null, rightNo: 2, right: [{ kind: "added", text }] }];
const file = { path: note, base: "미정", local: "로그만", remote: "팝업", merged: "로그만\n팝업", rows: diff("팝업") };
const views: Record<string, SyncItemView> = {
  m1: { item: merged, files: [file], issue: null, sources: [] },
  d1: {
    item: decision,
    files: [file],
    sources: [{ id: "local:sources/claude/a.md", side: "local", path: "sources/claude/a.md", text: "사용자: 팝업은 빼고 로그만 남겨." }, { id: "remote:sources/codex/b.md", side: "remote", path: "sources/codex/b.md", text: "사용자: 오류 팝업을 표시해." }],
    issue: { title: "오류 알림 방침", explanation: "양쪽 문서에서 반대되는 방침을 기록했습니다.", evidence: [{ source: "local:sources/claude/a.md", quote: "팝업은 빼고 로그만 남겨." }, { source: "remote:sources/codex/b.md", quote: "오류 팝업을 표시해." }], options: [
      { label: "로그 전용", description: "팝업 없이 로그만 남깁니다.", edits: [{ path: note, find: "팝업", text: "" }] },
      { label: "팝업 사용", description: "오류 팝업을 표시합니다.", edits: [{ path: note, find: "로그만", text: "" }] },
    ] },
  },
};
function snapshot(overrides: Partial<SyncSnapshot> = {}): SyncSnapshot {
  return { root: "/knowledge", enabled: true, busy: false, summary: { phase: "idle", message: null, items: 0, comparing: false, lastSyncedAt: NOW - 300 }, tracking: "main ↔ origin/main", model: "Claude opus", review: null, items: [], comparison: null, comparisonError: null, indexPending: false, ...overrides };
}
const listed = (items: SyncItem[], overrides: Partial<SyncSnapshot> = {}) => snapshot({ items, summary: { phase: "idle", message: null, items: items.length, comparing: false, lastSyncedAt: NOW - 300 }, ...overrides });

beforeEach(() => {
  Object.values(api).forEach((mock) => mock.mockReset());
  api.onStateChanged.mockResolvedValue(() => {});
  api.onSyncComparisonProgress.mockResolvedValue(() => {});
  api.getSync.mockResolvedValue(snapshot());
  api.getSyncItem.mockImplementation((id: string) => Promise.resolve(views[id]));
  api.previewSyncItem.mockImplementation((_id: string, option: number, customText: string | null) => Promise.resolve({ [note]: diff(customText ?? (option === 0 ? "최종: 로그만" : "최종: 팝업")) }));
  api.resolveSyncItem.mockResolvedValue(undefined);
  api.dismissSyncItem.mockResolvedValue(undefined);
  api.retrySyncComparison.mockResolvedValue(undefined);
  api.skipSyncComparison.mockResolvedValue(undefined);
  api.deferSync.mockResolvedValue(undefined);
  api.openNote.mockResolvedValue(undefined);
  api.syncNow.mockResolvedValue(undefined);
});

afterEach(() => {
  vi.useRealTimers();
});

describe("nothing to check", () => {
  it("shows when the PCs were last in sync and syncs on request", async () => {
    vi.useFakeTimers({ toFake: ["Date"] });
    vi.setSystemTime(new Date(NOW * 1000));
    render(<SyncApp />);
    expect(await screen.findByText("Your PCs are in sync")).toBeInTheDocument();
    expect(screen.getByText("Up to date")).toBeInTheDocument();
    expect(screen.getByText(/Last synced 5m ago with origin\/main/)).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Sync now" }));
    expect(api.syncNow).toHaveBeenCalledTimes(1);
  });

  it("says recording is committed and shared automatically, and review is optional", async () => {
    api.getSync.mockResolvedValue(snapshot({ review: 3 }));
    render(<SyncApp />);
    expect(await screen.findByText("3 notes recorded since your last review. They are already shared; review them when you want.")).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Review" }));
    expect(api.openReview).toHaveBeenCalledTimes(1);
  });

  it("explains a sync that keeps failing and lets the user try again", async () => {
    api.getSync.mockResolvedValue(snapshot({ summary: { phase: "failing", message: "Notes are saved on this PC but could not be sent: authentication failed", items: 0, comparing: false, lastSyncedAt: NOW - 3600 } }));
    render(<SyncApp />);
    expect(await screen.findByText("Sync is not getting through")).toBeInTheDocument();
    expect(screen.getByText(/authentication failed/)).toBeInTheDocument();
    expect(screen.getByText("Recording continues. Lore keeps retrying in the background.")).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Try again" }));
    expect(api.syncNow).toHaveBeenCalledTimes(1);
  });

  it("stays quiet about a failure that is still being retried", async () => {
    api.getSync.mockResolvedValue(snapshot({ summary: { phase: "retrying", message: "network unreachable", items: 0, comparing: false, lastSyncedAt: NOW - 600 } }));
    render(<SyncApp />);
    expect(await screen.findByText("Your PCs are in sync")).toBeInTheDocument();
    expect(screen.getByText("Retrying")).toBeInTheDocument();
  });

  it("shows a sync error returned by a manual sync", async () => {
    api.syncNow.mockRejectedValue("No remote upstream is configured.");
    render(<SyncApp />);
    await userEvent.click(await screen.findByRole("button", { name: "Sync now" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("No remote upstream is configured.");
  });

  it("asks for an upstream instead of offering sync when there is none", async () => {
    api.getSync.mockResolvedValue(snapshot({ tracking: null, summary: { phase: "idle", message: null, items: 0, comparing: false, lastSyncedAt: null } }));
    render(<SyncApp />);
    expect(await screen.findByText(/no upstream branch yet/)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Sync now" })).toBeDisabled();
  });
});

describe("items to check", () => {
  it("lists a note where both PCs' lines were kept and lets the user mark it checked", async () => {
    api.getSync.mockResolvedValueOnce(listed([merged])).mockResolvedValue(snapshot());
    render(<SyncApp />);
    const pane = await screen.findByRole("region", { name: "policy.md" });
    expect(screen.getByText("1 to check")).toBeInTheDocument();
    expect(within(pane).getByText("Lore kept the lines from both PCs. Check the note for repeated or disagreeing lines.")).toBeInTheDocument();
    expect(within(pane).getByText("This PC before sync")).toBeInTheDocument();
    expect(screen.getByText("Nothing waits on these. Recording and sync continue.")).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Open note" }));
    expect(api.openNote).toHaveBeenCalledWith(note);
    await userEvent.click(screen.getByRole("button", { name: "Mark as checked" }));
    expect(api.dismissSyncItem).toHaveBeenCalledWith("m1");
    expect(await screen.findByText("Your PCs are in sync")).toBeInTheDocument();
  });

  it("shows where a disagreement came from and applies the chosen wording after a preview", async () => {
    api.getSync.mockResolvedValue(listed([merged, decision]));
    render(<SyncApp />);
    await userEvent.click(await screen.findByRole("button", { name: /오류 알림 방침/ }));
    const pane = await screen.findByRole("region", { name: "오류 알림 방침" });
    expect(within(pane).getByText("팝업은 빼고 로그만 남겨.")).toBeInTheDocument();
    await userEvent.click(within(pane).getAllByText("View source conversation", { selector: "summary" })[0]);
    expect(screen.getByText("사용자: 팝업은 빼고 로그만 남겨.")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Apply" })).toBeDisabled();
    await userEvent.click(screen.getByRole("radio", { name: /로그 전용/ }));
    expect(await within(pane).findByText("최종: 로그만")).toBeInTheDocument();
    expect(api.previewSyncItem).toHaveBeenLastCalledWith("d1", 0, null);
    await userEvent.click(screen.getByRole("button", { name: "Apply" }));
    expect(api.resolveSyncItem).toHaveBeenCalledWith("d1", 0, null);
  });

  it("lets the user edit the wording of a single-edit option", async () => {
    api.getSync.mockResolvedValue(listed([decision]));
    render(<SyncApp />);
    const pane = await screen.findByRole("region", { name: "오류 알림 방침" });
    await userEvent.click(screen.getByRole("radio", { name: /팝업 사용/ }));
    await userEvent.click(screen.getByRole("checkbox", { name: "Edit the selected wording" }));
    const wording = screen.getByRole("textbox", { name: "Final wording" });
    await userEvent.type(wording, "조건부");
    await waitFor(() => expect(api.previewSyncItem).toHaveBeenLastCalledWith("d1", 1, "조건부"));
    // Once in the wording box, once in the previewed note.
    await waitFor(() => expect(within(pane).getAllByText("조건부")).toHaveLength(2));
    await userEvent.click(screen.getByRole("button", { name: "Apply" }));
    expect(api.resolveSyncItem).toHaveBeenCalledWith("d1", 1, "조건부");
  });

  it("keeps a decision the user does not want and shows why an option cannot be applied", async () => {
    api.getSync.mockResolvedValue(listed([decision]));
    api.previewSyncItem.mockRejectedValue("projects/demo/policy.md changed after the comparison, so this option no longer fits.");
    render(<SyncApp />);
    await screen.findByRole("region", { name: "오류 알림 방침" });
    await userEvent.click(screen.getByRole("radio", { name: /로그 전용/ }));
    expect(await screen.findByRole("alert")).toHaveTextContent("changed after the comparison");
    expect(screen.getByRole("button", { name: "Apply" })).toBeDisabled();
    await userEvent.click(screen.getByRole("button", { name: "Keep the note as it is" }));
    expect(api.dismissSyncItem).toHaveBeenCalledWith("d1");
  });

  it("closes without deciding anything", async () => {
    api.getSync.mockResolvedValue(listed([merged]));
    render(<SyncApp />);
    await screen.findByRole("region", { name: "policy.md" });
    await userEvent.click(screen.getByRole("button", { name: "Later" }));
    expect(api.deferSync).toHaveBeenCalledTimes(1);
  });
});

describe("model comparison", () => {
  it("shows a running comparison without blocking the items already listed", async () => {
    api.getSync.mockResolvedValue(listed([merged], { summary: { phase: "idle", message: null, items: 1, comparing: true, lastSyncedAt: NOW }, comparison: { completed: 1, total: 3, stage: "comparing", reads: 4, error: null } }));
    render(<SyncApp />);
    expect(await screen.findByRole("status")).toHaveTextContent("Claude opus is comparing the notes both PCs changed · 1 of 3 groups");
    expect(screen.getByRole("button", { name: "Mark as checked" })).toBeEnabled();
  });

  it("offers to retry or skip a comparison that could not finish", async () => {
    api.getSync.mockResolvedValue(snapshot({ comparisonError: "This quotation is not in source remote:a.md" }));
    render(<SyncApp />);
    expect(await screen.findByText("The comparison could not finish. Your notes are synced either way.")).toBeInTheDocument();
    expect(screen.getByText("This quotation is not in source remote:a.md")).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Try again" }));
    expect(api.retrySyncComparison).toHaveBeenCalledTimes(1);
    await userEvent.click(screen.getByRole("button", { name: "Skip" }));
    expect(api.skipSyncComparison).toHaveBeenCalledTimes(1);
  });
});
