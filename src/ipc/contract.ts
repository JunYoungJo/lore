export type Provider = "claude" | "codex";

/** A session is only read. Writing notes is the work of an apply batch (`ApplyBatch`). */
export type JobStatus = "pending" | "done" | "failed" | "skipped";

/** `offered`: a session offered candidates. `recorded`: approved candidates were written into notes. */
export type ActivityKind = "offered" | "recorded" | "no_change" | "skipped" | "failed" | "retry_scheduled" | "started" | "cleaned" | "cleanup_failed" | "cleanup_undone";

export interface Current {
  jobId: string;
  provider: Provider;
  project: string;
  startedAt: number;
  /** Approved candidates are being written into the notes, as opposed to a session being read. */
  applying: boolean;
}

export type CandidateKind = "decision" | "correction" | "lesson";

/** Words of the conversation a candidate rests on. The program has checked them against the turn. */
export interface Quote {
  ref: string;
  quote: string;
  speaker: "user" | "agent";
  /** When it was said, in the user's local time. Empty when the session file has no time. */
  at: string;
}

/** How a candidate stands to the notes that exist. `quotes` are passages of `note`, word for word. */
export interface Relation {
  type: "new" | "same" | "differs" | "obsolete";
  note: string;
  title: string;
  quotes: string[];
}

export type CandidateScope = "project" | "common";

/** One statement a session offers to remember. It waits on this PC until the user decides. */
export interface Candidate {
  id: number;
  jobId: string;
  sessionId: string;
  provider: Provider;
  project: string;
  language: string;
  kind: CandidateKind;
  text: string;
  /** `marked`: the session flagged it while working. `found`: the model found it afterwards. */
  source: "marked" | "found";
  scope: CandidateScope;
  evidence: Quote[];
  relation: Relation;
  state: "waiting" | "approved" | "applied" | "discarded" | "duplicate";
  edited: boolean;
  createdAt: number;
  /** When it last entered the waiting list: when it was found, or when a batch that could not be written was given up. */
  waitingSince: number;
  applyId: number | null;
}

export interface NoteResult {
  path: string;
  action: "created" | "edited" | "removed";
  /** The approved sentences this note took in. */
  items: string[];
}

/** Approved candidates of one project, written into the notes together. */
export interface ApplyBatch {
  id: number;
  project: string;
  /** `index_pending`: the notes are written and only the search index is still to be refreshed. */
  status: "pending" | "applying" | "index_pending" | "done" | "failed";
  /** Approved by the waiting period, not by the user. */
  auto: boolean;
  attempts: number;
  nextAttemptAt: number;
  lastError: string | null;
  result: { summary: string; notes: NoteResult[] } | null;
  candidates: number;
  updatedAt: number;
}

export interface CandidatesSnapshot {
  waiting: Candidate[];
  /** What was left out of those sessions because a note already says it. */
  leftOut: Candidate[];
  /** Batches whose notes are not written yet. */
  applying: ApplyBatch[];
  /** Batches that gave up. Nothing of them is in the notes. */
  failed: ApplyBatch[];
  /** The batches written most recently, newest first. */
  applied: ApplyBatch[];
  autoApprove: boolean;
  autoApproveDays: number;
  /** When automatic approval was turned on. Its days count from the later of this and `waitingSince`. */
  autoApproveSince: number;
  now: number;
}

/** The candidates that wait for the user. `oldestAt` is 0 when none waits. */
export interface Waiting {
  candidates: number;
  sessions: number;
  oldestAt: number;
}

/** One folder of a cleanup run. `absorb` names the folders merged into it. `startedAt` is null while it waits. */
export interface CleanupItem {
  folder: string;
  absorb: string[];
  state: "waiting" | "running" | "done" | "failed";
  startedAt: number | null;
}

/** The folders one request asked to clean up, cleaned one after another. It is gone once the last one is done. */
export interface CleanupRun {
  id: number;
  items: CleanupItem[];
}

/** A folder to clean up and the folders to merge into it first. */
export interface CleanupTarget {
  folder: string;
  absorb: string[];
}

/** The last cleanup that changed a folder, or the undoing of it. `commit` is null when it cannot be undone. */
export interface CleanupLast {
  at: number;
  kind: "changed" | "undone";
  commit: string | null;
  summary: string;
  changed: string[];
  removed: { path: string; reason: string }[];
  moved: { from: string; to: string }[];
}

/** A folder of notes that can be cleaned up: "common" or a project. */
export interface CleanupFolder {
  name: string;
  notes: number;
  /** Notes that are new or changed since the last cleanup; all of them when there was none. */
  fresh: number;
  due: boolean;
  last: CleanupLast | null;
  /** When a later run looked at the folder and found nothing to change. */
  checkedAt: number | null;
  failedAt: number | null;
  failure: string | null;
}

export interface CleanupSnapshot {
  repository: boolean;
  folders: CleanupFolder[];
  run: CleanupRun | null;
  /** Why a run that waits has not started yet. */
  blocked: string | null;
  auto: boolean;
  dueNotes: number;
}

export interface Counts {
  ready: number;
  waiting: number;
  failed: number;
  done: number;
  skipped: number;
}

export interface Activity {
  id: number;
  at: number;
  jobId: string;
  provider: Provider;
  project: string | null;
  kind: ActivityKind;
  /** The first line is the sentence to show. Later lines name each note that was removed or moved (see `goneNotes`). */
  summary: string | null;
  notes: string[];
  error: string | null;
}

export interface Job {
  id: string;
  provider: Provider;
  transcript: string;
  status: JobStatus;
  cursorLine: number;
  cursorHash: string;
  project: string | null;
  sessionId: string | null;
  attempts: number;
  nextAttemptAt: number;
  lastError: string | null;
  updatedAt: number;
}

export interface Commands {
  codex: string[];
  claude: string[];
  qmd: string[];
}

export type Backend = "codex" | "claude";

export type Models = Record<Backend, string | null>;

export type Efforts = Record<Backend, string | null>;

export interface ModelChoice {
  value: string;
  label: string;
  resolved: string | null;
  efforts: string[];
}

export type Catalog = Record<Backend, ModelChoice[] | null>;

export interface Settings {
  knowledgeRoot: string;
  backend: Backend;
  /** Use the other CLI when `backend` cannot run. */
  fallback: boolean;
  models: Models;
  efforts: Efforts;
  modelTimeoutSecs: number;
  qmdTimeoutSecs: number;
  claudeProjectsDir: string;
  codexSessionsDir: string;
  idleSecs: number;
  since: number;
  paused: boolean;
  syncEnabled: boolean;
  /** Clean up a folder of notes without being asked once enough of its notes have changed. */
  autoCleanup: boolean;
  /** Approve a candidate as it stands once it has waited `autoApproveDays`. Off: it waits for the user. */
  autoApprove: boolean;
  autoApproveDays: number;
  /** When automatic approval was turned on; 0 while it is off. */
  autoApproveSince: number;
  /** How much new conversation one step of a session reads. */
  maxChars: number;
  qmdIndex: string;
  commands: Commands;
}

export type EditableSettings = Pick<Settings, "knowledgeRoot" | "backend" | "fallback" | "models" | "efforts" | "modelTimeoutSecs" | "qmdTimeoutSecs" | "maxChars">;

export type HookState = "connected" | "other_path" | "missing";

export interface HookStatus {
  provider: Provider;
  state: HookState;
  config: string;
  command: string;
  found: string | null;
  error: string | null;
}

export type ConnectionState = "connected" | "partial" | "missing";

export interface Connection {
  provider: Provider;
  state: ConnectionState;
}

export type ChangeKind = "modified" | "added" | "deleted";

export interface FileChange {
  path: string;
  kind: ChangeKind;
}

export interface ReviewSession {
  at: number;
  provider: Provider;
  project: string | null;
  summary: string | null;
}

export interface ReviewFile extends FileChange {
  sessions: ReviewSession[];
}

export interface ReviewSnapshot {
  repository: boolean;
  waiting: boolean;
  root: string;
  files: ReviewFile[];
}

export interface Segment {
  kind: "same" | "added" | "removed";
  text: string;
}

export type RowKind = "same" | "changed" | "removed" | "added";

export interface DiffRow {
  kind: RowKind;
  leftNo: number | null;
  left: Segment[] | null;
  rightNo: number | null;
  right: Segment[] | null;
}

export interface AppUpdate {
  phase: "idle" | "checking" | "current" | "available" | "downloading" | "installing" | "error";
  version: string | null;
  notes: string | null;
  downloaded: number;
  total: number | null;
  error: string | null;
}

export interface Snapshot {
  update: AppUpdate;
  sync?: SyncSummary;
  syncing?: boolean;
  state: "idle" | "working" | "paused" | "reviewing";
  current: Current | null;
  cleanup: CleanupRun | null;
  /** The folders with enough changed notes to be worth a cleanup, as of the last look. */
  cleanupDue: { name: string; fresh: number }[];
  counts: Counts;
  recent: Activity[];
  attention: Job[];
  candidates: Waiting;
  /** Approved batches that could not be written into the notes. */
  failedApplies: number;
  problem: string | null;
  settings: Settings;
  autostart: boolean;
  hooks: HookStatus[];
  connections: Connection[];
  review: number | null;
  reviewing: boolean;
  version: string;
  catalog: Catalog;
  defaultModels: Record<Backend, string>;
  defaultEfforts: Efforts;
  cliAvailable: Record<Backend, boolean>;
}

export interface SetupProgress {
  step: number;
  completed: boolean;
  deferred: boolean;
  providers: Provider[];
  testStartedAt: number | null;
  recallConfirmed: boolean;
}

export interface SetupResource {
  label: string;
  path: string;
  state: "add" | "ready" | "update" | "preserved" | "conflict";
  detail: string;
  content: string;
}

export interface SetupPlan { token: string; resources: SetupResource[]; canApply: boolean }
export interface SetupClient { provider: Provider; plan: SetupPlan | null; error: string | null; hooks: HookStatus }
export interface SetupSnapshot {
  progress: SetupProgress;
  settings: Settings;
  diagnostics: {
    commands: Record<"codex" | "claude" | "qmd", { command: string[]; available: boolean }>;
    indexConfigured: boolean;
    problems: string[];
    qmdConfigDir: string;
    qmdCacheDir: string;
    hookCli: string;
  };
  tools: { name: string; path: string | null }[];
  clients: SetupClient[];
  platform: string;
  recorded: number;
  testRecorded: boolean;
  rootExists: boolean;
  qwenModel: string;
  embedModel: string | null;
  embedModels: EmbedModel[];
  collections: string[];
  repository: boolean;
  tracking: string | null;
}

export interface EmbedModel { uri: string; name: string; size: string; note: string }

export interface SetupCommand { task: string; remote?: string; path?: string; query?: string }
export interface PreparedCommand { command: string; argv: string[]; cwd: string; env: [string, string][]; token: string }
export interface TerminalOutput { bytes: number[]; offset: number; running: boolean; exitCode: number | null }
export interface ToolChecks {
  nodeVersion: string | null;
  nodeReady: boolean;
  auth: Record<Provider, "ready" | "signed_out" | "unavailable" | "unknown">;
}

/** `retrying`: a sync failed and is retried quietly. `failing`: it failed several times in a row. */
export interface SyncSummary { phase: "idle" | "retrying" | "failing"; message: string | null; items: number; comparing: boolean; lastSyncedAt: number | null }
export interface SyncFile { path: string; base: string | null; local: string | null; remote: string | null; merged: string | null; textConflict: boolean }
export interface SyncSource { id: string; side: string; path: string; text: string }
export interface SyncEdit { path: string; find: string; text: string }
export interface SyncOption { label: string; description: string; edits: SyncEdit[] }
export interface SyncIssue { title: string; explanation: string; evidence: { source: string; quote: string }[]; options: SyncOption[] }
/** Something to look at later. `merged`: both PCs' lines were kept. `kept`: deleted on one PC, edited on the other. `decision`: the model found notes that disagree. */
export interface SyncItem { id: string; kind: "merged" | "kept" | "decision"; plan: string; path: string; title: string; detail: string; createdAt: number; issue: number | null }
export interface SyncItemFile { path: string; base: string | null; local: string | null; remote: string | null; merged: string | null; rows: DiffRow[] }
export interface SyncItemView { item: SyncItem; files: SyncItemFile[]; issue: SyncIssue | null; sources: SyncSource[] }
export interface SyncComparisonProgress { completed: number; total: number; stage: string; reads: number; error: string | null }
export interface SyncSnapshot { root: string; enabled: boolean; busy: boolean; summary: SyncSummary; tracking: string | null; model: string; review: number | null; items: SyncItem[]; comparison: SyncComparisonProgress | null; comparisonError: string | null; comparisonRetryAt: number | null; comparisonRetryReason: string | null; indexPending: boolean }
