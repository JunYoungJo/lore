export type Provider = "claude" | "codex";

export type JobStatus = "pending" | "applying" | "index_pending" | "done" | "failed" | "skipped";

export type ActivityKind = "recorded" | "no_change" | "skipped" | "failed" | "retry_scheduled" | "started";

export interface Current {
  jobId: string;
  provider: Provider;
  project: string;
  startedAt: number;
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
  maxChars: number;
  notesBudget: number;
  qmdIndex: string;
  commands: Commands;
}

export type EditableSettings = Pick<Settings, "knowledgeRoot" | "backend" | "fallback" | "models" | "efforts" | "modelTimeoutSecs" | "qmdTimeoutSecs" | "maxChars" | "notesBudget">;

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
  counts: Counts;
  recent: Activity[];
  attention: Job[];
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
