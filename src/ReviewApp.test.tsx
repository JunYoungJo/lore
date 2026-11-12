import { act, render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

import type { DiffRow, ReviewSnapshot } from "@/ipc/contract";

const listeners: (() => void)[] = [];

const api = vi.hoisted(() => ({
  beginReview: vi.fn(),
  endReview: vi.fn(),
  getReview: vi.fn(),
  reviewDiff: vi.fn(),
  markReviewed: vi.fn(),
  onStateChanged: vi.fn((handler: () => void) => {
    listeners.push(handler);
    return Promise.resolve(() => {});
  }),
}));

vi.mock("@/ipc/api", () => api);

import { ReviewApp } from "@/ReviewApp";


function review(overrides: Partial<ReviewSnapshot> = {}): ReviewSnapshot {
  return {
    repository: true,
    waiting: false,
    root: "D:/doc/knowledge",
    files: [
      { path: "common/old.md", kind: "deleted", sessions: [] },
      { path: "projects/alpha/a.md", kind: "added", sessions: [] },
      {
        path: "projects/demo/bg.md",
        kind: "modified",
        sessions: [{ at: 1_800_000_000, provider: "claude", project: "demo", summary: "로그만 남기도록 바꿨습니다.\n둘째 줄" }],
      },
      { path: "projects/demo/new.md", kind: "added", sessions: [] },
    ],
    ...overrides,
  };
}

const same = (no: number, text: string): DiffRow => ({ kind: "same", leftNo: no, left: [{ kind: "same", text }], rightNo: no, right: [{ kind: "same", text }] });

const bgDiff: DiffRow[] = [
  same(1, "## 결정"),
  {
    kind: "changed",
    leftNo: 3,
    left: [
      { kind: "same", text: "경고 " },
      { kind: "removed", text: "팝업을 띄웁니다." },
    ],
    rightNo: 3,
    right: [
      { kind: "same", text: "경고 " },
      { kind: "added", text: "팝업 없이 로그만 남깁니다." },
    ],
  },
  { kind: "added", leftNo: null, left: null, rightNo: 5, right: [{ kind: "added", text: "## 변경 이유" }] },
];

const checkbox = (name: string) => screen.getByRole("checkbox", { name });

beforeEach(() => {
  listeners.length = 0;
  Object.values(api).forEach((mock) => mock.mockClear());
  api.beginReview.mockResolvedValue(undefined);
  api.endReview.mockResolvedValue(undefined);
  api.reviewDiff.mockImplementation((path: string) =>
    Promise.resolve(path === "projects/demo/bg.md" ? bgDiff : [{ kind: "added", leftNo: null, left: null, rightNo: 1, right: [{ kind: "added", text: "New note 본문" }] }]),
  );
});

describe("review window", () => {
  it("pauses recording and groups the changed notes by project with common first", async () => {
    api.getReview.mockResolvedValue(review());
    render(<ReviewApp />);
    const list = await screen.findByRole("list", { name: "Changed memory notes" });
    expect(api.beginReview).toHaveBeenCalledTimes(1);
    const groups = within(list).getAllByRole("list");
    expect(groups.map((group) => group.getAttribute("aria-label"))).toEqual(["common", "alpha", "demo"]);
    expect(within(groups[0]).getAllByRole("listitem").map((item) => item.textContent)).toEqual([expect.stringContaining("old.md")]);
    expect(within(groups[2]).getAllByRole("listitem").map((item) => item.textContent)).toEqual([expect.stringContaining("bg.md"), expect.stringContaining("new.md")]);
    expect(within(list).getByText("Modified")).toBeInTheDocument();
    expect(within(list).getAllByText("New note")).toHaveLength(2);
    expect(within(list).getByText("Deleted")).toBeInTheDocument();
    expect(within(list).getByText(/demo · 로그만 남기도록 바꿨습니다\./)).toBeInTheDocument();
    expect(within(list).queryByText("projects/demo")).not.toBeInTheDocument();
    expect(await screen.findByRole("region", { name: "Changes in common/old.md" })).toBeInTheDocument();
    expect(api.reviewDiff).toHaveBeenCalledWith("common/old.md");
  });

  it("leaves closing to the window's own title bar", async () => {
    api.getReview.mockResolvedValue(review());
    render(<ReviewApp />);
    await screen.findByRole("list", { name: "Changed memory notes" });
    expect(screen.queryByRole("button", { name: "닫기" })).not.toBeInTheDocument();
  });

  it("shows the picked note side by side with the changed words marked", async () => {
    api.getReview.mockResolvedValue(review());
    render(<ReviewApp />);
    await userEvent.click(await screen.findByRole("button", { name: /bg\.md/ }));
    const diff = await screen.findByRole("region", { name: "Changes in projects/demo/bg.md" });
    expect(api.reviewDiff).toHaveBeenLastCalledWith("projects/demo/bg.md");
    expect(within(diff).getByText("Last review")).toBeInTheDocument();
    expect(within(diff).getByText("Current")).toBeInTheDocument();
    expect(within(diff).getByText("−1 line")).toBeInTheDocument();
    expect(within(diff).getByText("+2 lines")).toBeInTheDocument();
    // The choice of lines is offered even when this note has nothing to fold, so the bar is the same for every note.
    expect(within(diff).getByRole("button", { name: "Changes only" })).toHaveAttribute("aria-pressed", "true");
    expect(within(diff).getByRole("button", { name: "Whole note" })).toHaveAttribute("aria-pressed", "false");
    const changed = within(diff).getByRole("row", { name: "Line 3: Changed" });
    const [left, right] = within(changed).getAllByRole("cell");
    expect(within(left).getByText("팝업을 띄웁니다.").tagName).toBe("DEL");
    expect(within(right).getByText("팝업 없이 로그만 남깁니다.").tagName).toBe("INS");
    expect(within(left).queryByText("팝업 없이 로그만 남깁니다.")).not.toBeInTheDocument();
    const added = within(diff).getByRole("row", { name: "Line 5: Added" });
    expect(within(added).getAllByRole("cell")[1]).toHaveTextContent("## 변경 이유");
  });

  it("folds long runs of unchanged lines but never folds changes", async () => {
    const rows: DiffRow[] = [...Array.from({ length: 10 }, (_, index) => same(index + 1, `같은 줄 ${index + 1}`)), ...Array.from({ length: 10 }, (_, index) => ({ kind: "added" as const, leftNo: null, left: null, rightNo: 11 + index, right: [{ kind: "added" as const, text: `넣은 줄 ${index + 1}` }] }))];
    api.getReview.mockResolvedValue(review());
    api.reviewDiff.mockResolvedValue(rows);
    render(<ReviewApp />);
    const diff = await screen.findByRole("region", { name: "Changes in common/old.md" });
    expect(diff).toHaveTextContent("같은 줄 2");
    expect(diff).not.toHaveTextContent("같은 줄 5");
    expect(diff).toHaveTextContent("같은 줄 9");
    expect(diff).toHaveTextContent("넣은 줄 5");
    await userEvent.click(within(diff).getByRole("button", { name: "Whole note" }));
    expect(diff).toHaveTextContent("같은 줄 5");
    expect(within(diff).getByRole("button", { name: "Whole note" })).toHaveAttribute("aria-pressed", "true");
    await userEvent.click(within(diff).getByRole("button", { name: "Changes only" }));
    expect(diff).not.toHaveTextContent("같은 줄 5");
    await userEvent.click(within(diff).getByRole("button", { name: "Expand 6 unchanged lines" }));
    expect(diff).toHaveTextContent("같은 줄 5");
  });

  it("starts with nothing checked and marks only the checked notes as reviewed", async () => {
    const rest = review().files.filter((file) => file.path !== "projects/demo/bg.md");
    api.getReview.mockResolvedValueOnce(review()).mockResolvedValueOnce(review({ files: rest }));
    api.markReviewed.mockResolvedValue(1);
    render(<ReviewApp />);
    await screen.findByRole("list", { name: "Changed memory notes" });
    expect(screen.getAllByRole("checkbox").every((box) => !(box as HTMLInputElement).checked)).toBe(true);
    expect(screen.getByRole("button", { name: "Mark reviewed (0)" })).toBeDisabled();
    expect(screen.getByText("These notes are already saved and shared. Marking them only clears them from this list.")).toBeInTheDocument();
    expect(screen.queryByLabelText("Commit message")).not.toBeInTheDocument();

    await userEvent.click(checkbox("Select projects/demo/bg.md"));
    await userEvent.click(screen.getByRole("button", { name: "Mark reviewed (1)" }));
    expect(api.markReviewed).toHaveBeenCalledWith(["projects/demo/bg.md"]);
    expect(await screen.findByText("Marked 1 note as reviewed")).toBeInTheDocument();

    expect(await screen.findByRole("button", { name: "Mark reviewed (0)" })).toBeDisabled();
    expect(screen.queryByRole("checkbox", { name: "Select projects/demo/bg.md" })).not.toBeInTheDocument();
    expect(checkbox("Select projects/demo/new.md")).not.toBeChecked();
  });

  it("checks a whole project or every note at once", async () => {
    api.getReview.mockResolvedValue(review());
    api.markReviewed.mockResolvedValue(4);
    render(<ReviewApp />);
    await screen.findByRole("list", { name: "Changed memory notes" });

    await userEvent.click(checkbox("Select all in demo"));
    expect(checkbox("Select projects/demo/bg.md")).toBeChecked();
    expect(checkbox("Select projects/demo/new.md")).toBeChecked();
    expect(checkbox("Select projects/alpha/a.md")).not.toBeChecked();
    expect(checkbox("Select all in demo")).toBeChecked();
    expect(checkbox("Select all")).toHaveProperty("indeterminate", true);
    expect(screen.getByRole("button", { name: "Mark reviewed (2)" })).toBeEnabled();

    await userEvent.click(checkbox("Select projects/demo/new.md"));
    expect(checkbox("Select all in demo")).not.toBeChecked();
    expect(checkbox("Select all in demo")).toHaveProperty("indeterminate", true);
    await userEvent.click(checkbox("Select all in demo"));
    expect(checkbox("Select projects/demo/new.md")).toBeChecked();

    await userEvent.click(checkbox("Select all"));
    expect(checkbox("Select all")).toBeChecked();
    expect(checkbox("Select all")).toHaveProperty("indeterminate", false);
    await userEvent.click(screen.getByRole("button", { name: "Mark reviewed (4)" }));
    expect(api.markReviewed).toHaveBeenCalledWith(["common/old.md", "projects/alpha/a.md", "projects/demo/bg.md", "projects/demo/new.md"]);
  });

  it("unchecks every note from the select-all box", async () => {
    api.getReview.mockResolvedValue(review());
    render(<ReviewApp />);
    await screen.findByRole("list", { name: "Changed memory notes" });
    await userEvent.click(checkbox("Select all"));
    await userEvent.click(checkbox("Select all"));
    expect(screen.getAllByRole("checkbox").every((box) => !(box as HTMLInputElement).checked)).toBe(true);
    expect(screen.getByRole("button", { name: "Mark reviewed (0)" })).toBeDisabled();
  });

  it("has nothing to mark when no note changed since the last review", async () => {
    api.getReview.mockResolvedValue(review({ files: [] }));
    render(<ReviewApp />);
    expect(await screen.findByText("No memory notes have changed since your last review.")).toBeInTheDocument();
    expect(screen.queryByRole("checkbox")).not.toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /Mark reviewed/ })).not.toBeInTheDocument();
  });

  it("waits for the recording in progress and starts once it finishes", async () => {
    api.getReview.mockResolvedValueOnce(review({ waiting: true, files: [] })).mockResolvedValueOnce(review());
    render(<ReviewApp />);
    expect(await screen.findByText("Review will begin after the current recording finishes.")).toBeInTheDocument();
    expect(screen.queryByRole("list", { name: "Changed memory notes" })).not.toBeInTheDocument();
    await act(async () => {
      listeners.forEach((listener) => listener());
    });
    expect(await screen.findByRole("list", { name: "Changed memory notes" })).toBeInTheDocument();
  });

  it("explains when the memory store is not a git repository", async () => {
    api.getReview.mockResolvedValue(review({ repository: false, files: [] }));
    render(<ReviewApp />);
    expect(await screen.findByText("D:/doc/knowledge is not a Git repository.")).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /Commit/ })).not.toBeInTheDocument();
  });
});
