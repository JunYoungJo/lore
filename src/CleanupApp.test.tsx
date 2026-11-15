import { act, render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

import type { CleanupFolder, CleanupSnapshot } from "@/ipc/contract";

const listeners: (() => void)[] = [];

const api = vi.hoisted(() => ({
  getCleanup: vi.fn(),
  startCleanup: vi.fn(),
  stopCleanup: vi.fn(),
  undoCleanup: vi.fn(),
  openNote: vi.fn(),
  openReview: vi.fn(),
  onStateChanged: vi.fn((handler: () => void) => {
    listeners.push(handler);
    return Promise.resolve(() => {});
  }),
}));

vi.mock("@/ipc/api", () => api);

import { CleanupApp } from "@/CleanupApp";

const NOW = Math.floor(Date.now() / 1000);

function folder(name: string, overrides: Partial<CleanupFolder> = {}): CleanupFolder {
  return { name, notes: 5, fresh: 5, due: false, last: null, checkedAt: null, failedAt: null, failure: null, ...overrides };
}

const LORE = folder("lore", {
  notes: 39,
  fresh: 18,
  due: true,
  last: {
    at: NOW - 7200,
    kind: "changed",
    commit: "abc123",
    summary: "싱크 재시도 노트 셋을 하나로 합쳤습니다.",
    changed: ["projects/lore/sync-retries.md", "common/find-the-cause-first.md"],
    removed: [{ path: "projects/lore/old-queue-format.md", reason: "큐 파일 형식이 바뀌었습니다." }],
    moved: [{ from: "projects/lore/tomp-2438.md", to: "projects/lore/voxel-size-from-tiff.md" }],
  },
});

function cleanup(overrides: Partial<CleanupSnapshot> = {}): CleanupSnapshot {
  return {
    repository: true,
    folders: [
      folder("common", { notes: 10, fresh: 0, checkedAt: NOW - 3600 }),
      folder("appengine", { notes: 21, fresh: 21, due: true }),
      LORE,
      folder("tc-ctf", { notes: 8, fresh: 2, last: { at: NOW - 90_000, kind: "undone", commit: null, summary: "", changed: [], removed: [], moved: [] } }),
      folder("wevrel", { notes: 15, fresh: 15, due: true, failedAt: NOW - 600, failure: "The model timed out." }),
    ],
    run: null,
    blocked: null,
    auto: false,
    dueNotes: 10,
    ...overrides,
  };
}

async function open(snapshot = cleanup()) {
  api.getCleanup.mockResolvedValue(snapshot);
  render(<CleanupApp />);
  await screen.findByRole("list", { name: "Folders of notes" });
}

const row = (name: string) => screen.getByRole("listitem", { name });

beforeEach(() => {
  vi.clearAllMocks();
  listeners.length = 0;
  api.startCleanup.mockResolvedValue(undefined);
  api.stopCleanup.mockResolvedValue(undefined);
  api.undoCleanup.mockResolvedValue(null);
});

describe("CleanupApp", () => {
  it("checks the folders that have grown and cleans them with one click", async () => {
    await open();
    expect(within(row("appengine")).getByRole("checkbox")).toBeChecked();
    expect(within(row("lore")).getByRole("checkbox")).toBeChecked();
    expect(within(row("wevrel")).getByRole("checkbox")).toBeChecked();
    expect(within(row("common")).getByRole("checkbox")).not.toBeChecked();
    expect(within(row("tc-ctf")).getByRole("checkbox")).not.toBeChecked();

    await userEvent.click(screen.getByRole("button", { name: "Clean up 3 folders" }));
    expect(api.startCleanup).toHaveBeenCalledWith([
      { folder: "appengine", absorb: [] },
      { folder: "lore", absorb: [] },
      { folder: "wevrel", absorb: [] },
    ]);
  });

  const namesIn = (group: string) => within(screen.getByRole("list", { name: group })).getAllByRole("listitem").map((item) => item.getAttribute("aria-label"));

  it("lists the folders that have grown first, the one with the most changed notes on top", async () => {
    await open();
    expect(namesIn("Many changed notes")).toEqual(["appengine", "lore", "wevrel"]);
    expect(namesIn("Other folders")).toEqual(["common", "tc-ctf"]);
    // The first of them is shown, with what a cleanup will do to a folder that never had one.
    expect(within(screen.getByRole("region", { name: "appengine" })).getByText(/reads the 21 notes of this folder/)).toBeInTheDocument();
  });

  it("lists every folder under one heading when none has grown", async () => {
    await open(cleanup({ folders: [folder("common"), folder("demo")] }));
    expect(namesIn("Folders")).toEqual(["common", "demo"]);
    expect(screen.getByRole("button", { name: "Clean up" })).toBeDisabled();
  });

  it("says of each folder how it stands since its last cleanup", async () => {
    await open();
    expect(within(row("appengine")).getByText("21 notes · never cleaned up")).toBeInTheDocument();
    expect(within(row("lore")).getByText("39 notes · 18 changed since the last cleanup")).toBeInTheDocument();
    expect(within(row("common")).getByText("10 notes · nothing changed since the last cleanup")).toBeInTheDocument();
    expect(within(row("wevrel")).getByText("15 notes · the last cleanup failed")).toBeInTheDocument();
  });

  it("lets the user change which folders are cleaned", async () => {
    await open();
    await userEvent.click(within(row("lore")).getByRole("checkbox"));
    await userEvent.click(within(row("wevrel")).getByRole("checkbox"));
    await userEvent.click(within(row("tc-ctf")).getByRole("checkbox"));
    await userEvent.click(screen.getByRole("button", { name: "Clean up 2 folders" }));
    expect(api.startCleanup).toHaveBeenCalledWith([
      { folder: "appengine", absorb: [] },
      { folder: "tc-ctf", absorb: [] },
    ]);
  });

  it("shows what the last cleanup of a folder removed, moved and changed", async () => {
    await open();
    await userEvent.click(within(row("lore")).getByRole("button", { name: "lore" }));
    const detail = screen.getByRole("region", { name: "lore" });
    expect(within(detail).getByText("싱크 재시도 노트 셋을 하나로 합쳤습니다.")).toBeInTheDocument();
    expect(within(detail).getByText("old-queue-format.md")).toBeInTheDocument();
    expect(within(detail).getByText("큐 파일 형식이 바뀌었습니다.")).toBeInTheDocument();
    expect(within(detail).getByText("tomp-2438.md")).toBeInTheDocument();
    expect(within(detail).getByText("voxel-size-from-tiff.md")).toBeInTheDocument();

    await userEvent.click(within(detail).getByRole("button", { name: "Open sync-retries.md" }));
    expect(api.openNote).toHaveBeenCalledWith("projects/lore/sync-retries.md");
    // A note the cleanup added to the shared folder is listed with its folder.
    expect(within(detail).getByText("common/find-the-cause-first.md")).toBeInTheDocument();
  });

  it("undoes the last cleanup of a folder after asking", async () => {
    await open();
    await userEvent.click(within(row("lore")).getByRole("button", { name: "lore" }));
    await userEvent.click(screen.getByRole("button", { name: "Undo" }));
    expect(api.undoCleanup).not.toHaveBeenCalled();
    await userEvent.click(within(screen.getByRole("alertdialog")).getByRole("button", { name: "Undo cleanup" }));
    expect(api.undoCleanup).toHaveBeenCalledWith("lore");
    expect(await screen.findByText("The cleanup of lore was undone.")).toBeInTheDocument();
  });

  it("offers no undo for a cleanup that was undone, failed or never ran", async () => {
    await open();
    for (const name of ["tc-ctf", "wevrel", "appengine", "common"]) {
      await userEvent.click(within(row(name)).getByRole("button", { name }));
      expect(screen.queryByRole("button", { name: "Undo" })).not.toBeInTheDocument();
    }
    await userEvent.click(within(row("wevrel")).getByRole("button", { name: "wevrel" }));
    expect(screen.getByText(/The model timed out\./)).toBeInTheDocument();
    await userEvent.click(within(row("tc-ctf")).getByRole("button", { name: "tc-ctf" }));
    expect(screen.getByText(/The last cleanup was undone/)).toBeInTheDocument();
  });

  it("shows a run folder by folder and stops it on request", async () => {
    await open(
      cleanup({
        run: {
          id: NOW - 120,
          items: [
            { folder: "appengine", absorb: [], state: "done", startedAt: NOW - 120 },
            { folder: "lore", absorb: [], state: "running", startedAt: NOW - 60 },
            { folder: "wevrel", absorb: [], state: "waiting", startedAt: null },
          ],
        },
      }),
    );
    // The folders of the run come first, in the order they are cleaned, and the one being cleaned is shown.
    expect(namesIn("This cleanup")).toEqual(["appengine", "lore", "wevrel"]);
    expect(namesIn("Other folders")).toEqual(["common", "tc-ctf"]);
    expect(screen.getByRole("region", { name: "lore" })).toBeInTheDocument();
    expect(within(row("appengine")).getByText("Done")).toBeInTheDocument();
    expect(within(row("lore")).getByText("Cleaning up…")).toBeInTheDocument();
    expect(within(row("wevrel")).getByText("Waiting")).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /^Clean up \d/ })).not.toBeInTheDocument();
    expect(within(row("lore")).getByRole("checkbox")).toBeDisabled();

    await userEvent.click(screen.getByRole("button", { name: "Stop" }));
    expect(api.stopCleanup).toHaveBeenCalled();
  });

  it("says why a run that waits has not started", async () => {
    await open(cleanup({ run: { id: NOW, items: [{ folder: "lore", absorb: [], state: "waiting", startedAt: null }] }, blocked: "Starts when recording is resumed." }));
    expect(screen.getByText("Starts when recording is resumed.")).toBeInTheDocument();
  });

  it("merges one project folder into another as a separate action", async () => {
    await open();
    await userEvent.click(within(row("tc-ctf")).getByRole("button", { name: "tc-ctf" }));
    // Merging is rare, so its form stays closed until it is asked for.
    expect(screen.queryByRole("group", { name: "Merge into another folder" })).not.toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Merge into another folder…" }));
    const merge = screen.getByRole("group", { name: "Merge into another folder" });
    // The shared folder is not a project, so it is neither offered as a destination nor mergeable.
    expect(within(merge).queryByRole("option", { name: "common" })).not.toBeInTheDocument();
    expect(within(merge).getByRole("button", { name: "Merge and clean up" })).toBeDisabled();
    await userEvent.selectOptions(within(merge).getByRole("combobox"), "lore");
    await userEvent.click(within(merge).getByRole("button", { name: "Merge and clean up" }));
    expect(api.startCleanup).toHaveBeenCalledWith([{ folder: "lore", absorb: ["tc-ctf"] }]);

    await userEvent.click(within(row("common")).getByRole("button", { name: "common" }));
    expect(screen.queryByRole("button", { name: "Merge into another folder…" })).not.toBeInTheDocument();
  });

  it("reloads when the app state changes and checks the due folders again after a run", async () => {
    const running = cleanup({ run: { id: NOW, items: [{ folder: "lore", absorb: [], state: "running", startedAt: NOW }] } });
    await open(running);
    api.getCleanup.mockResolvedValue(cleanup({ folders: [folder("appengine", { notes: 21, fresh: 21, due: true }), { ...LORE, fresh: 0, due: false }] }));
    await act(async () => listeners.forEach((listener) => listener()));
    expect(await screen.findByRole("button", { name: "Clean up 1 folder" })).toBeInTheDocument();
    expect(within(row("lore")).getByRole("checkbox")).not.toBeChecked();
  });

  it("reports a failed request", async () => {
    await open();
    api.startCleanup.mockRejectedValue("A cleanup is already waiting or running.");
    await userEvent.click(screen.getByRole("button", { name: "Clean up 3 folders" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("A cleanup is already waiting or running.");
  });
});
