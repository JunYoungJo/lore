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
  commitReview: vi.fn(),
  onStateChanged: vi.fn((handler: () => void) => {
    listeners.push(handler);
    return Promise.resolve(() => {});
  }),
}));

vi.mock("@/ipc/api", () => api);

import { ReviewApp } from "@/ReviewApp";

const DEFAULT_MESSAGE = "기억 기록 검토 (2026-11-05)";

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
    others: [
      { path: ".qmd/config/index.yml", kind: "modified" },
      { path: "sources/claude/s.md", kind: "modified" },
    ],
    message: DEFAULT_MESSAGE,
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
    Promise.resolve(path === "projects/demo/bg.md" ? bgDiff : [{ kind: "added", leftNo: null, left: null, rightNo: 1, right: [{ kind: "added", text: "새 문서 본문" }] }]),
  );
});

describe("review window", () => {
  it("pauses recording and groups the changed notes by project with common first", async () => {
    api.getReview.mockResolvedValue(review());
    render(<ReviewApp />);
    const list = await screen.findByRole("list", { name: "바뀐 기억 문서" });
    expect(api.beginReview).toHaveBeenCalledTimes(1);
    const groups = within(list).getAllByRole("list");
    expect(groups.map((group) => group.getAttribute("aria-label"))).toEqual(["common", "alpha", "demo"]);
    expect(within(groups[0]).getAllByRole("listitem").map((item) => item.textContent)).toEqual([expect.stringContaining("old.md")]);
    expect(within(groups[2]).getAllByRole("listitem").map((item) => item.textContent)).toEqual([expect.stringContaining("bg.md"), expect.stringContaining("new.md")]);
    expect(within(list).getByText("수정")).toBeInTheDocument();
    expect(within(list).getAllByText("새 문서")).toHaveLength(2);
    expect(within(list).getByText("삭제")).toBeInTheDocument();
    expect(within(list).getByText(/demo · 로그만 남기도록 바꿨습니다\./)).toBeInTheDocument();
    expect(within(list).queryByText("projects/demo")).not.toBeInTheDocument();
    expect(screen.getByText("함께 커밋되는 다른 파일 2개 (대화 사본, qmd 설정 등)")).toBeInTheDocument();
    expect(await screen.findByRole("region", { name: "common/old.md 변경 내용" })).toBeInTheDocument();
    expect(api.reviewDiff).toHaveBeenCalledWith("common/old.md");
  });

  it("leaves closing to the window's own title bar", async () => {
    api.getReview.mockResolvedValue(review());
    render(<ReviewApp />);
    await screen.findByRole("list", { name: "바뀐 기억 문서" });
    expect(screen.queryByRole("button", { name: "닫기" })).not.toBeInTheDocument();
  });

  it("shows the picked note side by side with the changed words marked", async () => {
    api.getReview.mockResolvedValue(review());
    render(<ReviewApp />);
    await userEvent.click(await screen.findByRole("button", { name: /bg\.md/ }));
    const diff = await screen.findByRole("region", { name: "projects/demo/bg.md 변경 내용" });
    expect(api.reviewDiff).toHaveBeenLastCalledWith("projects/demo/bg.md");
    expect(within(diff).getByText("마지막 커밋")).toBeInTheDocument();
    expect(within(diff).getByText("지금")).toBeInTheDocument();
    expect(within(diff).getByText("−1줄")).toBeInTheDocument();
    expect(within(diff).getByText("+2줄")).toBeInTheDocument();
    const changed = within(diff).getByRole("row", { name: "3줄 바뀜" });
    const [left, right] = within(changed).getAllByRole("cell");
    expect(within(left).getByText("팝업을 띄웁니다.").tagName).toBe("DEL");
    expect(within(right).getByText("팝업 없이 로그만 남깁니다.").tagName).toBe("INS");
    expect(within(left).queryByText("팝업 없이 로그만 남깁니다.")).not.toBeInTheDocument();
    const added = within(diff).getByRole("row", { name: "5줄 추가" });
    expect(within(added).getAllByRole("cell")[1]).toHaveTextContent("## 변경 이유");
  });

  it("folds long runs of unchanged lines but never folds changes", async () => {
    const rows: DiffRow[] = [...Array.from({ length: 10 }, (_, index) => same(index + 1, `같은 줄 ${index + 1}`)), ...Array.from({ length: 10 }, (_, index) => ({ kind: "added" as const, leftNo: null, left: null, rightNo: 11 + index, right: [{ kind: "added" as const, text: `넣은 줄 ${index + 1}` }] }))];
    api.getReview.mockResolvedValue(review());
    api.reviewDiff.mockResolvedValue(rows);
    render(<ReviewApp />);
    const diff = await screen.findByRole("region", { name: "common/old.md 변경 내용" });
    expect(diff).toHaveTextContent("같은 줄 2");
    expect(diff).not.toHaveTextContent("같은 줄 5");
    expect(diff).toHaveTextContent("같은 줄 9");
    expect(diff).toHaveTextContent("넣은 줄 5");
    await userEvent.click(within(diff).getByRole("button", { name: "모두 펼치기" }));
    expect(diff).toHaveTextContent("같은 줄 5");
    await userEvent.click(within(diff).getByRole("button", { name: "긴 부분 접기" }));
    expect(diff).not.toHaveTextContent("같은 줄 5");
    await userEvent.click(within(diff).getByRole("button", { name: "같은 줄 6개 펼치기" }));
    expect(diff).toHaveTextContent("같은 줄 5");
  });

  it("starts with nothing checked and commits only the checked notes", async () => {
    const rest = review().files.filter((file) => file.path !== "projects/demo/bg.md");
    api.getReview.mockResolvedValueOnce(review()).mockResolvedValueOnce(review({ files: rest }));
    api.commitReview.mockResolvedValue("abc1234");
    render(<ReviewApp />);
    await screen.findByRole("list", { name: "바뀐 기억 문서" });
    expect(screen.getAllByRole("checkbox").every((box) => !(box as HTMLInputElement).checked)).toBe(true);
    expect(screen.getByRole("button", { name: "선택한 문서 0개 커밋" })).toBeDisabled();

    await userEvent.click(checkbox("projects/demo/bg.md 선택"));
    const message = screen.getByLabelText("커밋 메시지");
    expect(message).toHaveValue(DEFAULT_MESSAGE);
    await userEvent.type(message, " 1차");
    await userEvent.click(screen.getByRole("button", { name: "선택한 문서 1개 커밋" }));
    expect(api.commitReview).toHaveBeenCalledWith(`${DEFAULT_MESSAGE} 1차`, ["projects/demo/bg.md"]);
    expect(await screen.findByText("커밋했습니다 (abc1234)")).toBeInTheDocument();

    expect(await screen.findByRole("button", { name: "선택한 문서 0개 커밋" })).toBeDisabled();
    expect(screen.queryByRole("checkbox", { name: "projects/demo/bg.md 선택" })).not.toBeInTheDocument();
    expect(checkbox("projects/demo/new.md 선택")).not.toBeChecked();
    expect(screen.getByLabelText("커밋 메시지")).toHaveValue(DEFAULT_MESSAGE);
  });

  it("checks a whole project or every note at once", async () => {
    api.getReview.mockResolvedValue(review());
    api.commitReview.mockResolvedValue("abc1234");
    render(<ReviewApp />);
    await screen.findByRole("list", { name: "바뀐 기억 문서" });

    await userEvent.click(checkbox("demo 전체 선택"));
    expect(checkbox("projects/demo/bg.md 선택")).toBeChecked();
    expect(checkbox("projects/demo/new.md 선택")).toBeChecked();
    expect(checkbox("projects/alpha/a.md 선택")).not.toBeChecked();
    expect(checkbox("demo 전체 선택")).toBeChecked();
    expect(checkbox("전체 선택")).toHaveProperty("indeterminate", true);
    expect(screen.getByRole("button", { name: "선택한 문서 2개 커밋" })).toBeEnabled();

    await userEvent.click(checkbox("projects/demo/new.md 선택"));
    expect(checkbox("demo 전체 선택")).not.toBeChecked();
    expect(checkbox("demo 전체 선택")).toHaveProperty("indeterminate", true);
    await userEvent.click(checkbox("demo 전체 선택"));
    expect(checkbox("projects/demo/new.md 선택")).toBeChecked();

    await userEvent.click(checkbox("전체 선택"));
    expect(checkbox("전체 선택")).toBeChecked();
    expect(checkbox("전체 선택")).toHaveProperty("indeterminate", false);
    await userEvent.click(screen.getByRole("button", { name: "선택한 문서 4개 커밋" }));
    expect(api.commitReview).toHaveBeenCalledWith(DEFAULT_MESSAGE, ["common/old.md", "projects/alpha/a.md", "projects/demo/bg.md", "projects/demo/new.md"]);
  });

  it("unchecks every note from the select-all box", async () => {
    api.getReview.mockResolvedValue(review());
    render(<ReviewApp />);
    await screen.findByRole("list", { name: "바뀐 기억 문서" });
    await userEvent.click(checkbox("전체 선택"));
    await userEvent.click(checkbox("전체 선택"));
    expect(screen.getAllByRole("checkbox").every((box) => !(box as HTMLInputElement).checked)).toBe(true);
    expect(screen.getByRole("button", { name: "선택한 문서 0개 커밋" })).toBeDisabled();
  });

  it("commits the other files alone when no note changed", async () => {
    api.getReview.mockResolvedValue(review({ files: [] }));
    api.commitReview.mockResolvedValue("abc1234");
    render(<ReviewApp />);
    expect(await screen.findByText("마지막 커밋 이후 바뀐 기억 문서가 없습니다.")).toBeInTheDocument();
    expect(screen.queryByRole("checkbox")).not.toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "커밋" }));
    expect(api.commitReview).toHaveBeenCalledWith(DEFAULT_MESSAGE, []);
  });

  it("has nothing to commit when nothing changed", async () => {
    api.getReview.mockResolvedValue(review({ files: [], others: [] }));
    render(<ReviewApp />);
    expect(await screen.findByText("마지막 커밋 이후 바뀐 기억 문서가 없습니다.")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "커밋" })).toBeDisabled();
  });

  it("waits for the recording in progress and starts once it finishes", async () => {
    api.getReview.mockResolvedValueOnce(review({ waiting: true, files: [], others: [], message: "" })).mockResolvedValueOnce(review());
    render(<ReviewApp />);
    expect(await screen.findByText("지금 기록 중인 세션이 끝나면 검토를 시작합니다.")).toBeInTheDocument();
    expect(screen.queryByRole("list", { name: "바뀐 기억 문서" })).not.toBeInTheDocument();
    await act(async () => {
      listeners.forEach((listener) => listener());
    });
    expect(await screen.findByRole("list", { name: "바뀐 기억 문서" })).toBeInTheDocument();
  });

  it("explains when the memory store is not a git repository", async () => {
    api.getReview.mockResolvedValue(review({ repository: false, files: [], others: [], message: "" }));
    render(<ReviewApp />);
    expect(await screen.findByText("D:/doc/knowledge는 git 저장소가 아니어서 검토할 수 없습니다.")).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /커밋/ })).not.toBeInTheDocument();
  });
});
