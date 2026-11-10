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

export interface Settings {
  knowledgeRoot: string;
  backend: Backend;
  models: Models;
  modelTimeoutSecs: number;
  qmdTimeoutSecs: number;
  claudeProjectsDir: string;
  codexSessionsDir: string;
  idleSecs: number;
  since: number;
  paused: boolean;
  maxChars: number;
  notesBudget: number;
  qmdIndex: string;
  commands: Commands;
}

export type EditableSettings = Pick<Settings, "knowledgeRoot" | "backend" | "models" | "modelTimeoutSecs" | "qmdTimeoutSecs" | "maxChars" | "notesBudget">;

export type HookState = "connected" | "other_path" | "missing";

export interface HookStatus {
  provider: Provider;
  state: HookState;
  config: string;
  command: string;
  found: string | null;
  error: string | null;
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

export interface Snapshot {
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
  review: number | null;
  reviewing: boolean;
  version: string;
}

export interface SyncSummary { phase: string; message: string | null; issues: number; blocked: boolean }
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
export interface SyncSnapshot { root: string; enabled: boolean; busy: boolean; summary: SyncSummary; plan: SyncPlan | null }
export interface SyncChoice { issue: number; option: number; customText: string | null }
export interface SyncPreview { files: Record<string, string>; token: string }
