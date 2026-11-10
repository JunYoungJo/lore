import { useCallback, useEffect, useRef, useState } from "react";
import { BookOpen, Check, CheckCircle2, Database, Download, ExternalLink, FolderOpen, Moon, Plug, RefreshCw, Search, Sparkles, Sun, X } from "lucide-react";

import * as api from "@/ipc/api";
import type { PreparedCommand, Provider, SetupClient, SetupCommand, SetupProgress, SetupSnapshot } from "@/ipc/contract";
import { SetupTerminal } from "@/components/SetupTerminal";
import { editable } from "@/store/appStore";
import { useTheme } from "@/theme";
import { BrandMark } from "@/ui/BrandMark";
import "@/styles/setup.css";

const STEPS = [
  { title: "Welcome", label: "A memory that follows your work", icon: Sparkles },
  { title: "Tools", label: "Prepare this computer", icon: Download },
  { title: "Memory folder", label: "Choose where memories live", icon: FolderOpen },
  { title: "AI connections", label: "Connect the tools you use", icon: Plug },
  { title: "Search index", label: "Make your memories searchable", icon: Database },
  { title: "Try it", label: "Remember something. Find it again.", icon: Search },
];
const label = (provider: Provider) => provider === "codex" ? "Codex" : "Claude Code";
const stateLabel = { ready: "Ready", add: "Will add", update: "Will update", preserved: "Existing · kept", conflict: "Needs attention" };
const ready = (client: SetupClient) => client.hooks.state === "connected" && client.plan?.resources.every((r) => r.state === "ready" || r.state === "preserved");
const errorMessage = (error: unknown) => error instanceof Error ? error.message : String(error);

export function IntegrationManager() {
  const [data, setData] = useState<SetupSnapshot | null>(null);
  const [progress, setProgress] = useState<SetupProgress | null>(null);
  const [step, setStep] = useState(0);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [busy, setBusy] = useState<string | null>(null);
  const [root, setRoot] = useState("");
  const [mode, setMode] = useState<"existing" | "create" | "clone">("existing");
  const [remote, setRemote] = useState("");
  const [backend, setBackend] = useState<Provider>("codex");
  const [model, setModel] = useState("");
  const [preview, setPreview] = useState<Provider | null>(null);
  const [command, setCommand] = useState<SetupCommand | null>(null);
  const [prepared, setPrepared] = useState<PreparedCommand | null>(null);
  const [theme, toggleTheme] = useTheme();
  const terminalContainer = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (prepared) terminalContainer.current?.scrollIntoView({ behavior: "smooth", block: "start" });
  }, [prepared]);

  const refresh = useCallback(async () => {
    const next = await api.getSetup();
    setData(next);
    setProgress(next.progress);
    return next;
  }, []);

  useEffect(() => {
    let disposed = false;
    void refresh().then((next) => {
      if (disposed) return;
      setProgress(next.progress);
      setStep(3);
      setRoot(next.settings.knowledgeRoot);
      setBackend(next.settings.backend);
      setModel(next.settings.models[next.settings.backend] ?? "");
    }).catch((e) => !disposed && setError(errorMessage(e)));
    const unlisten = api.onStateChanged(() => { void refresh().catch((e) => setError(errorMessage(e))); });
    return () => { disposed = true; void unlisten.then((stop) => stop()); };
  }, [refresh]);

  const run = async (name: string, action: () => Promise<unknown>) => {
    setError(null); setNotice(null); setBusy(name);
    try { await action(); await refresh(); }
    catch (e) { setError(errorMessage(e)); }
    finally { setBusy(null); }
  };
  const saveProgress = async (patch: Partial<SetupProgress>) => {
    if (!progress) return;
    const next = { ...progress, ...patch };
    await api.saveSetupProgress(next);
    setProgress(next);
  };
  const navigate = async (target: number) => {
    try { setStep(target); setError(null); setNotice(null); }
    catch (e) { setError(errorMessage(e)); }
  };
  const prepare = async (task: SetupCommand) => {
    try {
      const result = await api.setupPrepare(task);
      setCommand(task); setPrepared(result); setError(null);
    } catch (e) { setError(errorMessage(e)); }
  };
  const terminalFinished = useCallback(() => {
    void api.setupDetect().then(refresh).catch((e) => setError(errorMessage(e)));
  }, [refresh]);

  if (!data || !progress) return <main className="setup-loading">{error ?? "Preparing your setup…"}</main>;
  const tool = (name: string) => data.tools.find((t) => t.name === name)?.path;
  const selected = progress.providers;
  const showTerminal = step === 1 || step === 2 || step === 3 || step === 5;

  return (
    <main className="setup-shell">
      <aside className="setup-sidebar">
        <div className="setup-brand"><BrandMark /><span>Lore</span></div>
        <div className="setup-kicker">INTEGRATIONS</div>
        <nav aria-label="Setup steps">
          {STEPS.map(({ title, icon: Icon }, index) => (index < 1 || index > 4 ? null : (
            <button type="button" key={title} aria-current={step === index ? "step" : undefined} disabled={busy !== null} className={`setup-nav-item ${step === index ? "selected" : ""}`} onClick={() => void navigate(index)}>
              <span className="setup-nav-number"><Icon size={14} /></span>
              <Icon size={16} /><span>{title}</span>
            </button>
          )))}
        </nav>
        <div className="setup-sidebar-note"><BookOpen size={18} /><p>Your notes stay in your folder.<br />Your search index stays on this PC.</p></div>
        <button type="button" className="setup-theme" onClick={toggleTheme}>{theme === "dark" ? <Sun size={15} /> : <Moon size={15} />} {theme === "dark" ? "Light appearance" : "Dark appearance"}</button>
      </aside>

      <div className="setup-workspace">
        <header className="setup-topbar"><span>Manage Lore <span className="setup-divider">/</span> {STEPS[step].title}</span><button type="button" className="setup-later" disabled={busy !== null} onClick={() => void api.hideWindow()}>Close <X size={14} /></button></header>
        <div className="setup-scroll">
          <div className="setup-page-head"><span className="setup-eyebrow">MANAGE THIS PC</span><h1>{STEPS[step].label}</h1></div>
          {error && <div className="setup-alert" role="alert">{error}<button type="button" aria-label="Dismiss error" onClick={() => setError(null)}><X size={14} /></button></div>}
          {notice && <div className="setup-notice" role="status">{notice}</div>}

          {step === 1 && <>
            <p className="setup-intro">Lore needs a coding CLI to record memories and qmd to search them. Prepare a command below, review it, and run it in the terminal.</p>
            <section className="setup-card tool-list" aria-label="Required tools">
              {[
                { name: "Node.js 22+", path: tool("node"), task: "install_node", docs: "node", description: "Runtime for qmd. Check the version if Node is already installed." },
                { name: "Git", path: tool("git"), task: "install_git", docs: "git", description: "Create or clone a memory repository and share it across PCs." },
                ...(data.platform === "macos" ? [{ name: "Homebrew SQLite", path: tool("sqlite3"), task: "install_sqlite", docs: "brew", description: "qmd needs Homebrew SQLite on macOS, rather than the system SQLite." }] : []),
                { name: "qmd", path: data.diagnostics.commands.qmd.available ? data.settings.commands.qmd.join(" ") : null, task: "install_qmd", docs: "qmd", description: "Local keyword and semantic search over your memory notes." },
              ].map((t) => <div className="setup-tool" key={t.name}><div className={`setup-tool-state ${t.path ? "found" : ""}`}>{t.path ? <Check size={17} /> : <Download size={17} />}</div><div className="grow"><h3>{t.name}<span className={`setup-badge ${t.path ? "ok" : ""}`}>{t.path ? "Detected" : "Needed"}</span></h3><p>{t.description}</p>{t.path && <code className="setup-path">{t.path}</code>}</div><div className="setup-tool-actions">{t.name.startsWith("Node") && t.path && <button type="button" className="pill ghost" onClick={() => void prepare({ task: "node_version" })}>Check version</button>}{(!t.path || t.name === "qmd") && <button type="button" className="pill primary" disabled={busy !== null} onClick={() => void prepare({ task: t.task })}>{t.path ? "Reinstall" : "Prepare install"}</button>}<button type="button" aria-label={`${t.name} documentation`} className="icon-btn" onClick={() => void api.setupOpenDocs(t.docs).catch((e) => setError(errorMessage(e)))}><ExternalLink size={14} /></button></div></div>)}
            </section>
            {data.platform === "linux" && <p className="help">Install Node.js 22+ and Git using the official links or your distribution’s package manager, then check again.</p>}
            {data.platform === "macos" && !tool("brew") && <p className="help">Homebrew is needed for the prepared macOS commands. <button type="button" className="link-btn" onClick={() => void api.setupOpenDocs("brew")}>Open Homebrew instructions <ExternalLink size={12} /></button></p>}
            <button type="button" className="btn ghost" disabled={busy !== null} onClick={() => void run("detect", api.setupDetect)}><RefreshCw size={14} />Check again</button>
          </>}

          {step === 2 && <>
            <p className="setup-intro">Your memory is a folder of Markdown files. Use a Git remote to share the same notes across computers; each PC builds its own search index.</p>
            <section className="setup-card">
              <div className="setup-options" aria-label="Memory folder mode">{(["existing", "create", "clone"] as const).map((value) => <button type="button" key={value} className={mode === value ? "selected" : ""} onClick={() => setMode(value)}>{value === "existing" ? "Use existing" : value === "create" ? "Create new" : "Clone from Git"}</button>)}</div>
              <label className="field-label" htmlFor="setup-root">Local folder</label><input className="input mono" id="setup-root" value={root} onChange={(e) => setRoot(e.target.value)} />
              {mode === "clone" && <><label className="field-label" htmlFor="setup-remote">Git remote URL</label><input id="setup-remote" className="input mono" placeholder="git@github.com:you/lore-memory.git" value={remote} onChange={(e) => setRemote(e.target.value)} /><p className="help">Clone into an empty or new folder. Your existing Git credentials handle access; you can answer prompts in the terminal.</p><button type="button" className="btn ghost" onClick={() => void prepare({ task: "clone", remote, path: root })}>Prepare clone</button></>}
              <div className="setup-folder-layout"><FolderOpen size={19} /><code>common/ &nbsp; projects/ &nbsp; sources/</code><span>Notes and their supporting conversations</span></div>
              <button type="button" className="btn primary" disabled={busy !== null} onClick={() => void run("repository", async () => { await api.setupRepository(root, mode === "create"); setNotice("Memory folder saved. Recheck AI connections if the folder changed."); })}>{busy === "repository" ? "Preparing folder…" : mode === "create" ? "Create repository & use folder" : "Use this folder"}</button>
              <p className="help">Saved folder: <span className="mono">{data.settings.knowledgeRoot}</span></p>
            </section>
          </>}

          {step === 3 && <>
            <p className="setup-intro">Choose where you work. Lore connects search, memory guidance, a search skill, and session hooks for each selected tool.</p>
            <div className="setup-clients">{data.clients.map((client) => {
              const available = data.diagnostics.commands[client.provider].available;
              return <section className={`setup-card setup-client ${selected.includes(client.provider) ? "chosen" : ""}`} key={client.provider}>
                <label className="setup-client-title"><input type="checkbox" checked={selected.includes(client.provider)} disabled={busy !== null} onChange={(e) => void run("selection", () => saveProgress({ providers: e.target.checked ? [...selected, client.provider] : selected.filter((p) => p !== client.provider) }))} /><h2>{label(client.provider)}</h2><span className={`setup-badge ${ready(client) ? "ok" : ""}`}>{ready(client) ? "Configured" : "Setup needed"}</span></label>
                {!available && <div className="setup-client-install"><p>Install the CLI before connecting.</p><button type="button" className="pill primary" onClick={() => void prepare({ task: `install_${client.provider}` })}>Prepare install</button></div>}
                <div className="setup-integration-status">{client.plan?.resources.map((r) => <span key={r.label}><span className={`mini-dot ${r.state === "ready" || r.state === "preserved" ? "ok" : ""}`} />{r.label}</span>)}<span><span className={`mini-dot ${client.hooks.state === "connected" ? "ok" : ""}`} />Session hooks</span></div>
                {client.error && <p role="alert" className="setup-inline-error">{client.error}</p>}
                {client.hooks.error && <p role="alert" className="setup-inline-error">{client.hooks.error}</p>}
                <div className="setup-client-buttons"><button type="button" className="btn primary" disabled={!available || !selected.includes(client.provider) || !client.plan || busy !== null} onClick={() => setPreview(preview === client.provider ? null : client.provider)}>Review connection</button><button type="button" className="btn ghost" disabled={busy !== null} onClick={() => void prepare({ task: `login_${client.provider}` })}>Sign in</button><button type="button" className="link-btn" disabled={busy !== null} onClick={() => void run("disconnect", async () => { const kept = await api.setupDisconnect(client.provider); setNotice(kept.length ? kept.join("\n") : "Removed Lore-owned entries and hooks. Reused entries are kept."); })}>Disconnect</button></div>
                {preview === client.provider && client.plan && <div className="setup-preview" aria-label={`${label(client.provider)} connection preview`}>
                  {client.plan.resources.map((r) => <div key={r.label} className="setup-resource"><h3>{r.label}<span className={`setup-badge ${r.state === "conflict" ? "danger" : ""}`}>{stateLabel[r.state]}</span></h3><code className="setup-path">{r.path}</code><p>{r.detail}</p>{r.state === "preserved" ? <p>Your existing Lore section will be kept as written.</p> : <details><summary>Content to use</summary><pre>{r.content}</pre></details>}</div>)}
                  <div className="setup-resource"><h3>Session hooks</h3><code className="setup-path">{client.hooks.config}</code><p>Add missing SessionEnd and PreCompact hooks using:</p><pre>{client.hooks.command}</pre><p>Existing non-Lore hooks are kept. Hook configuration is backed up before changes.</p></div>
                  {!client.plan.canApply && <p className="setup-inline-error">An existing qmd entry or skill differs from Lore’s configuration. Your edits are kept. Resolve it in the file above, then check again.</p>}
                  <button type="button" className="btn primary" disabled={!client.plan.canApply || busy !== null} onClick={() => void run("connect", async () => { await api.setupConnect(client.provider, client.plan!.token); setNotice("Connection saved. Restart your AI client, then review its MCP and hook approvals."); setPreview(null); })}>{busy === "connect" ? "Connecting…" : "Apply connection"}</button>
                </div>}
              </section>;
            })}</div>
            <section className="setup-card"><h2>Recording account</h2><p>Memories from either tool are recorded with this CLI and model.</p><div className="setup-model-row"><div><label className="field-label" htmlFor="setup-backend">CLI</label><select id="setup-backend" className="input" value={backend} onChange={(e) => { const p = e.target.value as Provider; setBackend(p); setModel(data.settings.models[p] ?? ""); }}><option value="codex">Codex</option><option value="claude">Claude Code</option></select></div><div className="grow"><label className="field-label" htmlFor="setup-model">Model (optional)</label><input id="setup-model" className="input mono" placeholder="Use the CLI default" value={model} onChange={(e) => setModel(e.target.value)} /></div><button type="button" className="btn ghost" disabled={busy !== null} onClick={() => void run("model", async () => { await api.saveSettings({ ...editable(data.settings), backend, models: { ...data.settings.models, [backend]: model.trim() || null } }); setNotice("Recording model saved."); })}>Save</button></div></section>
            <section className="setup-card"><h2>One approval stays with you</h2><p>Restart your AI client to load the MCP server and skill. In Codex, open <code>/hooks</code> and trust Lore’s hooks. Check <code>/mcp</code> for the <code>qmd</code> connection. Lore does not grant trust on your behalf.</p><button type="button" className="btn ghost" onClick={() => void prepare({ task: "trust_codex" })}>Open Codex in terminal</button><p className="help">Setup terminal sessions are excluded from memory recording. Use a normal project session for the final test.</p></section>
            <button type="button" className="btn ghost" disabled={busy !== null} onClick={() => void run("detect", api.setupDetect)}><RefreshCw size={14} />Check connections again</button>
          </>}

          {step === 4 && <>
            <p className="setup-intro">qmd builds a local index from the memory folder. Git shares your notes; each PC keeps its own model cache and index.</p>
            <section className="setup-card"><div className="setup-index-heading"><div className="setup-feature-icon"><Database size={26} /></div><div><h2>{data.diagnostics.indexConfigured ? "An index is already configured" : "Build your first index"}</h2><p>{data.diagnostics.indexConfigured ? "Refresh collections and add any memories that are missing from this PC’s index." : "New indexes use Qwen3-Embedding-0.6B for multilingual memories, including Korean."}</p></div></div><div className="setup-meta"><span>Memory folder</span><code>{data.settings.knowledgeRoot}</code><span>Local configuration</span><code>{data.diagnostics.qmdConfigDir}</code><span>Local model cache</span><code>{data.diagnostics.qmdCacheDir}</code></div><p className="help">The first run may download about 640 MB for the embedding model and take several minutes. Existing embedding model choices are preserved. A timeout can be increased in Settings, then this step can be retried.</p><button type="button" className="btn primary" disabled={busy !== null || !data.rootExists || !data.diagnostics.commands.qmd.available} onClick={() => void run("index", async () => { await api.setupIndex(); setNotice("Index refresh finished. Continue to test recording and recall."); })}>{busy === "index" ? <><RefreshCw size={14} className="setup-spin" />Downloading models / indexing…</> : data.diagnostics.indexConfigured ? "Refresh local index" : "Build local index"}</button></section>
            <div className="setup-subtle"><CheckCircle2 size={16} />This step does not change the shared app defaults or another PC’s settings.</div>
          </>}

          {/* Keep the terminal mounted while navigating so interactive sessions
              retain their scrollback and input. */}
          <div ref={terminalContainer} hidden={!showTerminal}><SetupTerminal command={command} prepared={prepared} onFinished={terminalFinished} onError={(e) => setError(errorMessage(e))} /></div>
        </div>

        <footer className="setup-footer"><span className="setup-footer-note">{busy === "index" ? "Keep Lore open while the index is being built." : "Changes apply to this PC."}</span><button type="button" className="btn ghost" disabled={busy !== null} onClick={() => void api.openSetup()}>Open introduction</button><button type="button" className="btn primary" disabled={busy !== null} onClick={() => void api.hideWindow()}>Done</button></footer>
      </div>
    </main>
  );
}
