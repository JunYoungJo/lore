import { act, render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

import type { ApplyBatch, Candidate, CandidatesSnapshot } from "@/ipc/contract";

const listeners: (() => void)[] = [];
const focusListeners: (() => void)[] = [];

const api = vi.hoisted(() => ({
  getCandidates: vi.fn(),
  editCandidate: vi.fn(),
  approveCandidates: vi.fn(),
  discardCandidates: vi.fn(),
  retryApply: vi.fn(),
  releaseApply: vi.fn(),
  clearWritten: vi.fn(),
  takeCandidatesFocus: vi.fn(),
  onCandidatesFocus: vi.fn((handler: () => void) => {
    focusListeners.push(handler);
    return Promise.resolve(() => {});
  }),
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
const ENDS = candidate(5, {
  text: "검증할 때 앱을 끄고 다시 띄우는 일은 사용자가 한다.",
  relation: { type: "obsolete", note: "projects/lore/restart-for-checks.md", title: "검증 때 재시작", quotes: ["검증할 때는 에이전트가 앱을 끄고 다시 띄운다."] },
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
const footer = () => screen.getByRole("contentinfo");
const rowsOf = () => within(list()).getAllByRole("button").filter((button) => button.classList.contains("cand-row"));
const row = (item: Candidate) => within(list()).getByRole("button", { name: new RegExp(item.text) });
const stateOf = (item: Candidate) => within(row(item)).getByRole("img").getAttribute("aria-label");
const field = () => screen.getByRole("textbox", { name: "Sentence to remember" });

beforeEach(() => {
  vi.clearAllMocks();
  listeners.length = 0;
  focusListeners.length = 0;
  api.takeCandidatesFocus.mockResolvedValue(null);
  api.editCandidate.mockResolvedValue(true);
  api.approveCandidates.mockResolvedValue(1);
  api.discardCandidates.mockResolvedValue(1);
  api.retryApply.mockResolvedValue(true);
  api.releaseApply.mockResolvedValue(true);
  api.clearWritten.mockResolvedValue(1);
});

describe("candidates window", () => {
  it("lists the waiting candidates by session, none decided yet, and shows the first one", async () => {
    await open();
    expect(screen.getByText("3 candidates waiting · 0 decided")).toBeInTheDocument();
    const groups = within(list()).getAllByRole("list");
    const found = new Intl.DateTimeFormat("en-US", { month: "short", day: "numeric" }).format(new Date(LOCAL.createdAt * 1000));
    expect(groups.map((group) => group.getAttribute("aria-label"))).toEqual([`lore · Claude · ${found}`, "viewer · Codex · 1h ago"]);
    expect([LOCAL, DIFFERS, LESSON].map(stateOf)).toEqual(["Not decided", "Not decided", "Not decided"]);
    expect(row(LOCAL)).toHaveAttribute("aria-current", "true");
    expect(detail()).toHaveTextContent(LOCAL.text);
    expect(detail()).toHaveTextContent("marked by the session while it worked");
    expect(detail()).toHaveTextContent("You · 2026-11-17 10:42");
    expect(detail()).toHaveTextContent("큐는 로컬. 싱크는 문서만.");
    expect(detail()).toHaveTextContent("Adds this to your notes");
    expect(detail()).toHaveTextContent("1 of 3");
  });

  it("says in each row what kind it is and what approving it does to the notes", async () => {
    await open(snapshot({ waiting: [LOCAL, DIFFERS, ENDS, { ...LESSON, scope: "common" }] }));
    const rows = rowsOf();
    expect(rows[0]).toHaveTextContent("Decision");
    expect(rows[0]).not.toHaveTextContent("·");
    expect(rows[1]).toHaveTextContent("Correction");
    expect(rows[1]).toHaveTextContent("Edits a note");
    expect(rows[2]).toHaveTextContent("Deletes a note");
    expect(rows[3]).toHaveTextContent("Lesson");
    expect(rows[3]).toHaveTextContent("All projects");
    // How long a candidate waited matters only when it is approved without the user.
    expect(rows[0]).not.toHaveTextContent(/Waiting|Found today/);
  });

  it("shows the note a candidate edits, with the passages as they are written, and opens it", async () => {
    await open();
    await userEvent.click(row(DIFFERS));
    expect(detail()).toHaveTextContent(DIFFERS.text);
    expect(detail()).toHaveTextContent("Edits this note");
    expect(detail()).toHaveTextContent("기록 흐름");
    expect(detail()).toHaveTextContent("projects/lore/recording-flow.md");
    expect(detail()).toHaveTextContent("These 2 passages change:");
    expect(detail()).toHaveTextContent("세션이 끝나면 모델이 노트를 바로 고쳐 쓴다.");
    expect(detail()).toHaveTextContent("변경은 리뷰 창의 diff로 확인한다.");
    // The note it edits is where it goes, so there is no scope to choose.
    expect(within(detail()).queryByRole("radiogroup", { name: "Applies to" })).not.toBeInTheDocument();
    await userEvent.click(within(detail()).getByRole("button", { name: "Open projects/lore/recording-flow.md" }));
    expect(api.openNote).toHaveBeenCalledWith("projects/lore/recording-flow.md");
  });

  it("says that approving a candidate that ends a note deletes the note", async () => {
    await open(snapshot({ waiting: [ENDS] }));
    expect(detail()).toHaveTextContent("Deletes this note");
    expect(detail()).toHaveTextContent("검증 때 재시작");
    expect(detail()).toHaveTextContent("The note says:");
    expect(detail()).toHaveTextContent("검증할 때는 에이전트가 앱을 끄고 다시 띄운다.");
    expect(detail()).toHaveTextContent("If part of the note still holds, that part is kept and only the rest is removed.");
  });

  it("says when the evidence is the agent's words and not the user's", async () => {
    await open();
    await userEvent.click(row(LESSON));
    expect(detail()).toHaveTextContent("Agent");
    expect(detail()).toHaveTextContent("These are the agent’s words, not yours.");
  });

  it("rewords the sentence: Enter saves, Shift+Enter keeps a new line, Esc and an unchanged sentence save nothing", async () => {
    const served = await open();
    await userEvent.click(within(detail()).getByRole("button", { name: /^Edit/ }));
    expect(field()).toHaveValue(LOCAL.text);
    await userEvent.keyboard("{Escape}");
    expect(screen.queryByRole("textbox", { name: "Sentence to remember" })).not.toBeInTheDocument();
    await userEvent.keyboard("e");
    await userEvent.keyboard("{Enter}");
    expect(api.editCandidate).not.toHaveBeenCalled();

    served.next = snapshot({ waiting: [{ ...LOCAL, text: "후보는 이 PC에만\n둔다.", edited: true }, DIFFERS, LESSON] });
    await userEvent.keyboard("e");
    await userEvent.clear(field());
    await userEvent.type(field(), "후보는 이 PC에만{Shift>}{Enter}{/Shift}둔다.{Enter}");
    expect(api.editCandidate).toHaveBeenCalledTimes(1);
    expect(api.editCandidate).toHaveBeenCalledWith(1, "후보는 이 PC에만\n둔다.", null);
    expect(await within(list()).findByText(/후보는 이 PC에만/)).toBeInTheDocument();
    expect(row({ ...LOCAL, text: "후보는 이 PC에만" })).toHaveTextContent("Edited");
  });

  it("saves the sentence when the field is left or Save is clicked, and puts it back when it is emptied", async () => {
    await open();
    await userEvent.keyboard("e");
    await userEvent.clear(field());
    await userEvent.tab();
    expect(api.editCandidate).not.toHaveBeenCalled();
    expect(detail()).toHaveTextContent(LOCAL.text);

    await userEvent.keyboard("e");
    await userEvent.keyboard(" 꼭");
    await userEvent.tab();
    expect(api.editCandidate).toHaveBeenLastCalledWith(1, `${LOCAL.text} 꼭`, null);

    await userEvent.keyboard("e");
    await userEvent.keyboard(" 반드시");
    await userEvent.click(within(detail()).getByRole("button", { name: /^Save/ }));
    expect(api.editCandidate).toHaveBeenCalledTimes(2);
    expect(api.editCandidate).toHaveBeenLastCalledWith(1, `${LOCAL.text} 반드시`, null);
  });

  it("does not take the letters typed into the sentence as decisions", async () => {
    await open();
    await userEvent.keyboard("e");
    await userEvent.keyboard("adl");
    expect(stateOf(LOCAL)).toBe("Not decided");
    expect(field()).toHaveValue(`${LOCAL.text}adl`);
  });

  it("changes whether a candidate applies to its project or to all projects", async () => {
    await open();
    const scope = within(detail()).getByRole("radiogroup", { name: "Applies to" });
    expect(within(scope).getByRole("radio", { name: /^This project · lore/ })).toHaveAttribute("aria-checked", "true");
    expect(within(scope).getByRole("radio", { name: /^All projects/ })).toHaveAttribute("aria-checked", "false");
    await userEvent.click(within(scope).getByRole("radio", { name: /^All projects/ }));
    expect(api.editCandidate).toHaveBeenCalledWith(1, null, "common");
    await userEvent.keyboard("2");
    expect(api.editCandidate).toHaveBeenCalledTimes(2);
    await userEvent.click(within(scope).getByRole("radio", { name: /^This project/ }));
    expect(api.editCandidate).toHaveBeenCalledTimes(2);
  });

  it("decides one candidate after another and moves on to the next undecided one", async () => {
    await open();
    await userEvent.click(within(detail()).getByRole("button", { name: /^Approve and next/ }));
    expect(stateOf(LOCAL)).toBe("Approved");
    expect(row(DIFFERS)).toHaveAttribute("aria-current", "true");
    await userEvent.keyboard("d");
    expect(stateOf(DIFFERS)).toBe("Discarded");
    expect(row(LESSON)).toHaveAttribute("aria-current", "true");
    await userEvent.keyboard("l");
    expect(stateOf(LESSON)).toBe("Later");
    expect(screen.getByText("3 candidates waiting · 3 decided")).toBeInTheDocument();
    expect(footer()).toHaveTextContent("1 approve");
    expect(footer()).toHaveTextContent("1 discard");
    expect(footer()).toHaveTextContent("1 later");
    expect(screen.getByRole("button", { name: "Apply: approve 1, discard 1" })).toBeEnabled();

    // Every one is decided, so it stays on the last; u takes the decision back.
    expect(row(LESSON)).toHaveAttribute("aria-current", "true");
    await userEvent.keyboard("u");
    expect(stateOf(LESSON)).toBe("Not decided");
    await userEvent.keyboard("{ArrowUp}");
    expect(row(DIFFERS)).toHaveAttribute("aria-current", "true");
    await userEvent.keyboard("a");
    expect(stateOf(DIFFERS)).toBe("Approved");
    expect(row(LESSON)).toHaveAttribute("aria-current", "true");
    expect(api.approveCandidates).not.toHaveBeenCalled();
    expect(api.discardCandidates).not.toHaveBeenCalled();
  });

  it("applies the decisions at once and goes on with what is left", async () => {
    const served = await open();
    await userEvent.keyboard("a");
    await userEvent.keyboard("d");
    served.next = snapshot({ waiting: [LESSON], leftOut: [], applying: [batch(4, { status: "applying" })] });
    await userEvent.click(screen.getByRole("button", { name: "Apply: approve 1, discard 1" }));
    expect(api.approveCandidates).toHaveBeenCalledWith([1]);
    expect(api.discardCandidates).toHaveBeenCalledWith([2]);
    expect(await screen.findByText("1 candidate waiting · 0 decided")).toBeInTheDocument();
    expect(detail()).toHaveTextContent(LESSON.text);
    expect(footer()).toHaveTextContent("Writing what you approved into the notes…");
    expect(screen.getByRole("button", { name: "Apply" })).toBeDisabled();
    expect(screen.getByRole("tab", { name: /^To review/ })).toHaveAttribute("aria-selected", "true");

    await userEvent.click(screen.getByRole("tab", { name: /^Written/ }));
    expect(screen.getByRole("region", { name: "Written for lore" })).toHaveTextContent("Writing 2 approved candidates into the notes…");
  });

  it("shows what was written once nothing is left to decide", async () => {
    const served = await open(snapshot({ waiting: [LOCAL], leftOut: [] }));
    await userEvent.keyboard("a");
    served.next = snapshot({ waiting: [], leftOut: [], applied: [WRITTEN] });
    await userEvent.click(screen.getByRole("button", { name: "Apply: approve 1" }));
    expect(api.discardCandidates).not.toHaveBeenCalled();
    const result = await screen.findByRole("region", { name: "Written for lore" });
    expect(screen.getByRole("tab", { name: /^Written/ })).toHaveAttribute("aria-selected", "true");
    expect(result).toHaveTextContent("2 candidates into 3 notes · approved by you");
    expect(result).toHaveTextContent("기록 흐름 노트를 후보 승인 방식으로 고쳤습니다.");
    expect(result).toHaveTextContent("Edited");
    expect(result).toHaveTextContent("New note");
    expect(result).toHaveTextContent("Removed");
    // A removed note has nothing to open.
    expect(within(result).queryByRole("button", { name: "Open projects/lore/diff-review.md" })).not.toBeInTheDocument();
    await userEvent.click(within(result).getByRole("button", { name: "Open projects/lore/candidate-queue.md" }));
    expect(api.openNote).toHaveBeenCalledWith("projects/lore/candidate-queue.md");
  });

  it("approves the undecided candidates together, but not one that arrived while the window was open until it is seen", async () => {
    const served = await open(snapshot({ waiting: [LOCAL, DIFFERS] }));
    served.next = snapshot();
    await stateChanged();
    expect(await within(list()).findByRole("button", { name: new RegExp(LESSON.text) })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Approve 2 undecided candidates" })).toBeEnabled();
    await userEvent.click(row(LESSON));
    await userEvent.click(screen.getByRole("button", { name: "Approve 3 undecided candidates" }));
    expect([LOCAL, DIFFERS, LESSON].map(stateOf)).toEqual(["Approved", "Approved", "Approved"]);
    expect(screen.getByRole("button", { name: "Approve 0 undecided candidates" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "Apply: approve 3" })).toBeEnabled();
    expect(api.approveCandidates).not.toHaveBeenCalled();
  });

  it("keeps its place when the candidate in view leaves the list", async () => {
    const served = await open();
    await userEvent.click(row(DIFFERS));
    served.next = snapshot({ waiting: [LOCAL, LESSON] });
    await stateChanged();
    expect(await screen.findByText("2 candidates waiting · 0 decided")).toBeInTheDocument();
    expect(row(LESSON)).toHaveAttribute("aria-current", "true");
    expect(detail()).toHaveTextContent(LESSON.text);
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

  it("says when a candidate is approved without the user", async () => {
    // Automatic approval has been on since before these candidates were found.
    await open(snapshot({ autoApprove: true, autoApproveDays: 3, autoApproveSince: NOW - 30 * DAY }));
    expect(rowsOf()[0]).toHaveTextContent("Auto-approves in 1 day");
    expect(rowsOf()[2]).toHaveTextContent("Auto-approves in 3 days");
    expect(detail()).toHaveTextContent("Auto-approves in 1 day");
    expect(footer()).toHaveTextContent("are approved automatically after 3 days");
  });

  it("counts the days of automatic approval from when it was turned on or the candidate came back to the list", async () => {
    // Turned on a moment ago: what has waited two days is not approved at once.
    await open(snapshot({ autoApprove: true, autoApproveDays: 3, autoApproveSince: NOW }));
    expect(rowsOf().map((item) => /Auto-approves in \d+ days?/.exec(item.textContent ?? "")?.[0])).toEqual(["Auto-approves in 3 days", "Auto-approves in 3 days", "Auto-approves in 3 days"]);
  });

  it("gives a candidate that was put back in the list its full waiting time again", async () => {
    const back = candidate(4, { text: "되돌아온 후보", createdAt: NOW - 20 * DAY, waitingSince: NOW - DAY });
    await open(snapshot({ waiting: [back], leftOut: [], autoApprove: true, autoApproveDays: 3, autoApproveSince: NOW - 30 * DAY }));
    expect(rowsOf()[0]).toHaveTextContent("Auto-approves in 2 days");
  });

  const writtenTab = () => userEvent.click(screen.getByRole("tab", { name: /^Written/ }));

  it("says that a written batch still waits for the search index", async () => {
    const late = batch(8, { status: "index_pending", attempts: 2, lastError: "The notes are written, but the search index could not be refreshed: qmd was not found.", result: WRITTEN.result });
    await open(snapshot({ waiting: [], leftOut: [], applied: [late] }));
    await writtenTab();
    const result = screen.getByRole("region", { name: "Written for lore" });
    expect(result).toHaveTextContent("The notes are written. The search index is not refreshed yet; Lore keeps trying.");
    expect(result).toHaveTextContent("2 candidates into 3 notes");
  });

  it("offers to try a failed batch again or to put its candidates back in the list", async () => {
    const failed = batch(6, { status: "failed", attempts: 3, lastError: "A removal must rest on a candidate that ends that note." });
    await open(snapshot({ waiting: [], leftOut: [], failed: [failed] }));
    expect(screen.getByRole("tab", { name: /^Written/ })).toHaveTextContent("1");
    await writtenTab();
    const written = screen.getByRole("region", { name: "Written for lore" });
    expect(written).toHaveTextContent("2 approved candidates could not be written. The notes are unchanged.");
    expect(written).toHaveTextContent("A removal must rest on a candidate that ends that note.");
    await userEvent.click(within(written).getByRole("button", { name: "Try again" }));
    expect(api.retryApply).toHaveBeenCalledWith(6);
    await userEvent.click(within(written).getByRole("button", { name: "Put back in the list" }));
    expect(api.releaseApply).toHaveBeenCalledWith(6);
  });

  it("says when a batch waits to be tried again", async () => {
    const retrying = batch(7, { status: "pending", attempts: 1, nextAttemptAt: NOW + 300, lastError: "The model timed out." });
    await open(snapshot({ waiting: [], leftOut: [], applying: [retrying] }));
    await writtenTab();
    expect(screen.getByRole("region", { name: "Written for lore" })).toHaveTextContent("Trying again in about 5 min. The model timed out.");
  });

  it("says that nothing waits and that nothing was written yet", async () => {
    await open(snapshot({ waiting: [], leftOut: [] }));
    expect(screen.getByText("0 candidates waiting · 0 decided")).toBeInTheDocument();
    expect(screen.getByText("No candidate is waiting. New ones appear when a session ends.")).toBeInTheDocument();
    expect(screen.getByText("Nothing left to review.")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Apply" })).toBeDisabled();
    await writtenTab();
    expect(screen.getByText("Nothing has been written yet. What you approve appears here once Lore writes it.")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Clear" })).toBeDisabled();
  });

  it("lists every batch by the day it was written, newest first, and counts on the tab what needs the user", async () => {
    // The clock is fixed so the day a batch falls on does not depend on when the test runs.
    vi.useFakeTimers({ toFake: ["Date"] });
    vi.setSystemTime(new Date(2026, 10, 18, 12, 0, 0));
    try {
      const at = Math.floor(new Date(2026, 10, 18, 11, 30, 0).getTime() / 1000);
      const writing = batch(10, { status: "applying", updatedAt: at, project: "viewer" });
      const failed = batch(11, { status: "failed", updatedAt: at - 60, project: "lore", lastError: "No plan." });
      const yesterday = batch(12, { updatedAt: at - DAY, result: WRITTEN.result });
      const older = batch(13, { updatedAt: at - 2 * DAY, project: "viewer", auto: true, candidates: 1, result: { summary: "", notes: [{ path: "projects/viewer/a.md", action: "created", items: ["하나"] }] } });
      await open(snapshot({ waiting: [], leftOut: [], applying: [writing], failed: [failed], applied: [older, yesterday] }));
      expect(screen.getByRole("tab", { name: /^Written/ })).toHaveTextContent("Written 2");
      await writtenTab();
      const days = within(screen.getByRole("list", { name: "Written batches" })).getAllByRole("list");
      expect(days.map((day) => day.getAttribute("aria-label"))).toEqual(["Today", "Yesterday", "Nov 16"]);
      expect(within(days[0]).getAllByRole("button").map((row) => row.textContent)).toEqual(["viewer · 11:30Writing 2 approved candidates into the notes…", "lore · 11:29Could not be written · 2 candidates"]);
      expect(days[2]).toHaveTextContent("1 candidate into 1 note · approved automatically");
      // The newest is shown first; another is shown when chosen.
      expect(screen.getByRole("region", { name: "Written for viewer" })).toHaveTextContent("Writing 2 approved candidates into the notes…");
      await userEvent.click(within(days[1]).getByRole("button"));
      expect(screen.getByRole("region", { name: "Written for lore" })).toHaveTextContent("Yesterday 11:30 · 2 candidates into 3 notes · approved by you");
    } finally {
      vi.useRealTimers();
    }
  });

  it("opens on the Written tab with the batch it was opened for, and turns to another one when asked again", async () => {
    const other = batch(5, { project: "viewer", updatedAt: NOW - 30, result: { summary: "뷰어 노트", notes: [{ path: "projects/viewer/a.md", action: "created", items: ["하나"] }] } });
    api.takeCandidatesFocus.mockResolvedValueOnce(4);
    await open(snapshot({ applied: [other, WRITTEN] }));
    expect(await screen.findByRole("region", { name: "Written for lore" })).toHaveTextContent("기록 흐름 노트를 후보 승인 방식으로 고쳤습니다.");
    expect(screen.getByRole("tab", { name: /^Written/ })).toHaveAttribute("aria-selected", "true");

    await userEvent.click(screen.getByRole("tab", { name: /^To review/ }));
    api.takeCandidatesFocus.mockResolvedValueOnce(5);
    await act(async () => focusListeners.forEach((listener) => listener()));
    expect(await screen.findByRole("region", { name: "Written for viewer" })).toHaveTextContent("뷰어 노트");
    expect(screen.getByRole("tab", { name: /^Written/ })).toHaveAttribute("aria-selected", "true");
  });

  it("clears the written batches after asking, and the review keys do nothing on the Written tab", async () => {
    const late = batch(8, { status: "index_pending", result: WRITTEN.result });
    await open(snapshot({ applied: [WRITTEN, late] }));
    await writtenTab();
    await userEvent.keyboard("a");
    await userEvent.click(screen.getByRole("button", { name: "Clear" }));
    const confirm = screen.getByRole("alertdialog", { name: "Clear written batches confirmation" });
    // Only what is done is cleared; a batch whose index is still refreshed stays.
    expect(confirm).toHaveTextContent("Remove 1 written batch from this list? The notes stay as they are.");
    await userEvent.click(within(confirm).getByRole("button", { name: "Cancel" }));
    expect(api.clearWritten).not.toHaveBeenCalled();
    await userEvent.click(screen.getByRole("button", { name: "Clear" }));
    await userEvent.click(within(screen.getByRole("alertdialog")).getByRole("button", { name: "Clear" }));
    expect(api.clearWritten).toHaveBeenCalledTimes(1);
    await userEvent.click(screen.getByRole("tab", { name: /^To review/ }));
    expect(stateOf(LOCAL)).toBe("Not decided");
  });

  it("reports an action that failed and keeps the decisions", async () => {
    await open();
    await userEvent.keyboard("a");
    api.approveCandidates.mockRejectedValue("The database is locked.");
    await userEvent.click(screen.getByRole("button", { name: "Apply: approve 1" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("The database is locked.");
    expect(stateOf(LOCAL)).toBe("Approved");
    expect(screen.getByRole("button", { name: "Apply: approve 1" })).toBeEnabled();
  });
});
