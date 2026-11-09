import { Bot, Database, FolderOpen, Gauge, Plug, Power } from "lucide-react";

import * as api from "@/ipc/api";
import type { EditableSettings, HookStatus, Settings, Snapshot } from "@/ipc/contract";
import { providerLabel } from "@/format";
import { cleanModels, editable, useAppStore } from "@/store/appStore";
import { IconChip } from "@/ui/IconChip";
import { Segmented } from "@/ui/Segmented";
import { Toggle } from "@/ui/Toggle";

const DEFAULT_MODEL = { codex: "gpt-6.1-sol", claude: "haiku" } as const;
const STATE_LABEL = { connected: "연결됨", other_path: "다른 경로", missing: "연결 안 됨" } as const;

function HookRow({ hook }: { hook: HookStatus }) {
  const connecting = useAppStore((store) => store.connecting);
  const connectHook = useAppStore((store) => store.connectHook);
  const disconnectHook = useAppStore((store) => store.disconnectHook);
  const busy = connecting === hook.provider;
  return (
    <div className="hook" aria-label={`${providerLabel(hook.provider)} 훅`}>
      <IconChip glyph={Bot} tint={hook.provider === "claude" ? "amber" : "indigo"} size={30} />
      <div className="hook-main">
        <div className="hook-name">
          {hook.provider === "claude" ? "Claude Code" : "Codex"}
          <span className={`state ${hook.state}`}>{STATE_LABEL[hook.state]}</span>
        </div>
        <div className="hook-path" title={hook.config}>
          {hook.config}
        </div>
        {hook.error && <div className="hook-error">{hook.error}</div>}
      </div>
      {busy ? (
        <span className="row-kind">처리 중…</span>
      ) : (
        <div className="hook-actions">
          {hook.state !== "connected" && (
            <button type="button" className="pill primary" disabled={connecting !== null} onClick={() => void connectHook(hook.provider)}>
              {hook.state === "other_path" ? "다시 연결" : "연결"}
            </button>
          )}
          {hook.state !== "missing" && (
            <button type="button" className="pill ghost" disabled={connecting !== null} onClick={() => void disconnectHook(hook.provider)}>
              해제
            </button>
          )}
        </div>
      )}
    </div>
  );
}

type LimitKey = "modelTimeoutSecs" | "qmdTimeoutSecs" | "maxChars" | "notesBudget";

const LIMITS: { key: LimitKey; id: string; label: string }[] = [
  { key: "modelTimeoutSecs", id: "model-timeout", label: "기록 모델 시간 제한(초)" },
  { key: "qmdTimeoutSecs", id: "qmd-timeout", label: "qmd 색인 시간 제한(초)" },
  { key: "maxChars", id: "max-chars", label: "한 번에 보내는 새 대화(자)" },
  { key: "notesBudget", id: "notes-budget", label: "한 번에 보내는 기존 문서(자)" },
];

function isDirty(draft: EditableSettings, saved: Settings): boolean {
  const current = editable(saved);
  const models = (value: EditableSettings["models"]) => JSON.stringify(cleanModels(value));
  return (Object.keys(current) as (keyof EditableSettings)[]).some((key) => (key === "models" ? models(draft.models) !== models(current.models) : draft[key] !== current[key]));
}

export function SettingsView({ snapshot }: { snapshot: Snapshot }) {
  const draft = useAppStore((store) => store.draft);
  const editDraft = useAppStore((store) => store.editDraft);
  const setAutostart = useAppStore((store) => store.setAutostart);
  if (!draft) return null;

  return (
    <div className="body settings">
      <section className="card" aria-label="기록 모델">
        <div className="card-title">
          <Bot size={15} strokeWidth={1.9} />
          기록 모델
        </div>
        <span className="field-label">실행할 CLI</span>
        <Segmented
          label="실행할 CLI"
          value={draft.backend}
          options={[
            { value: "codex", label: "Codex" },
            { value: "claude", label: "Claude" },
          ]}
          onChange={(backend) => editDraft({ backend })}
        />
        <label className="field-label" htmlFor="model">
          모델
        </label>
        <input
          id="model"
          className="input mono"
          value={draft.models[draft.backend] ?? ""}
          placeholder={`${DEFAULT_MODEL[draft.backend]} (기본값)`}
          onChange={(event) => editDraft({ models: { ...draft.models, [draft.backend]: event.target.value } })}
        />
        <div className="help">
          기록할 때마다 {draft.backend === "claude" ? "claude --print" : "codex exec"}가 한 번 실행되고, 로그인한 계정의 사용 한도에서 차감됩니다.
        </div>
      </section>

      <section className="card" aria-label="훅 연결">
        <div className="card-title">
          <Plug size={15} strokeWidth={1.9} />
          훅 연결
        </div>
        {snapshot.hooks.map((hook) => (
          <HookRow key={hook.provider} hook={hook} />
        ))}
        <div className="help">
          연결하면 세션 종료 훅과 컨텍스트 압축 직전 훅에 qmd-memory-cli를 추가하고, 해제하면 qmd-memory 훅만 지웁니다. 다른 훅은 그대로 두고, 바꾸기 전 파일은 같은 폴더에 백업합니다.
          <br />
          Codex는 연결 후 /hooks에서 새 훅을 신뢰로 표시해야 실행됩니다.
        </div>
      </section>

      <section className="card" aria-label="기억 저장소">
        <div className="card-title">
          <Database size={15} strokeWidth={1.9} />
          기억 저장소
        </div>
        <label className="field-label" htmlFor="knowledge-root">
          위치
        </label>
        <div className="input-row">
          <input id="knowledge-root" className="input mono" value={draft.knowledgeRoot} onChange={(event) => editDraft({ knowledgeRoot: event.target.value })} />
          <button type="button" className="icon-btn" title="저장된 위치 열기" aria-label="저장된 위치 열기" onClick={() => void api.openFolder("knowledge")}>
            <FolderOpen size={16} strokeWidth={1.9} />
          </button>
        </div>
        <div className="help">common/과 projects/에 기억 문서를, sources/에 근거 대화 사본을 둡니다.</div>
      </section>

      <section className="card" aria-label="처리 한도">
        <div className="card-title">
          <Gauge size={15} strokeWidth={1.9} />
          처리 한도
        </div>
        <div className="field-grid">
          {LIMITS.map(({ key, id, label }) => (
            <div key={key}>
              <label className="field-label" htmlFor={id}>
                {label}
              </label>
              <input
                id={id}
                className="input mono"
                type="number"
                min={1}
                inputMode="numeric"
                value={draft[key] || ""}
                onChange={(event) => editDraft({ [key]: Number(event.target.value) })}
              />
            </div>
          ))}
        </div>
        <div className="help">
          시간 제한을 넘긴 실행은 중단하고 나중에 다시 시도합니다. 저장하면 시간 제한은 지금 실행 중인 호출에도 바로 적용되고, 글자 수 한도는 다음 기록부터 적용됩니다. 기존 문서가 한도를 넘으면 관련 문서만 본문을 보내고 나머지는 제목만 보냅니다.
        </div>
      </section>

      <section className="card" aria-label="시작">
        <div className="toggle-row">
          <IconChip glyph={Power} tint="blue" size={30} />
          <div className="toggle-text">
            <div className="toggle-title">Windows 로그인 시 자동 실행</div>
            <div className="toggle-sub">로그인하면 트레이에서 바로 기록을 시작합니다.</div>
          </div>
          <Toggle label="Windows 로그인 시 자동 실행" checked={snapshot.autostart} onChange={(enabled) => void setAutostart(enabled)} />
        </div>
        <button type="button" className="link-btn" onClick={() => void api.openFolder("data")}>
          <FolderOpen size={13} strokeWidth={1.9} />앱 데이터 폴더 열기
        </button>
      </section>
    </div>
  );
}

export function SettingsFooter({ snapshot }: { snapshot: Snapshot }) {
  const draft = useAppStore((store) => store.draft);
  const saving = useAppStore((store) => store.saving);
  const error = useAppStore((store) => store.error);
  const saveDraft = useAppStore((store) => store.saveDraft);
  const showStatus = useAppStore((store) => store.showStatus);
  if (!draft) return null;
  const dirty = isDirty(draft, snapshot.settings);
  return (
    <footer className="settings-foot">
      <span className="grow">{error}</span>
      <button type="button" className="btn ghost" onClick={showStatus}>
        취소
      </button>
      <button type="button" className="btn primary" disabled={!dirty || saving} onClick={() => void saveDraft()}>
        {saving ? "저장 중…" : "저장"}
      </button>
    </footer>
  );
}
