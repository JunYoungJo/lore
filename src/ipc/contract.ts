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

export type EditableSettings = Pick<Settings, "knowledgeRoot" | "backend" | "models" | "efforts" | "modelTimeoutSecs" | "qmdTimeoutSecs" | "maxChars" | "notesBudget">;

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
  others: FileChange[];
  message: string;
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

export interface SyncSummary { phase: string; message: string | null; issues: number; blocked: boolean; lastSyncedAt: number | null }
export interface SyncFile { path: string; base: string | null; local: string | null; remote: string | null; merged: string | null; textConflict: boolean }
export interface SyncSource { id: string; side: string; path: string; text: string }
export interface SyncEdit { path: string; find: string; text: string }
export interface SyncOption { label: string; description: string; edits: SyncEdit[] }
export interface SyncIssue { title: string; explanation: string; evidence: { source: string; quote: string }[]; options: SyncOption[] }
export interface SyncAnalysis { summary: string; issues: SyncIssue[] }
export interface SyncPlan {
  id: string; root: string; branch: string; remoteName: string; remoteRef: string;
  local: string; remote: string; base: string; tree: string; phase: string;
  files: SyncFile[]; sources: SyncSource[]; unsupported: string[]; otherFiles: string[];
  analysis: SyncAnalysis | null; approved: string | null; message: string | null;
}
export interface LineCount { added: number; removed: number }
export interface SyncFileLines { path: string; local: LineCount; remote: LineCount }
export interface SyncComparisonProgress { completed: number; total: number; stage: string; reads: number; error: string | null }
export interface SyncSnapshot { root: string; enabled: boolean; busy: boolean; summary: SyncSummary; plan: SyncPlan | null; tracking: string | null; model: string; review: number | null; changes: SyncFileLines[]; comparison?: SyncComparisonProgress | null }
export interface SyncChoice { issue: number; option: number; customText: string | null }
export interface SyncPreview { files: Record<string, string>; diffs: Record<string, DiffRow[]>; token: string }
