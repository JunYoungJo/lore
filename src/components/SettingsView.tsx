import { useEffect, useState } from "react";
import { Check, ChevronRight, FolderOpen } from "lucide-react";

import * as api from "@/ipc/api";
import type { ConnectionState, Snapshot } from "@/ipc/contract";
import { effortsFor, findChoice, HEAVY_EFFORTS } from "@/recordingModel";
import { useAppStore } from "@/store/appStore";
import { syncState } from "@/syncState";
import { useThemePreference, type ThemePreference } from "@/theme";
import { Segmented } from "@/ui/Segmented";
import { Toggle } from "@/ui/Toggle";
import { useNow } from "@/useNow";

const OTHER = "__other";
const CLIENT = { claude: "Claude Code", codex: "Codex" } as const;
const CONNECTION: Record<ConnectionState, string> = { connected: "Connected", partial: "Needs repair", missing: "Not connected" };

type LimitKey = "modelTimeoutSecs" | "qmdTimeoutSecs" | "maxChars" | "notesBudget";

const LIMITS: { key: LimitKey; id: string; label: string; unit: string }[] = [
  { key: "modelTimeoutSecs", id: "model-timeout", label: "Recording timeout", unit: "sec" },
  { key: "qmdTimeoutSecs", id: "qmd-timeout", label: "qmd timeout", unit: "sec" },
  { key: "maxChars", id: "max-chars", label: "New conversation per recording", unit: "chars" },
  { key: "notesBudget", id: "notes-budget", label: "Existing notes per recording", unit: "chars" },
];

function CommitField({
  id,
  label,
  value,
  placeholder,
  unit,
  numeric = false,
  onCommit,
}: {
  id: string;
  label: string;
  value: string;
  placeholder?: string;
  unit?: string;
  numeric?: boolean;
  onCommit: (text: string) => Promise<boolean>;
}) {
  const [text, setText] = useState(value);
  const [editing, setEditing] = useState(false);
  const [saved, setSaved] = useState(false);

  useEffect(() => {
    if (!editing) setText(value);
  }, [value, editing]);

  const commit = async () => {
    if (text !== value) setSaved(await onCommit(text));
    setEditing(false);
  };

  return (
    <div className="set-row">
      <label className="set-main set-title" htmlFor={id}>
        {label}
      </label>
      <div className="set-field">
        {saved && (
          <span className="saved">
            <Check size={12} strokeWidth={2.4} />
            Saved
          </span>
        )}
        <input
          id={id}
          className={`input mono set-input${numeric ? " num" : ""}`}
          type={numeric ? "number" : "text"}
          inputMode={numeric ? "numeric" : undefined}
          min={numeric ? 1 : undefined}
          value={text}
          placeholder={placeholder}
          onFocus={() => setEditing(true)}
          onChange={(event) => {
            setText(event.target.value);
            setSaved(false);
          }}
          onBlur={() => void commit()}
          onKeyDown={(event) => {
            if (event.key === "Enter") event.currentTarget.blur();
          }}
        />
        {unit && <span className="set-unit">{unit}</span>}
      </div>
    </div>
  );
}

function RecordingModel({ snapshot }: { snapshot: Snapshot }) {
  const saveSettings = useAppStore((store) => store.saveSettings);
  const { settings } = snapshot;
  const backend = settings.backend;
  const choices = snapshot.catalog[backend];
  const saved = settings.models[backend];
  const defaultModel = snapshot.defaultModels[backend];
  const listed = findChoice(choices, saved)?.value;
  const [other, setOther] = useState(saved !== null && listed === undefined);
  const efforts = effortsFor(snapshot, backend, other ? saved : (listed ?? null));
  const effort = settings.efforts[backend];
  const defaultEffort = snapshot.defaultEfforts[backend];
  const keptEffort = (model: string | null) => (effort && effortsFor(snapshot, backend, model).includes(effort) ? effort : null);
  const chooseModel = (model: string | null) => saveSettings({ models: { ...settings.models, [backend]: model }, efforts: { ...settings.efforts, [backend]: keptEffort(model) } });

  if (!choices) {
    return (
      <CommitField
        id="model"
        label="Model"
        value={saved ?? ""}
        placeholder={`${defaultModel} (default)`}
        onCommit={(text) => saveSettings({ models: { ...settings.models, [backend]: text } })}
      />
    );
  }

  return (
    <>
      <div className="set-row">
        <label className="set-main set-title" htmlFor="model">
          Model
        </label>
        <select
          id="model"
          className="input set-select"
          value={other ? OTHER : (listed ?? "")}
          onChange={(event) => {
            const value = event.target.value;
            setOther(value === OTHER);
            if (value !== OTHER) void chooseModel(value || null);
          }}
        >
          <option value="">Default · {findChoice(choices, defaultModel)?.label ?? defaultModel}</option>
          {choices.map((choice) => (
            <option key={choice.value} value={choice.value}>
              {choice.label}
            </option>
          ))}
          <option value={OTHER}>Other…</option>
        </select>
      </div>
      {other && <CommitField id="model-name" label="Model name" value={saved ?? ""} placeholder={defaultModel} onCommit={(text) => chooseModel(text.trim() || null)} />}
      {efforts.length > 0 && (
        <div className="set-row">
          <label className="set-main set-title" htmlFor="effort">
            Effort
          </label>
          <select id="effort" className="input set-select" value={effort ?? ""} onChange={(event) => void saveSettings({ efforts: { ...settings.efforts, [backend]: event.target.value || null } })}>
            <option value="">{defaultEffort ? `Default · ${defaultEffort}` : "Default"}</option>
            {efforts.map((level) => (
              <option key={level} value={level}>
                {level}
              </option>
            ))}
          </select>
        </div>
      )}
      {effort && HEAVY_EFFORTS.includes(effort) && <div className="set-hint">Higher effort makes each recording slower and uses more of your allowance. If recordings time out, raise Recording timeout under Advanced.</div>}
    </>
  );
}

export function SettingsView({ snapshot }: { snapshot: Snapshot }) {
  const now = useNow(15_000);
  const saveSettings = useAppStore((store) => store.saveSettings);
  const setAutostart = useAppStore((store) => store.setAutostart);
  const setSyncEnabled = useAppStore((store) => store.setSyncEnabled);
  const syncNow = useAppStore((store) => store.syncNow);
  const [theme, setTheme] = useThemePreference();
  const [advanced, setAdvanced] = useState(false);
  const { settings } = snapshot;
  const backend = settings.backend;
  const sync = syncState(snapshot, now);

  return (
    <div className="body">
      <div className="eyebrow">Recording</div>
      <section className="group" aria-label="Recording">
        <div className="set-row">
          <span className="set-main set-title">Record with</span>
          <Segmented
            label="Recording CLI"
            value={backend}
            options={[
              { value: "codex", label: "Codex" },
              { value: "claude", label: "Claude" },
            ]}
            onChange={(next) => void saveSettings({ backend: next })}
          />
        </div>
        <RecordingModel key={`${backend}:${snapshot.catalog[backend] ? "list" : "text"}`} snapshot={snapshot} />
        <div className="set-hint">Each recording runs {backend === "claude" ? "claude --print" : "codex exec"} and uses your signed-in account’s allowance.</div>
      </section>

      <div className="eyebrow">Memory &amp; sync</div>
      <section className="group" aria-label="Memory and sync">
        <div className="set-row">
          <div className="set-main">
            <div className="set-title">Memory folder</div>
            <div className="set-path mono" title={settings.knowledgeRoot}>
              {settings.knowledgeRoot}
            </div>
          </div>
          <button type="button" className="icon-btn" title="Open memory folder" aria-label="Open memory folder" onClick={() => void api.openFolder("knowledge")}>
            <FolderOpen size={16} strokeWidth={1.9} />
          </button>
          <button type="button" className="pill ghost" onClick={() => void api.openIntegrations("folder")}>
            Change…
          </button>
        </div>
        <div className="set-row">
          <div className="set-main">
            <div className="set-title">Auto-sync</div>
            <div className="set-sub">Every 5 minutes with the memory repository’s upstream.</div>
          </div>
          <Toggle label="Auto-sync" checked={settings.syncEnabled} onChange={(enabled) => void setSyncEnabled(enabled)} />
        </div>
        <div className="set-row">
          <div className="set-main">
            <div className="set-title">Last sync</div>
            <div className="set-sub">{sync.text}</div>
          </div>
          <button type="button" className="pill ghost" disabled={snapshot.syncing} onClick={() => void (sync.opensWindow ? api.openSync() : syncNow())}>
            {sync.opensWindow ? "Open sync" : "Sync now"}
          </button>
        </div>
      </section>

      <div className="eyebrow">Connections</div>
      <section className="group" aria-label="Connections">
        {snapshot.connections.map((connection) => (
          <div key={connection.provider} className="set-row" aria-label={`${CLIENT[connection.provider]} connection`}>
            <div className="set-main">
              <div className="set-title">{CLIENT[connection.provider]}</div>
              <div className="set-sub">Search, guidance, skill and session hooks</div>
            </div>
            <span className={`conn-state ${connection.state}`}>{CONNECTION[connection.state]}</span>
          </div>
        ))}
        <button type="button" className="link-row accent" onClick={() => void api.openIntegrations()}>
          <span className="grow">Manage connections</span>
          <ChevronRight size={14} strokeWidth={1.9} />
        </button>
      </section>

      <div className="eyebrow">General</div>
      <section className="group" aria-label="General">
        <div className="set-row">
          <div className="set-main">
            <div className="set-title">Launch at login</div>
            <div className="set-sub">Start recording from the tray when you sign in.</div>
          </div>
          <Toggle label="Launch at login" checked={snapshot.autostart} onChange={(enabled) => void setAutostart(enabled)} />
        </div>
        <div className="set-row">
          <span className="set-main set-title">Appearance</span>
          <Segmented<ThemePreference>
            label="Appearance"
            value={theme}
            options={[
              { value: "system", label: "System" },
              { value: "light", label: "Light" },
              { value: "dark", label: "Dark" },
            ]}
            onChange={setTheme}
          />
        </div>
      </section>

      <div className="eyebrow">
        Advanced
        <button type="button" className="eyebrow-btn" aria-expanded={advanced} onClick={() => setAdvanced(!advanced)}>
          {advanced ? "Hide" : "Show"}
        </button>
      </div>
      {advanced && (
        <section className="group" aria-label="Advanced">
          {LIMITS.map(({ key, id, label, unit }) => (
            <CommitField key={key} id={id} label={label} unit={unit} numeric value={String(settings[key] || "")} onCommit={(text) => saveSettings({ [key]: Number(text) })} />
          ))}
          <div className="set-hint">Timed-out recordings stop and retry later. Character limits apply from the next recording.</div>
          <button type="button" className="link-row" onClick={() => void api.openFolder("data")}>
            <span className="grow">Open app data folder</span>
            <ChevronRight size={14} strokeWidth={1.9} />
          </button>
          <button type="button" className="link-row" onClick={() => void api.openSetup()}>
            <span className="grow">Run setup guide again</span>
            <ChevronRight size={14} strokeWidth={1.9} />
          </button>
        </section>
      )}
    </div>
  );
}
