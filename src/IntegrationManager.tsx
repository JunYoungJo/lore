import { useCallback, useEffect, useState, type ReactNode } from "react";
import { Check, ExternalLink, FolderOpen, Minus, RefreshCw, ShieldCheck, X } from "lucide-react";

import * as api from "@/ipc/api";
import type { PreparedCommand, Provider, SetupClient, SetupCommand, SetupResource, SetupSnapshot, ToolChecks } from "@/ipc/contract";
import { SetupTerminal } from "@/components/SetupTerminal";
import { useTheme } from "@/theme";
import { BrandMark } from "@/ui/BrandMark";
import { WindowClose } from "@/ui/WindowClose";
import "@/styles/integrations.css";

type Section = Provider | "folder" | "index" | "tools";
type Tone = "ok" | "warn" | "off";
interface Status { tone: Tone; label: string }
type Run = (name: string, action: () => Promise<unknown>) => Promise<void>;
type Prepare = (command: SetupCommand) => Promise<void>;

const NAMES: Record<Provider, string> = { codex: "Codex", claude: "Claude Code" };
const PROVIDERS: Provider[] = ["codex", "claude"];
const RESOURCE_STATE: Record<SetupResource["state"], string> = { ready: "Ready", preserved: "Kept as written", add: "Will add", update: "Will update", conflict: "Edited outside Lore" };
const errorMessage = (error: unknown) => (error instanceof Error ? error.message : String(error));
const plural = (count: number, word: string) => `${count} ${word}${count === 1 ? "" : "s"}`;

function clientStatus(data: SetupSnapshot, client: SetupClient): Status {
  if (!data.diagnostics.commands[client.provider].available) return { tone: "off", label: "Not installed" };
  const states = client.plan?.resources.map((resource) => resource.state) ?? [];
  if (client.plan && states.every((state) => state === "ready" || state === "preserved") && client.hooks.state === "connected") return { tone: "ok", label: "Connected" };
  if (states.every((state) => state === "add") && client.hooks.state === "missing" && !client.hooks.error) return { tone: "off", label: "Not connected" };
  return { tone: "warn", label: "Needs repair" };
}

interface Tool { key: string; name: string; short: string; path: string | null; found: boolean; state: string; task?: string; docs: string; description: string }

function toolsOf(data: SetupSnapshot, checks: ToolChecks | null): Tool[] {
  const path = (name: string) => data.tools.find((tool) => tool.name === name)?.path ?? null;
  const installable = data.platform !== "linux";
  const node = path("node");
  const nodeReady = node !== null && (checks?.nodeReady ?? true);
  const nodeState = !node ? "Not found" : checks?.nodeVersion ? `${checks.nodeVersion}${nodeReady ? "" : " · too old"}` : "Found";
  const found = (name: string) => (path(name) ? "Found" : "Not found");
  const qmd = data.diagnostics.commands.qmd.available;
  return [
    { key: "node", name: "Node.js 22 or newer", short: "Node.js", path: node, found: nodeReady, state: nodeState, task: installable ? "install_node" : undefined, docs: "node", description: "Runs qmd." },
    { key: "git", name: "Git", short: "Git", path: path("git"), found: path("git") !== null, state: found("git"), task: installable ? "install_git" : undefined, docs: "git", description: "Shares your memory folder between PCs." },
    ...(data.platform === "macos" ? [{ key: "sqlite3", name: "Homebrew SQLite", short: "SQLite", path: path("sqlite3"), found: path("sqlite3") !== null, state: found("sqlite3"), task: "install_sqlite", docs: "brew", description: "qmd needs Homebrew’s SQLite on macOS." }] : []),
    { key: "qmd", name: "qmd", short: "qmd", path: qmd ? data.settings.commands.qmd.join(" ") : null, found: qmd, state: qmd ? "Found" : "Not found", task: "install_qmd", docs: "qmd", description: "Local search over your notes. Installed with npm." },
  ];
}

function toolsStatus(tools: Tool[]): Status {
  const missing = tools.filter((tool) => !tool.found);
  if (missing.length === 0) return { tone: "ok", label: "All found" };
  return { tone: "warn", label: missing.length === 1 ? `${missing[0].short} missing` : `${missing.length} missing` };
}

function indexStatus(data: SetupSnapshot): Status {
  if (!data.diagnostics.commands.qmd.available) return { tone: "warn", label: "Needs qmd" };
  if (!data.diagnostics.indexConfigured) return { tone: "warn", label: "Not built" };
  return { tone: "ok", label: "Ready" };
}

const folderStatus = (data: SetupSnapshot): Status => (data.rootExists ? { tone: "ok", label: "In use" } : { tone: "warn", label: "Folder not found" });

function statuses(data: SetupSnapshot, checks: ToolChecks | null): Record<Section, Status> {
  const client = (provider: Provider) => clientStatus(data, data.clients.find((c) => c.provider === provider)!);
  return { codex: client("codex"), claude: client("claude"), folder: folderStatus(data), index: indexStatus(data), tools: toolsStatus(toolsOf(data, checks)) };
}

const ORDER: Section[] = ["codex", "claude", "folder", "index", "tools"];

function StateIcon({ tone }: { tone: Tone }) {
  return <span className={`int-icon ${tone}`}>{tone === "ok" ? <Check size={11} strokeWidth={3} /> : tone === "warn" ? <span className="int-icon-mark">!</span> : <Minus size={11} strokeWidth={3} />}</span>;
}

function Row({ tone, title, sub, state, action }: { tone: Tone; title: string; sub?: ReactNode; state?: string; action?: ReactNode }) {
  return (
    <div className="int-row">
      <StateIcon tone={tone} />
      <div className="int-row-main">
        <div className="int-row-title">{title}</div>
        {sub && <div className="int-row-sub">{sub}</div>}
      </div>
      {state && <span className={`int-row-state ${tone}`}>{state}</span>}
      {action}
    </div>
  );
}

function Head({ over, title, status, lead }: { over: string; title: string; status: Status; lead: ReactNode }) {
  return (
    <>
      <span className="int-over">{over}</span>
      <div className="int-title">
        <h1>{title}</h1>
        <span className={`int-chip ${status.tone}`}>{status.label}</span>
      </div>
      <p className="int-lead">{lead}</p>
    </>
  );
}

const program = (command: string) => command.replace(/\s+hook\s+\w+\s*$/, "").replace(/^["']|["']$/g, "");
const folderOf = (path: string) => path.replace(/[\\/][^\\/]*$/, "");

function ClientPage({ data, client, status, checks, busy, run, prepare, notify }: { data: SetupSnapshot; client: SetupClient; status: Status; checks: ToolChecks | null; busy: string | null; run: Run; prepare: Prepare; notify: (text: string) => void }) {
  const [review, setReview] = useState(false);
  const name = NAMES[client.provider];
  const plan = client.plan;
  const installed = status.label !== "Not installed";
  const lead = !installed
    ? `Install the ${name} CLI to connect it to your memory.`
    : status.tone === "ok"
      ? `${name} can search your memory, and Lore records its sessions when they end.`
      : status.tone === "off"
        ? `Connect ${name} so it can search your memory and Lore records its sessions when they end.`
        : `Lore gives ${name} a search connection, a short memory guideline, a search skill and session hooks. Some of them need repair.`;
  const hookTone: Tone = client.hooks.state === "connected" ? "ok" : client.hooks.state === "missing" && !client.hooks.error ? "off" : "warn";
  const hookSub = client.hooks.error
    ? client.hooks.error
    : client.hooks.state === "other_path" && client.hooks.found
      ? <>They run <code>{program(client.hooks.found)}</code>, another copy of Lore. This copy runs from <code>{folderOf(data.diagnostics.hookCli)}</code>. Repair points the hooks here.</>
      : <><code>{client.hooks.config}</code> · SessionEnd, PreCompact</>;
  const hookState = client.hooks.error ? "Can’t read" : client.hooks.state === "connected" ? "Ready" : client.hooks.state === "other_path" ? "Another copy" : "Will add";
  const auth = checks?.auth[client.provider];
  const connect = () => run("connect", async () => {
    await api.setupConnect(client.provider, plan!.token);
    setReview(false);
    notify(`Connection saved. Restart ${name} so it loads the skill and the search connection.`);
  });

  return (
    <>
      <Head over="AI tool" title={name} status={status} lead={lead} />
      {!installed && (
        <div className="int-actions">
          <button type="button" className="btn primary" disabled={busy !== null} onClick={() => void prepare({ task: `install_${client.provider}` })}>Install {name}</button>
        </div>
      )}
      {installed && <>
        <div className="int-label">Connection</div>
        <div className="int-group">
          {plan?.resources.map((resource) => (
            <Row
              key={resource.label}
              tone={resource.state === "ready" || resource.state === "preserved" ? "ok" : resource.state === "conflict" ? "warn" : status.tone === "off" ? "off" : "warn"}
              title={resource.label}
              sub={resource.state === "conflict" ? <>This entry differs from Lore’s. Your edits are kept; change <code>{resource.path}</code> yourself, then check again.</> : <code>{resource.path}</code>}
              state={RESOURCE_STATE[resource.state]}
            />
          ))}
          {client.error && <Row tone="warn" title="Connection files" sub={client.error} state="Can’t prepare" />}
          <Row tone={hookTone} title="Session hooks" sub={hookSub} state={hookState} />
        </div>
        {client.provider === "codex" && client.hooks.state === "connected" && (
          <div className="int-trust">
            <ShieldCheck size={18} strokeWidth={1.9} />
            <div>
              <p><b>Codex asks you to trust new hooks.</b> In Codex, open <code>/hooks</code> and trust Lore’s two hooks. <code>/mcp</code> should list <code>qmd</code>.</p>
              <button type="button" className="btn" disabled={busy !== null} onClick={() => void prepare({ task: "trust_codex" })}>Open Codex in terminal</button>
            </div>
          </div>
        )}
        <div className="int-label">Account</div>
        <div className="int-group">
          <Row
            tone={auth === "ready" ? "ok" : auth === undefined ? "off" : "warn"}
            title={`${name} sign-in`}
            sub={`Used when Lore records with ${name}.`}
            state={auth === "ready" ? "Signed in" : auth === "signed_out" ? "Signed out" : auth === undefined ? "Checking…" : "Couldn’t check"}
            action={auth && auth !== "ready" ? <button type="button" className="btn ghost" disabled={busy !== null} onClick={() => void prepare({ task: `login_${client.provider}` })}>Sign in</button> : undefined}
          />
        </div>
        <div className="int-actions">
          {status.tone !== "ok" && <button type="button" className="btn primary" disabled={!plan?.canApply || busy !== null} onClick={() => void connect()}>{busy === "connect" ? "Saving…" : status.tone === "off" ? "Connect" : "Repair connection"}</button>}
          {plan && <button type="button" className="btn ghost" aria-expanded={review} onClick={() => setReview(!review)}>{status.tone === "ok" ? "Review files" : "Review changes"}</button>}
          <span className="grow" />
          {status.tone !== "off" && <button type="button" className="link-btn int-danger" disabled={busy !== null} onClick={() => void run("disconnect", async () => {
            const kept = await api.setupDisconnect(client.provider);
            notify(kept.length ? kept.join("\n") : "Removed Lore’s entries and hooks. Entries you had before are kept.");
          })}>Disconnect</button>}
        </div>
        {plan && !plan.canApply && <p className="int-note warn">An existing qmd entry or skill differs from Lore’s. Your edits are kept. Change it in the file shown above, then check again.</p>}
        {review && plan && (
          <div className="int-review" aria-label={`${name} connection files`}>
            {plan.resources.map((resource) => (
              <div key={resource.label} className="int-review-item">
                <h3>{resource.label}<span className={`int-badge ${resource.state === "conflict" ? "warn" : ""}`}>{RESOURCE_STATE[resource.state]}</span></h3>
                <code className="int-path">{resource.path}</code>
                <p>{resource.detail}</p>
                {resource.state === "preserved" ? <p>Your existing Lore section will be kept as written.</p> : <details><summary>Content Lore writes</summary><pre>{resource.content}</pre></details>}
              </div>
            ))}
            <div className="int-review-item">
              <h3>Session hooks<span className="int-badge">{hookState}</span></h3>
              <code className="int-path">{client.hooks.config}</code>
              <p>SessionEnd and PreCompact run:</p>
              <pre>{client.hooks.command}</pre>
              <p>Other hooks are kept. The hook settings file is backed up before changes.</p>
            </div>
          </div>
        )}
      </>}
    </>
  );
}

function FolderPage({ data, status, busy, run, prepare, notify, startChanging }: { data: SetupSnapshot; status: Status; busy: string | null; run: Run; prepare: Prepare; notify: (text: string) => void; startChanging: boolean }) {
  const [changing, setChanging] = useState(startChanging);
  const [mode, setMode] = useState<"existing" | "create" | "clone">("existing");
  const [root, setRoot] = useState(data.settings.knowledgeRoot);
  const [remote, setRemote] = useState("");
  const repository = data.repository ? `Git repository · ${data.tracking ?? "no upstream"}` : "Not a Git repository";
  const options = [
    { value: "existing" as const, title: "A folder I have", text: "An existing memory folder on this PC." },
    { value: "create" as const, title: "Start a new one", text: "An empty folder. Lore creates a Git repository." },
    { value: "clone" as const, title: "Copy from Git", text: "Clone your memory from another PC’s remote." },
  ];

  return (
    <>
      <Head over="This PC" title="Memory folder" status={status} lead="Your notes are Markdown files in this folder. Git shares them between your PCs." />
      <div className="int-folder">
        <span className="int-folder-icon"><FolderOpen size={20} strokeWidth={1.9} /></span>
        <div className="int-folder-main">
          <div className="int-folder-path mono">{data.settings.knowledgeRoot}</div>
          <div className="int-folder-sub">{repository}</div>
        </div>
        <button type="button" className="btn ghost" disabled={!data.rootExists} onClick={() => void api.openFolder("knowledge")}>Open</button>
        <button type="button" className={`btn ${changing ? "on" : ""}`} aria-expanded={changing} onClick={() => setChanging(!changing)}>Change…</button>
      </div>
      {changing && (
        <section className="int-change" aria-label="Use a different folder">
          <h2>Use a different folder</h2>
          <div className="int-opts">
            {options.map((option) => (
              <button key={option.value} type="button" className={`int-opt ${mode === option.value ? "on" : ""}`} aria-pressed={mode === option.value} onClick={() => setMode(option.value)}>
                <strong>{option.title}</strong>
                <small>{option.text}</small>
              </button>
            ))}
          </div>
          <label className="field-label" htmlFor="int-root">Folder</label>
          <div className="int-input-row">
            <input id="int-root" className="input mono" value={root} onChange={(event) => setRoot(event.target.value)} />
            <button type="button" className="btn ghost" onClick={() => void api.setupPickFolder().then((picked) => picked && setRoot(picked))}>Browse…</button>
          </div>
          {mode === "clone" && <>
            <label className="field-label" htmlFor="int-remote">Git remote URL</label>
            <div className="int-input-row">
              <input id="int-remote" className="input mono" placeholder="git@github.com:you/lore-memory.git" value={remote} onChange={(event) => setRemote(event.target.value)} />
              <button type="button" className="btn ghost" disabled={busy !== null} onClick={() => void prepare({ task: "clone", remote, path: root })}>Prepare clone</button>
            </div>
            <p className="int-note">Clone into an empty or new folder, then choose Use this folder. Your Git credentials handle access.</p>
          </>}
          <p className="int-note">After the change, build the search index for that folder and repair the AI connections; the list on the left shows what is left. The current folder is not changed.</p>
          <div className="int-actions">
            <button type="button" className="btn primary" disabled={busy !== null || !root.trim()} onClick={() => void run("repository", async () => {
              await api.setupRepository(root, mode === "create");
              setChanging(false);
              notify("Memory folder changed. Build the search index and repair the AI connections next.");
            })}>{busy === "repository" ? "Preparing folder…" : mode === "create" ? "Create & use this folder" : "Use this folder"}</button>
            <button type="button" className="btn ghost" onClick={() => setChanging(false)}>Cancel</button>
          </div>
        </section>
      )}
    </>
  );
}

function IndexPage({ data, status, busy, run, notify, show }: { data: SetupSnapshot; status: Status; busy: string | null; run: Run; notify: (text: string) => void; show: (section: Section) => void }) {
  const [changing, setChanging] = useState(false);
  const current = data.embedModels.find((model) => model.uri === data.embedModel);
  const [chosen, setChosen] = useState(data.embedModel ?? data.embedModels[0].uri);
  const target = data.embedModels.find((model) => model.uri === chosen) ?? data.embedModels[0];
  const projects = data.collections.filter((name) => name.startsWith("project-")).length;
  const collections = data.collections.length === 0 ? "None yet" : `${data.collections.includes("common") ? "common and " : ""}${plural(projects, "project")}`;
  const qmd = data.diagnostics.commands.qmd.available;
  const built = data.diagnostics.indexConfigured;
  const working = busy === "index" || busy === "embed";

  return (
    <>
      <Head over="This PC" title="Search index" status={status} lead="qmd searches your notes through an index kept on this PC. Each PC builds its own; Git does not share it." />
      {!qmd && (
        <div className="int-actions">
          <button type="button" className="btn primary" onClick={() => show("tools")}>Install qmd in Tools</button>
        </div>
      )}
      {qmd && built && (
        <div className="int-group">
          <div className="int-row">
            <span className="int-row-label">Embedding model</span>
            <div className="int-row-main">
              <div className="int-row-title">{current?.name ?? data.embedModel?.split("/").pop() ?? "qmd default"}</div>
              {current && <div className="int-row-sub">{current.note}</div>}
            </div>
            <button type="button" className={`btn ghost ${changing ? "on" : ""}`} aria-expanded={changing} disabled={busy !== null} onClick={() => setChanging(!changing)}>Change…</button>
          </div>
          <div className="int-row">
            <span className="int-row-label">Collections</span>
            <div className="int-row-main"><div className="int-row-title">{collections}</div></div>
          </div>
          <div className="int-row">
            <span className="int-row-label">Stored on this PC</span>
            <div className="int-row-main">
              <code className="int-path">{data.diagnostics.qmdConfigDir}</code>
              <code className="int-path">{data.diagnostics.qmdCacheDir}</code>
            </div>
          </div>
        </div>
      )}
      {qmd && built && changing && (
        <section className="int-change" aria-label="Change the embedding model">
          <h2>Change the embedding model</h2>
          <div className="int-opts two">
            {data.embedModels.map((model) => (
              <button key={model.uri} type="button" className={`int-opt ${chosen === model.uri ? "on" : ""}`} aria-pressed={chosen === model.uri} onClick={() => setChosen(model.uri)}>
                <strong>{model.name}{model.uri === data.embedModel && <span className="int-badge ok">In use</span>}</strong>
                <small>{model.note} Download {model.size}.</small>
              </button>
            ))}
          </div>
          <p className="int-note warn">Changing the model rebuilds the whole index on this PC. qmd downloads {target.name} ({target.size}) if it is not on this PC yet. Until the rebuild finishes, semantic search returns nothing; keyword search keeps working. Restart open Claude Code and Codex sessions afterwards.</p>
          <div className="int-actions">
            <button type="button" className="btn primary" disabled={busy !== null || chosen === data.embedModel} onClick={() => void run("embed", async () => {
              await api.setupEmbedModel(chosen);
              setChanging(false);
              notify(`Index rebuilt with ${target.name}. Restart open Claude Code and Codex sessions so they search with it.`);
            })}>{busy === "embed" ? <><RefreshCw size={14} className="int-spin" />Rebuilding the index…</> : `Rebuild with ${target.name}`}</button>
            <button type="button" className="btn ghost" disabled={busy === "embed"} onClick={() => setChanging(false)}>Cancel</button>
          </div>
        </section>
      )}
      {qmd && (
        <div className="int-actions">
          <button type="button" className={`btn ${built ? "" : "primary"}`} disabled={busy !== null || !data.rootExists} onClick={() => void run("index", async () => {
            await api.setupIndex();
            notify(built ? "Index refreshed." : "Index built. AI tools can search your notes now.");
          })}>{busy === "index" ? <><RefreshCw size={14} className="int-spin" />{built ? "Refreshing…" : "Building…"}</> : built ? "Refresh index" : "Build index"}</button>
          <p className="int-hint">{working ? "Keep Lore open until this finishes." : built ? "Lore refreshes the index after each recording. Use this when search misses notes you copied in by hand." : `The first build downloads ${data.embedModels[0].name} (${data.embedModels[0].size}) and can take several minutes.`}</p>
        </div>
      )}
    </>
  );
}

function ToolsPage({ data, checks, status, busy, run, prepare, recheck }: { data: SetupSnapshot; checks: ToolChecks | null; status: Status; busy: string | null; run: Run; prepare: Prepare; recheck: () => Promise<void> }) {
  return (
    <>
      <Head over="This PC" title="Tools" status={status} lead="Lore uses these programs on this PC. Ones you already have are reused." />
      <div className="int-group">
        {toolsOf(data, checks).map((tool) => (
          <Row
            key={tool.key}
            tone={tool.found ? "ok" : "warn"}
            title={tool.name}
            sub={tool.path ? <code>{tool.path}</code> : tool.description}
            state={tool.found ? tool.state : tool.path ? tool.state : undefined}
            action={tool.found ? undefined : tool.task
              ? <button type="button" className="btn primary" disabled={busy !== null} onClick={() => void prepare({ task: tool.task! })}>Install</button>
              : <button type="button" className="btn ghost" onClick={() => void api.setupOpenDocs(tool.docs)}>Download <ExternalLink size={12} /></button>}
          />
        ))}
      </div>
      <div className="int-actions">
        <button type="button" className="btn ghost" disabled={busy !== null} onClick={() => void run("detect", async () => { await api.setupDetect(); await recheck(); })}><RefreshCw size={14} />Check again</button>
      </div>
    </>
  );
}

export function IntegrationManager() {
  useTheme();
  const [data, setData] = useState<SetupSnapshot | null>(null);
  const [checks, setChecks] = useState<ToolChecks | null>(null);
  const [section, setSection] = useState<Section | null>(null);
  const [askedFolder, setAskedFolder] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [busy, setBusy] = useState<string | null>(null);
  const [command, setCommand] = useState<SetupCommand | null>(null);
  const [prepared, setPrepared] = useState<PreparedCommand | null>(null);

  const refresh = useCallback(async () => {
    const next = await api.getSetup();
    setData(next);
    return next;
  }, []);
  const recheck = useCallback(async () => {
    setChecks(await api.setupCheckTools().catch(() => null));
  }, []);

  useEffect(() => {
    let disposed = false;
    const wanted = new URLSearchParams(window.location.search).get("section");
    const folder = wanted === "folder";
    setAskedFolder(folder);
    void refresh()
      .then((next) => {
        if (disposed) return;
        const all = statuses(next, null);
        setSection(folder ? "folder" : wanted === "tools" ? "tools" : (ORDER.find((key) => all[key].tone === "warn") ?? "codex"));
      })
      .catch((e) => !disposed && setError(errorMessage(e)));
    void recheck();
    const unlisten = api.onStateChanged(() => void refresh().catch((e) => setError(errorMessage(e))));
    return () => {
      disposed = true;
      void unlisten.then((stop) => stop());
    };
  }, [refresh, recheck]);

  useEffect(() => {
    const stop = api.onIntegrationsSection((wanted) => {
      if (wanted === "folder") {
        setAskedFolder(true);
        setSection("folder");
      } else if (wanted === "tools") {
        setSection("tools");
      }
    });
    return () => void stop.then((unlisten) => unlisten());
  }, []);

  const run: Run = async (name, action) => {
    setError(null);
    setNotice(null);
    setBusy(name);
    try {
      await action();
      await refresh();
    } catch (e) {
      setError(errorMessage(e));
    } finally {
      setBusy(null);
    }
  };
  const prepare: Prepare = async (task) => {
    try {
      setPrepared(await api.setupPrepare(task));
      setCommand(task);
      setError(null);
    } catch (e) {
      setError(errorMessage(e));
    }
  };
  const terminalFinished = useCallback(() => {
    void api.setupDetect().then(refresh).then(recheck).catch((e) => setError(errorMessage(e)));
  }, [refresh, recheck]);

  if (!data || !section) return <main className="int-loading">{error ?? "Loading…"}</main>;
  const all = statuses(data, checks);
  const attention = ORDER.filter((key) => all[key].tone === "warn").length;
  const show = (next: Section) => {
    setSection(next);
    setAskedFolder(false);
    setNotice(null);
  };
  const item = (key: Section, title: string, sub: string) => (
    <button key={key} type="button" className={`int-item ${section === key ? "on" : ""}`} aria-current={section === key ? "page" : undefined} onClick={() => show(key)}>
      <span className={`int-dot ${all[key].tone}`} />
      <span className="int-item-main">
        <span className="int-item-title">{title}</span>
        <span className={`int-item-sub ${all[key].tone}`}>{sub}</span>
      </span>
    </button>
  );
  const client = (provider: Provider) => data.clients.find((c) => c.provider === provider)!;

  return (
    <main className="int-window">
      <header className="int-bar" data-tauri-drag-region>
        <span className="int-brand" data-tauri-drag-region><BrandMark size={22} /></span>
        <span className="int-bar-title" data-tauri-drag-region>Manage Lore</span>
        <span className={`int-chip ${attention ? "warn" : "ok"}`} data-tauri-drag-region>{attention ? `${attention} ${attention === 1 ? "needs" : "need"} attention` : "All set"}</span>
        <span className="grow" data-tauri-drag-region />
        <WindowClose disabled={busy !== null} onClose={() => void api.hideWindow()} />
      </header>
      <div className="int-main">
        <aside className="int-rail" aria-label="Manage">
          <div className="int-rail-label">AI tools</div>
          {PROVIDERS.map((provider) => item(provider, NAMES[provider], all[provider].label))}
          <div className="int-rail-label">This PC</div>
          {item("folder", "Memory folder", data.rootExists ? data.settings.knowledgeRoot : all.folder.label)}
          {item("index", "Search index", all.index.label)}
          {item("tools", "Tools", all.tools.tone === "ok" ? toolsOf(data, checks).map((tool) => tool.short).join(", ").replace(/, ([^,]*)$/, " and $1") + " found" : all.tools.label)}
          <div className="int-rail-foot">
            <button type="button" className="link-btn" onClick={() => void api.openSetup()}>Run setup guide again</button>
          </div>
        </aside>
        <div className="int-right">
          <section className="int-pane" aria-label={section === "folder" ? "Memory folder" : section === "index" ? "Search index" : section === "tools" ? "Tools" : NAMES[section]}>
            {error && <div className="int-alert" role="alert">{error}<button type="button" aria-label="Dismiss error" onClick={() => setError(null)}><X size={14} /></button></div>}
            {notice && <div className="int-notice" role="status">{notice}</div>}
            {(section === "codex" || section === "claude") && <ClientPage key={section} data={data} client={client(section)} status={all[section]} checks={checks} busy={busy} run={run} prepare={prepare} notify={setNotice} />}
            {section === "folder" && <FolderPage key={askedFolder ? "asked" : "folder"} data={data} status={all.folder} busy={busy} run={run} prepare={prepare} notify={setNotice} startChanging={askedFolder} />}
            {section === "index" && <IndexPage data={data} status={all.index} busy={busy} run={run} notify={setNotice} show={show} />}
            {section === "tools" && <ToolsPage data={data} checks={checks} status={all.tools} busy={busy} run={run} prepare={prepare} recheck={recheck} />}
          </section>
          <div className="int-drawer" hidden={!prepared}>
            <SetupTerminal command={command} prepared={prepared} onFinished={terminalFinished} onError={(e) => setError(errorMessage(e))} onClose={() => setPrepared(null)} />
          </div>
        </div>
      </div>
    </main>
  );
}
