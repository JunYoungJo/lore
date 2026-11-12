import { useEffect, useState } from "react";
import { Check, ChevronRight, FolderOpen } from "lucide-react";

import * as api from "@/ipc/api";
import type { Backend, CleanupFolder, ConnectionState, Snapshot } from "@/ipc/contract";
import { effortsFor, findChoice, HEAVY_EFFORTS } from "@/recordingModel";
import { useAppStore } from "@/store/appStore";
import { syncState } from "@/syncState";
import { useThemePreference, type ThemePreference } from "@/theme";
import { Segmented } from "@/ui/Segmented";
import { Toggle } from "@/ui/Toggle";
import { useNow } from "@/useNow";
import { AppUpdates } from "@/components/AppUpdates";

const OTHER = "__other";
const CLIENT = { claude: "Claude Code", codex: "Codex" } as const;
const CLI_NAME: Record<Backend, string> = { claude: "Claude", codex: "Codex" };
const CLI_COMMAND: Record<Backend, string> = { claude: "claude --print", codex: "codex exec" };
const CONNECTION: Record<ConnectionState, string> = { connected: "Connected", partial: "Needs repair", missing: "Not connected" };

type LimitKey = "modelTimeoutSecs" | "qmdTimeoutSecs" | "maxChars" | "notesBudget";

const LIMITS: { key: LimitKey; id: string; label: string; unit: string }[] = [
  { key: "modelTimeoutSecs", id: "model-timeout", label: "Recording timeout", unit: "sec" },
  { key: "qmdTimeoutSecs", id: "qmd-timeout", label: "qmd timeout", unit: "sec" },
  { key: "maxChars", id: "max-chars", label: "New conversation per recording", unit: "chars" },
  { key: "notesBudget", id: "notes-budget", label: "Existing notes per recording", unit: "chars" },
];

interface CommitInputProps {
  id: string;
  value: string;
  placeholder?: string;
  unit?: string;
  numeric?: boolean;
  onCommit: (text: string) => Promise<boolean>;
}

function CommitInput({ id, value, placeholder, unit, numeric = false, onCommit }: CommitInputProps) {
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
  );
}

function CommitField({ label, ...input }: CommitInputProps & { label: string }) {
  return (
    <div className="set-row">
      <label className="set-main set-title" htmlFor={input.id}>
        {label}
      </label>
      <CommitInput {...input} />
    </div>
  );
}

function CliFields({ snapshot, backend }: { snapshot: Snapshot; backend: Backend }) {
  const saveSettings = useAppStore((store) => store.saveSettings);
  const { settings } = snapshot;
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

  const id = (name: string) => `${backend}-${name}`;

  if (!choices) {
    return (
      <div className="cli-fields">
        <div className="cli-field wide">
          <label htmlFor={id("model")}>Model</label>
          <CommitInput id={id("model")} value={saved ?? ""} placeholder={`${defaultModel} (default)`} onCommit={(text) => saveSettings({ models: { ...settings.models, [backend]: text } })} />
        </div>
      </div>
    );
  }

  return (
    <div className="cli-fields">
      <div className="cli-field">
        <label htmlFor={id("model")}>Model</label>
        <select
          id={id("model")}
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
      {efforts.length > 0 && (
        <div className="cli-field">
          <label htmlFor={id("effort")}>Effort</label>
          <select id={id("effort")} className="input set-select" value={effort ?? ""} onChange={(event) => void saveSettings({ efforts: { ...settings.efforts, [backend]: event.target.value || null } })}>
            <option value="">{defaultEffort ? `Default · ${defaultEffort}` : "Default"}</option>
            {efforts.map((level) => (
              <option key={level} value={level}>
                {level}
              </option>
            ))}
          </select>
        </div>
      )}
      {other && (
        <div className="cli-field wide">
          <label htmlFor={id("model-name")}>Model name</label>
          <CommitInput id={id("model-name")} value={saved ?? ""} placeholder={defaultModel} onCommit={(text) => chooseModel(text.trim() || null)} />
        </div>
      )}
      {effort && HEAVY_EFFORTS.includes(effort) && <div className="cli-note">Higher effort makes each recording slower and uses more of your allowance. If recordings time out, raise Recording timeout under Advanced.</div>}
    </div>
  );
}

function Recording({ snapshot }: { snapshot: Snapshot }) {
  const saveSettings = useAppStore((store) => store.saveSettings);
  const { backend: first, fallback } = snapshot.settings;
  const second: Backend = first === "claude" ? "codex" : "claude";

  return (
    <section className="group" aria-label="Recording">
      {[first, second].map((cli) => {
        const isFirst = cli === first;
        const on = isFirst || fallback;
        const found = snapshot.cliAvailable[cli];
        const role = !found
          ? "Not found on this PC."
          : !on
            ? "Not used for recording."
            : !isFirst
              ? `Used when ${CLI_NAME[first]} cannot run, for example at its usage limit.`
              : fallback
                ? "Used first."
                : "Records every session.";
        return (
          <div key={cli} className={`cli${on ? "" : " off"}`} role="group" aria-label={CLI_NAME[cli]}>
            <div className="cli-head">
              <span className={`cli-order${on ? "" : " off"}`} aria-hidden="true">
                {on ? (isFirst ? 1 : 2) : "–"}
              </span>
              <span className="cli-name">{CLI_NAME[cli]}</span>
              {!found && (
                <button type="button" className="pill ghost" onClick={() => void api.openIntegrations("tools")}>
                  Set up…
                </button>
              )}
              {on && !isFirst && (
                <button type="button" className="pill ghost" onClick={() => void saveSettings({ backend: cli })}>
                  Use first
                </button>
              )}
              {isFirst
                ? fallback && <Toggle label={`Record with ${CLI_NAME[cli]}`} checked onChange={() => void saveSettings({ backend: second, fallback: false })} />
                : <Toggle label={`Record with ${CLI_NAME[cli]}`} checked={fallback} disabled={!found && !fallback} onChange={(next) => void saveSettings({ fallback: next })} />}
            </div>
            <div className="cli-role">{role}</div>
            {on && <CliFields key={`${cli}:${snapshot.catalog[cli] ? "list" : "text"}`} snapshot={snapshot} backend={cli} />}
          </div>
        );
      })}
      <div className="set-hint">
        {fallback ? "Lore records with the first CLI that can run. Each uses its own signed-in account’s allowance." : `Each recording runs ${CLI_COMMAND[first]} and uses your signed-in account’s allowance.`}
      </div>
    </section>
  );
}

const COMMON = "common";

function Cleanup({ snapshot }: { snapshot: Snapshot }) {
  const startCleanup = useAppStore((store) => store.startCleanup);
  const [open, setOpen] = useState(false);
  const [folders, setFolders] = useState<CleanupFolder[]>([]);
  const [folder, setFolder] = useState("");
  const [merge, setMerge] = useState("");
  const cleaning = snapshot.cleanup;
  const choosing = open && !cleaning;

  useEffect(() => {
    if (!choosing) return;
    let stale = false;
    api
      .getCleanupFolders()
      .then((list) => !stale && setFolders(list))
      .catch((error) => useAppStore.setState({ error: error instanceof Error ? error.message : String(error) }));
    return () => {
      stale = true;
    };
  }, [choosing]);

  const named = (item: CleanupFolder) => `${item.name} · ${item.notes} ${item.notes === 1 ? "note" : "notes"}`;
  const canMerge = folder !== "" && folder !== COMMON;
  const status = !cleaning
    ? "Merge repeated notes and remove outdated ones, one folder at a time."
    : cleaning.startedAt !== null
      ? `Cleaning up ${cleaning.folder}…`
      : `${cleaning.folder} cleanup starts when ${snapshot.settings.paused ? "recording is resumed" : "the current work is done"}.`;

  return (
    <>
      <div className="set-row">
        <div className="set-main">
          <div className="set-title">Clean up notes</div>
          <div className="set-sub">{status}</div>
        </div>
        <button type="button" className="pill ghost" aria-expanded={choosing} disabled={Boolean(cleaning)} onClick={() => setOpen(!open)}>
          Clean up…
        </button>
      </div>
      {choosing && (
        <div className="cleanup">
          <div className="cli-fields">
            <div className="cli-field">
              <label htmlFor="cleanup-folder">Folder to clean up</label>
              <select
                id="cleanup-folder"
                className="input set-select"
                value={folder}
                onChange={(event) => {
                  setFolder(event.target.value);
                  setMerge("");
                }}
              >
                <option value="">Choose a folder</option>
                {folders.map((item) => (
                  <option key={item.name} value={item.name}>
                    {named(item)}
                  </option>
                ))}
              </select>
            </div>
            {canMerge && (
              <div className="cli-field">
                <label htmlFor="cleanup-merge">Merge another folder into it</label>
                <select id="cleanup-merge" className="input set-select" value={merge} onChange={(event) => setMerge(event.target.value)}>
                  <option value="">None</option>
                  {folders
                    .filter((item) => item.name !== COMMON && item.name !== folder)
                    .map((item) => (
                      <option key={item.name} value={item.name}>
                        {named(item)}
                      </option>
                    ))}
                </select>
              </div>
            )}
          </div>
          <div className="cleanup-foot">
            <span className="cleanup-note">The recording model reads the whole folder. Its changes are applied and committed at once; check them in the review window.</span>
            <button
              type="button"
              className="pill primary"
              disabled={!folder}
              onClick={() => {
                setOpen(false);
                void startCleanup(folder, merge ? [merge] : []);
              }}
            >
              Start cleanup
            </button>
          </div>
        </div>
      )}
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
  const sync = syncState(snapshot, now);

  return (
    <div className="body">
      <AppUpdates snapshot={snapshot} />
      <div className="eyebrow">Recording</div>
      <Recording snapshot={snapshot} />

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
            <div className="set-sub">Share notes after each recording and check other PCs every 5 minutes.</div>
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
        <Cleanup snapshot={snapshot} />
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
