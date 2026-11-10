import { Bot, Database, FolderOpen, Gauge, Plug, Power } from "lucide-react";

import * as api from "@/ipc/api";
import type { EditableSettings, HookStatus, Settings, Snapshot } from "@/ipc/contract";
import { providerLabel } from "@/format";
import { cleanModels, editable, useAppStore } from "@/store/appStore";
import { IconChip } from "@/ui/IconChip";
import { Segmented } from "@/ui/Segmented";
import { Toggle } from "@/ui/Toggle";

const DEFAULT_MODEL = { codex: "gpt-6.1-sol", claude: "haiku" } as const;
const STATE_LABEL = { connected: "Connected", other_path: "Different path", missing: "Not connected" } as const;

function HookRow({ hook }: { hook: HookStatus }) {
  const connecting = useAppStore((store) => store.connecting);
  const connectHook = useAppStore((store) => store.connectHook);
  const disconnectHook = useAppStore((store) => store.disconnectHook);
  const busy = connecting === hook.provider;
  return (
    <div className="hook" aria-label={`${providerLabel(hook.provider)} hooks`}>
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
        <span className="row-kind">Working…</span>
      ) : (
        <div className="hook-actions">
          {hook.state !== "connected" && (
            <button type="button" className="pill primary" disabled={connecting !== null} onClick={() => void connectHook(hook.provider)}>
              {hook.state === "other_path" ? "Reconnect" : "Connect"}
            </button>
          )}
          {hook.state !== "missing" && (
            <button type="button" className="pill ghost" disabled={connecting !== null} onClick={() => void disconnectHook(hook.provider)}>
              Disconnect
            </button>
          )}
        </div>
      )}
    </div>
  );
}

type LimitKey = "modelTimeoutSecs" | "qmdTimeoutSecs" | "maxChars" | "notesBudget";

const LIMITS: { key: LimitKey; id: string; label: string }[] = [
  { key: "modelTimeoutSecs", id: "model-timeout", label: "Recording timeout (seconds)" },
  { key: "qmdTimeoutSecs", id: "qmd-timeout", label: "qmd timeout (seconds)" },
  { key: "maxChars", id: "max-chars", label: "New conversation per request (characters)" },
  { key: "notesBudget", id: "notes-budget", label: "Existing notes per request (characters)" },
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
      <section className="card" aria-label="Recording model">
        <div className="card-title">
          <Bot size={15} strokeWidth={1.9} />
          Recording model
        </div>
        <span className="field-label">CLI</span>
        <Segmented
          label="CLI"
          value={draft.backend}
          options={[
            { value: "codex", label: "Codex" },
            { value: "claude", label: "Claude" },
          ]}
          onChange={(backend) => editDraft({ backend })}
        />
        <label className="field-label" htmlFor="model">
          Model
        </label>
        <input
          id="model"
          className="input mono"
          value={draft.models[draft.backend] ?? ""}
          placeholder={`${DEFAULT_MODEL[draft.backend]} (default)`}
          onChange={(event) => editDraft({ models: { ...draft.models, [draft.backend]: event.target.value } })}
        />
        <div className="help">
          Each recording runs {draft.backend === "claude" ? "claude --print" : "codex exec"} and uses your signed-in account’s allowance.
        </div>
      </section>

      <section className="card" aria-label="Hook connections">
        <div className="card-title">
          <Plug size={15} strokeWidth={1.9} />
          Hook connections
        </div>
        {snapshot.hooks.map((hook) => (
          <HookRow key={hook.provider} hook={hook} />
        ))}
        <div className="help">
          Connecting adds qmd-memory-cli to session-end and pre-compaction hooks. Disconnecting removes only Lore’s hooks. Existing hooks are kept, and the settings file is backed up before changes.
          <br />
          In Codex, trust the new hooks in /hooks after connecting.
        </div>
      </section>

      <section className="card" aria-label="Memory repository">
        <div className="card-title">
          <Database size={15} strokeWidth={1.9} />
          Memory repository
        </div>
        <label className="field-label" htmlFor="knowledge-root">
          Location
        </label>
        <div className="input-row">
          <input id="knowledge-root" className="input mono" value={draft.knowledgeRoot} onChange={(event) => editDraft({ knowledgeRoot: event.target.value })} />
          <button type="button" className="icon-btn" title="Open saved location" aria-label="Open saved location" onClick={() => void api.openFolder("knowledge")}>
            <FolderOpen size={16} strokeWidth={1.9} />
          </button>
        </div>
        <div className="help">Memory notes live in common/ and projects/. Supporting conversation copies live in sources/.</div>
      </section>

      <section className="card" aria-label="Processing limits">
        <div className="card-title">
          <Gauge size={15} strokeWidth={1.9} />
          Processing limits
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
          Timed-out calls are stopped and retried later. Saved timeouts apply to running calls; character limits apply to the next recording. If existing notes exceed the limit, only relevant notes are sent in full; the rest are listed by title.
        </div>
      </section>

      <section className="card" aria-label="Startup">
        <div className="toggle-row">
          <IconChip glyph={Power} tint="blue" size={30} />
          <div className="toggle-text">
            <div className="toggle-title">Launch at login</div>
            <div className="toggle-sub">Start recording from the tray when you sign in.</div>
          </div>
          <Toggle label="Launch at login" checked={snapshot.autostart} onChange={(enabled) => void setAutostart(enabled)} />
        </div>
        <button type="button" className="link-btn" onClick={() => void api.openFolder("data")}>
          <FolderOpen size={13} strokeWidth={1.9} />Open app data folder
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
        Cancel
      </button>
      <button type="button" className="btn primary" disabled={!dirty || saving} onClick={() => void saveDraft()}>
        {saving ? "Saving…" : "Save"}
      </button>
    </footer>
  );
}
