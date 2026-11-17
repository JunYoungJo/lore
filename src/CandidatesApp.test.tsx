import { act, render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

import type { ApplyBatch, Candidate, CandidatesSnapshot } from "@/ipc/contract";

const listeners: (() => void)[] = [];

const api = vi.hoisted(() => ({
  getCandidates: vi.fn(),
  editCandidate: vi.fn(),
  approveCandidates: vi.fn(),
  discardCandidates: vi.fn(),
  retryApply: vi.fn(),
  releaseApply: vi.fn(),
  openNote: vi.fn(),
  onStateChanged: vi.fn((handler: () => void) => {
    listeners.push(handler);
    return Promise.resolve(() => {});
  }),
}));

vi.mock("@/ipc/api", () => api);

import { CandidatesApp } from "@/CandidatesApp";

const NOW = Math.floor(Date.now() / 1000);
const DAY = 86_400;

function candidate(id: number, overrides: Partial<Candidate> = {}): Candidate {
  const createdAt = overrides.createdAt ?? NOW - 2 * DAY - 60;
  return {
    waitingSince: createdAt,
    id,
    jobId: "job-lore",
    sessionId: "session-lore",
    provider: "claude",
    project: "lore",
    language: "ko",
    kind: "decision",
    text: `후보 ${id}`,
    source: "found",
    scope: "project",
    evidence: [{ ref: "t3", quote: "큐는 로컬. 싱크는 문서만.", speaker: "user", at: "2026-11-17 10:42:05" }],
    relation: { type: "new", note: "", title: "", quotes: [] },
    state: "waiting",
    edited: false,
    createdAt,
    applyId: null,
    ...overrides,
  };
}

const LOCAL = candidate(1, { text: "후보 큐는 이 PC에만 두고, 싱크는 노트만 한다.", source: "marked" });
const DIFFERS = candidate(2, {
  kind: "correction",
  text: "세션이 끝나면 노트를 바로 쓰지 않고 후보를 목록으로 보여 준다.",
  relation: {
    type: "differs",
    note: "projects/lore/recording-flow.md",
    title: "기록 흐름",
    quotes: ["세션이 끝나면 모델이 노트를 바로 고쳐 쓴다.", "변경은 리뷰 창의 diff로 확인한다."],
  },
});
const LESSON = candidate(3, {
  jobId: "job-viewer",
  sessionId: "session-viewer",
  provider: "codex",
  project: "viewer",
  kind: "lesson",
  text: "Windows에서는 동기 명령 안에서 창을 만들면 멈춘다.",
  evidence: [{ ref: "t9", quote: "동기 명령에서 창을 만들자 교착이 일어났습니다.", speaker: "agent", at: "" }],
  createdAt: NOW - 3600,
});
const LEFT_OUT = candidate(9, {
  text: "프롬프트와 고정 문구는 영어로 쓴다.",
  state: "duplicate",
  relation: { type: "same", note: "projects/lore/language.md", title: "언어", quotes: ["프롬프트는 영어로 쓴다."] },
});

function batch(id: number, overrides: Partial<ApplyBatch> = {}): ApplyBatch {
  return { id, project: "lore", status: "done", auto: false, attempts: 0, nextAttemptAt: 0, lastError: null, result: null, candidates: 2, updatedAt: NOW - 120, ...overrides };
}

const WRITTEN = batch(4, {
  result: {
    summary: "기록 흐름 노트를 후보 승인 방식으로 고쳤습니다.",
    notes: [
      { path: "projects/lore/recording-flow.md", action: "edited", items: [DIFFERS.text] },
      { path: "projects/lore/candidate-queue.md", action: "created", items: [LOCAL.text] },
      { path: "projects/lore/diff-review.md", action: "removed", items: [DIFFERS.text] },
    ],
  },
});

function snapshot(overrides: Partial<CandidatesSnapshot> = {}): CandidatesSnapshot {
  return { waiting: [LOCAL, DIFFERS, LESSON], leftOut: [LEFT_OUT], applying: [], failed: [], applied: [], autoApprove: false, autoApproveDays: 3, autoApproveSince: 0, now: NOW, ...overrides };
}

/** Serve `first`, then whatever `next` is set to, the way the window reloads after each action. */
function serve(first: CandidatesSnapshot) {
  const served = { next: first };
  api.getCandidates.mockImplementation(async () => served.next);
  return served;
}

async function open(first: CandidatesSnapshot = snapshot()) {
  const served = serve(first);
  render(<CandidatesApp />);
  await screen.findByRole("contentinfo");
  return served;
}

const stateChanged = () => act(async () => listeners.forEach((listener) => listener()));

const list = () => screen.getByRole("list", { name: "Candidates by session" });
const detail = () => screen.getByRole("region", { name: "Candidate" });

beforeEach(() => {
  vi.clearAllMocks();
  listeners.length = 0;
  api.editCandidate.mockResolvedValue(true);
  api.approveCandidates.mockResolvedValue(1);
  api.discardCandidates.mockResolvedValue(1);
  api.retryApply.mockResolvedValue(true);
  api.releaseApply.mockResolvedValue(true);
});

describe("candidates window", () => {
  it("lists the waiting candidates by session, all ticked, and shows the first one", async () => {
    await open();
    expect(screen.getByText("3 candidates waiting")).toBeInTheDocument();
    const groups = within(list()).getAllByRole("list");
    const found = new Intl.DateTimeFormat("en-US", { month: "short", day: "numeric" }).format(new Date(LOCAL.createdAt * 1000));
    expect(groups.map((group) => group.getAttribute("aria-label"))).toEqual([`lore · Claude · ${found}`, "viewer · Codex · 1h ago"]);
    expect(within(groups[0]).getAllByRole("checkbox").map((box) => (box as HTMLInputElement).checked)).toEqual([true, true]);
    expect(screen.getByRole("checkbox", { name: "Select all" })).toBeChecked();
    expect(screen.getByText("3 of 3 selected")).toBeInTheDocument();
    expect(screen.getByLabelText("Sentence to remember")).toHaveValue(LOCAL.text);
    expect(detail()).toHaveTextContent("marked by the session while it worked");
    expect(detail()).toHaveTextContent("You · 2026-11-17 10:42");
    expect(detail()).toHaveTextContent("큐는 로컬. 싱크는 문서만.");
    expect(detail()).toHaveTextContent("No existing note was found on this.");
  });

  it("says in each row what kind it is, where it came from and how it stands to the notes", async () => {
    await open();
    const rows = within(list()).getAllByRole("listitem").filter((row) => row.classList.contains("review-item"));
    expect(rows[0]).toHaveTextContent("Decision");
    expect(rows[0]).toHaveTextContent("Marked in the session");
    expect(rows[0]).toHaveTextContent("Waiting 2 days");
    expect(rows[1]).toHaveTextContent("Correction");
    expect(rows[1]).toHaveTextContent("Found afterwards");
    expect(rows[1]).toHaveTextContent("Differs from a note");
    expect(rows[2]).toHaveTextContent("Lesson");
    expect(rows[2]).toHaveTextContent("Found today");
  });

  it("shows the note a candidate differs from, with the passages as they are written, and opens it", async () => {
    await open();
    await userEvent.click(screen.getByRole("button", { name: new RegExp(DIFFERS.text) }));
    expect(screen.getByLabelText("Sentence to remember")).toHaveValue(DIFFERS.text);
    expect(detail()).toHaveTextContent("기록 흐름");
    expect(detail()).toHaveTextContent("projects/lore/recording-flow.md");
    expect(detail()).toHaveTextContent("2 passages of this note that the candidate changes, as written:");
    expect(detail()).toHaveTextContent("세션이 끝나면 모델이 노트를 바로 고쳐 쓴다.");
    expect(detail()).toHaveTextContent("변경은 리뷰 창의 diff로 확인한다.");
    // Where it goes is the note it differs from, so there is no scope to choose.
    expect(within(detail()).queryByRole("group", { name: "Applies to" })).not.toBeInTheDocument();
    await userEvent.click(within(detail()).getByRole("button", { name: "Open projects/lore/recording-flow.md" }));
    expect(api.openNote).toHaveBeenCalledWith("projects/lore/recording-flow.md");
  });

  it("says when the evidence is the agent's words and not the user's", async () => {
    await open();
    await userEvent.click(screen.getByRole("button", { name: new RegExp(LESSON.text) }));
    expect(detail()).toHaveTextContent("Agent");
    expect(detail()).toHaveTextContent("These are the agent’s words, not yours.");
  });

  it("saves a reworded sentence when the field is left, and nothing when it did not change", async () => {
    const served = await open();
    const field = screen.getByLabelText("Sentence to remember");
    await userEvent.click(field);
    await userEvent.tab();
    expect(api.editCandidate).not.toHaveBeenCalled();
    served.next = snapshot({ waiting: [{ ...LOCAL, text: "후보는 이 PC에만 둔다.", edited: true }, DIFFERS, LESSON] });
    await userEvent.clear(field);
    await userEvent.type(field, "후보는 이 PC에만 둔다.");
    await userEvent.tab();
    expect(api.editCandidate).toHaveBeenCalledWith(1, "후보는 이 PC에만 둔다.", null);
    expect(await within(list()).findByText("후보는 이 PC에만 둔다.")).toBeInTheDocument();
    expect(within(list()).getAllByRole("listitem").find((row) => row.classList.contains("review-item"))).toHaveTextContent("Marked in the session · Edited");
  });

  it("puts back the sentence when it is emptied", async () => {
    await open();
    const field = screen.getByLabelText("Sentence to remember");
    await userEvent.clear(field);
    await userEvent.tab();
    expect(api.editCandidate).not.toHaveBeenCalled();
    expect(field).toHaveValue(LOCAL.text);
  });

  it("changes whether a candidate applies to its project or to all projects", async () => {
    await open();
    const scope = within(detail()).getByRole("group", { name: "Applies to" });
    expect(within(scope).getByRole("button", { name: "This project (lore)" })).toHaveAttribute("aria-pressed", "true");
    expect(detail()).toHaveTextContent("in the notes of lore");
    await userEvent.click(within(scope).getByRole("button", { name: "All projects" }));
    expect(api.editCandidate).toHaveBeenCalledWith(1, null, "common");
  });

  it("approves only the ticked candidates and then shows what was written", async () => {
    const served = await open();
    await userEvent.click(screen.getByRole("checkbox", { name: `Approve: ${LESSON.text}` }));
    expect(screen.getByText("2 of 3 selected")).toBeInTheDocument();
    expect(screen.getByRole("checkbox", { name: "Select all" })).not.toBeChecked();
    served.next = snapshot({ waiting: [LESSON], leftOut: [], applying: [batch(4, { status: "applying" })] });
    await userEvent.click(screen.getByRole("button", { name: "Approve 2 selected" }));
    expect(api.approveCandidates).toHaveBeenCalledWith([1, 2]);
    const written = await screen.findByRole("region", { name: "Written into the notes" });
    expect(written).toHaveTextContent("lore: writing 2 approved candidates into the notes…");
    // The one left unticked still waits, and stays unticked.
    expect(screen.getByRole("checkbox", { name: `Approve: ${LESSON.text}` })).not.toBeChecked();
    expect(screen.getByRole("button", { name: "Approve 0 selected" })).toBeDisabled();

    served.next = snapshot({ waiting: [LESSON], leftOut: [], applied: [WRITTEN] });
    await stateChanged();
    const result = await screen.findByRole("region", { name: "Written for lore" });
    expect(result).toHaveTextContent("lore · 2 candidates into 3 notes");
    expect(result).toHaveTextContent("기록 흐름 노트를 후보 승인 방식으로 고쳤습니다.");
    expect(result).toHaveTextContent("Edited");
    expect(result).toHaveTextContent("New note");
    expect(result).toHaveTextContent("Removed");
    expect(result).toHaveTextContent(LOCAL.text);
    // A removed note has nothing to open.
    expect(within(result).queryByRole("button", { name: "Open projects/lore/diff-review.md" })).not.toBeInTheDocument();
    await userEvent.click(within(result).getByRole("button", { name: "Open projects/lore/candidate-queue.md" }));
    expect(api.openNote).toHaveBeenCalledWith("projects/lore/candidate-queue.md");
  });

  it("leaves a candidate that arrives while the window is open unticked", async () => {
    const served = await open(snapshot({ waiting: [LOCAL, DIFFERS] }));
    expect(screen.getByRole("button", { name: "Approve 2 selected" })).toBeEnabled();
    served.next = snapshot();
    await stateChanged();
    expect(await screen.findByRole("checkbox", { name: `Approve: ${LESSON.text}` })).not.toBeChecked();
    expect(screen.getByRole("button", { name: "Approve 2 selected" })).toBeEnabled();
    await userEvent.click(screen.getByRole("checkbox", { name: "Select all" }));
    expect(screen.getByRole("button", { name: "Approve 3 selected" })).toBeEnabled();
  });

  it("asks before discarding the selected candidates", async () => {
    await open();
    await userEvent.click(screen.getByRole("button", { name: "Discard 3 selected" }));
    const confirm = screen.getByRole("alertdialog", { name: "Discard candidates confirmation" });
    expect(confirm).toHaveTextContent("Discard 3 selected candidates?");
    await userEvent.click(within(confirm).getByRole("button", { name: "Cancel" }));
    expect(api.discardCandidates).not.toHaveBeenCalled();
    await userEvent.click(screen.getByRole("button", { name: "Discard 3 selected" }));
    await userEvent.click(within(screen.getByRole("alertdialog")).getByRole("button", { name: "Discard 3" }));
    expect(api.discardCandidates).toHaveBeenCalledWith([1, 2, 3]);
  });

  it("discards the candidate in view and moves on to the next", async () => {
    const served = await open();
    served.next = snapshot({ waiting: [DIFFERS, LESSON] });
    await userEvent.click(within(detail()).getByRole("button", { name: "Discard this candidate" }));
    expect(api.discardCandidates).toHaveBeenCalledWith([1]);
    expect(await screen.findByDisplayValue(DIFFERS.text)).toBeInTheDocument();
  });

  it("counts what was left out because a note already says it, and lists it on request", async () => {
    await open();
    const group = within(list()).getAllByRole("listitem").find((item) => item.classList.contains("review-group"))!;
    expect(group).toHaveTextContent("1 statement already in your notes was left out.");
    expect(group).not.toHaveTextContent(LEFT_OUT.text);
    await userEvent.click(within(group).getByRole("button", { name: "Show" }));
    expect(group).toHaveTextContent(LEFT_OUT.text);
    expect(group).toHaveTextContent("projects/lore/language.md");
    await userEvent.click(within(group).getByRole("button", { name: "Hide" }));
    expect(group).not.toHaveTextContent(LEFT_OUT.text);
  });

  const rowsOf = () => within(list()).getAllByRole("listitem").filter((row) => row.classList.contains("review-item"));

  it("says when a candidate is approved without the user", async () => {
    // Automatic approval has been on since before these candidates were found.
    await open(snapshot({ autoApprove: true, autoApproveDays: 3, autoApproveSince: NOW - 30 * DAY }));
    expect(rowsOf()[0]).toHaveTextContent("Auto-approves in 1 day");
    expect(rowsOf()[2]).toHaveTextContent("Auto-approves in 3 days");
    expect(screen.getByRole("contentinfo")).toHaveTextContent("are approved automatically after 3 days");
  });

  it("counts the days of automatic approval from when it was turned on or the candidate came back to the list", async () => {
    // Turned on a moment ago: what has waited two days is not approved at once.
    await open(snapshot({ autoApprove: true, autoApproveDays: 3, autoApproveSince: NOW }));
    expect(rowsOf().map((row) => /Auto-approves in \d+ days?/.exec(row.textContent ?? "")?.[0])).toEqual(["Auto-approves in 3 days", "Auto-approves in 3 days", "Auto-approves in 3 days"]);
  });

  it("gives a candidate that was put back in the list its full waiting time again", async () => {
    const back = candidate(4, { text: "되돌아온 후보", createdAt: NOW - 20 * DAY, waitingSince: NOW - DAY });
    await open(snapshot({ waiting: [back], leftOut: [], autoApprove: true, autoApproveDays: 3, autoApproveSince: NOW - 30 * DAY }));
    expect(rowsOf()[0]).toHaveTextContent("Auto-approves in 2 days");
  });

  it("says that a written batch still waits for the search index", async () => {
    const late = batch(8, { status: "index_pending", attempts: 2, lastError: "The notes are written, but the search index could not be refreshed: qmd was not found.", result: WRITTEN.result });
    await open(snapshot({ waiting: [], leftOut: [], applied: [late] }));
    const result = screen.getByRole("region", { name: "Written for lore" });
    expect(result).toHaveTextContent("The notes are written. The search index is not refreshed yet; Lore keeps trying.");
    expect(result).toHaveTextContent("lore · 2 candidates into 3 notes");
  });

  it("offers to try a failed batch again or to put its candidates back in the list", async () => {
    const failed = batch(6, { status: "failed", attempts: 3, lastError: "A removal must rest on a candidate that ends that note." });
    await open(snapshot({ waiting: [], leftOut: [], failed: [failed] }));
    const written = screen.getByRole("region", { name: "Written into the notes" });
    expect(written).toHaveTextContent("lore: 2 approved candidates could not be written. The notes are unchanged.");
    expect(written).toHaveTextContent("A removal must rest on a candidate that ends that note.");
    await userEvent.click(within(written).getByRole("button", { name: "Try again" }));
    expect(api.retryApply).toHaveBeenCalledWith(6);
    await userEvent.click(within(written).getByRole("button", { name: "Put back in the list" }));
    expect(api.releaseApply).toHaveBeenCalledWith(6);
  });

  it("says when a batch waits to be tried again", async () => {
    const retrying = batch(7, { status: "pending", attempts: 1, nextAttemptAt: NOW + 300, lastError: "The model timed out." });
    await open(snapshot({ waiting: [], leftOut: [], applying: [retrying] }));
    expect(screen.getByRole("region", { name: "Written into the notes" })).toHaveTextContent("lore: trying again in about 5 min. The model timed out.");
  });

  it("says that nothing waits and that nothing was approved yet", async () => {
    await open(snapshot({ waiting: [], leftOut: [] }));
    expect(screen.getByText("0 candidates waiting")).toBeInTheDocument();
    expect(screen.getByText("No candidate is waiting. New ones appear when a session ends.")).toBeInTheDocument();
    expect(screen.getByRole("region", { name: "Written into the notes" })).toHaveTextContent("Nothing has been approved yet.");
    expect(screen.getByRole("button", { name: "Approve 0 selected" })).toBeDisabled();
  });

  it("reports an action that failed and keeps the list", async () => {
    await open();
    api.approveCandidates.mockRejectedValue("The database is locked.");
    await userEvent.click(screen.getByRole("button", { name: "Approve 3 selected" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("The database is locked.");
    expect(screen.getByLabelText("Sentence to remember")).toHaveValue(LOCAL.text);
  });
});
