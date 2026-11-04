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

export interface Settings {
  knowledgeRoot: string;
  backend: "codex" | "claude";
  model: string | null;
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

export type HookState = "connected" | "other_path" | "missing";

export interface HookStatus {
  provider: Provider;
  state: HookState;
  config: string;
  command: string;
  found: string | null;
  error: string | null;
}

export interface Snapshot {
  state: "idle" | "working" | "paused";
  current: Current | null;
  counts: Counts;
  recent: Activity[];
  attention: Job[];
  problem: string | null;
  settings: Settings;
  autostart: boolean;
  hooks: HookStatus[];
  version: string;
}
