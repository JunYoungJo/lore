import { useCallback, useEffect, useRef, useState } from "react";
import { ArrowLeft, ArrowRight, BookOpen, Check, CheckCircle2, ChevronDown, Download, ExternalLink, FolderOpen, GitBranch, LoaderCircle, MessageSquare, Search, ShieldCheck, X } from "lucide-react";

import * as api from "@/ipc/api";
import type { PreparedCommand, Provider, SetupCommand, SetupProgress, SetupSnapshot, ToolChecks } from "@/ipc/contract";
import { SetupTerminal } from "@/components/SetupTerminal";
import { editable } from "@/store/appStore";
import { BrandMark } from "@/ui/BrandMark";
import { useTheme } from "@/theme";
import "@/styles/onboarding.css";

const NAMES: Record<Provider, string> = { codex: "Codex", claude: "Claude Code" };
const STAGES = ["Meet Lore", "Your tools", "Your memory", "Get connected", "Start remembering"];
const message = (e: unknown) => e instanceof Error ? e.message : String(e);

type Need = { title: string; description: string; task?: string; docs?: string; action?: "resume" };

export function nextSetupNeed(data: SetupSnapshot, checks: ToolChecks, providers: Provider[]): Need | null {
  const tool = (name: string) => data.tools.some((t) => t.name === name && t.path);
  const mac = data.platform === "macos";
  if (mac && !tool("brew") && (!checks.nodeReady || !tool("sqlite3"))) return { title: "One prerequisite on your Mac", description: "Local search needs Homebrew’s SQLite. Install Homebrew using its official instructions, then return here. We’ll check again.", docs: "brew" };
  if (!checks.nodeReady || !tool("npm")) return { title: "Prepare the search runtime", description: `qmd needs Node.js 22 or newer. ${checks.nodeVersion ? `This PC has ${checks.nodeVersion}.` : "We couldn’t find a supported runtime yet."}`, task: data.platform === "linux" ? undefined : "install_node", docs: "node" };
  if (!tool("git")) return { title: "Let’s add Git", description: "Lore uses Git to recognize your projects and to share memory between computers.", task: data.platform === "linux" ? undefined : "install_git", docs: "git" };
  if (mac && !tool("sqlite3")) return { title: "Prepare local search on your Mac", description: "qmd uses Homebrew SQLite to keep your search index on this computer.", task: "install_sqlite", docs: "qmd" };
  if (!data.diagnostics.commands.qmd.available) return { title: "Let’s add local memory search", description: "qmd finds the right notes for your AI. It runs on this PC; your Markdown files stay in your memory folder.", task: "install_qmd", docs: "qmd" };
  for (const provider of providers) {
    if (!data.diagnostics.commands[provider].available) return { title: `Install ${NAMES[provider]}`, description: `Lore will use your ${NAMES[provider]} installation to work with your coding sessions. Review the command before running it.`, task: `install_${provider}` };
    if (checks.auth[provider] !== "ready") return { title: `Sign in to ${NAMES[provider]}`, description: checks.auth[provider] === "unknown" ? "We couldn’t confirm the sign-in. Open the sign-in flow, or check again if you’re already signed in." : "Use the account you already use for coding. The terminal will guide you through the browser sign-in.", task: `login_${provider}` };
  }
  if (data.settings.paused) return { title: "Recording is paused", description: "Resume Lore when you’re ready for new sessions to become memories. Your existing pause preference has been kept.", action: "resume" };
  return null;
}

export function MemoryStory({ compact = false }: { compact?: boolean }) {
  const [scene, setScene] = useState(0);
  const scenes = [
    { tag: "DURING YOUR WORK", title: "A decision worth keeping", text: "“Let’s start with email sign-in. It keeps the first release simple.”", foot: "You work with your AI as usual.", icon: MessageSquare },
    { tag: "AFTER THE SESSION", title: "The reason stays with the decision", text: "Decision → email sign-in first\nReason → keep the first release simple\nSource → your project conversation", foot: "Lore keeps a note you can read and check.", icon: BookOpen },
    { tag: "IN A NEW SESSION", title: "Pick up where you left off", text: "“We chose email sign-in to keep the first release simple. That decision is in our project notes.”", foot: "Your AI finds the note and reads the evidence.", icon: Search },
  ];
  const current = scenes[scene];
  const Icon = current.icon;
  return <section className={`memory-story ${compact ? "compact" : ""}`} aria-label="How Lore memory works">
    <div className="story-orbit"><span /><span /><div className="story-symbol"><Icon size={26} strokeWidth={1.4} /></div></div>
    <div className="story-body" key={scene}><span className="story-tag">{current.tag}</span><h3>{current.title}</h3><div className="story-note">{current.text}</div><p>{current.foot}</p></div>
    <div className="story-controls"><span className="story-example">Illustrative example</span><div className="story-dots">{scenes.map((s, i) => <button key={s.tag} type="button" aria-label={`Example ${i + 1}: ${s.title}`} aria-pressed={scene === i} onClick={() => setScene(i)} />)}</div><button type="button" className="story-next" aria-label="Next example" onClick={() => setScene((s) => (s + 1) % 3)}><ArrowRight size={16} /></button></div>
  </section>;
}

export function SetupApp() {
  const [data, setData] = useState<SetupSnapshot | null>(null);
  const [progress, setProgress] = useState<SetupProgress | null>(null);
  const [step, setStep] = useState(0);
  const [selected, setSelected] = useState<Provider[]>([]);
  const [root, setRoot] = useState("");
  const [mode, setMode] = useState<"create" | "existing" | "clone">("create");
  const [remote, setRemote] = useState("");
  const [checks, setChecks] = useState<ToolChecks | null>(null);
  const [checking, setChecking] = useState(false);
  const [busy, setBusy] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [command, setCommand] = useState<SetupCommand | null>(null);
  const [prepared, setPrepared] = useState<PreparedCommand | null>(null);
  const [showTerminal, setShowTerminal] = useState(false);
  const [terminalActive, setTerminalActive] = useState(false);
  const [details, setDetails] = useState(false);
  const terminalContainer = useRef<HTMLDivElement>(null);
  const scrollContainer = useRef<HTMLDivElement>(null);
  const initialized = useRef(false);
  useTheme();

  const refresh = useCallback(async () => {
    const next = await api.getSetup();
    setData(next);
    setProgress(next.progress);
    if (!initialized.current) {
      initialized.current = true;
      setStep(Math.min(next.progress.step, 4));
      setSelected(next.progress.providers.length ? next.progress.providers : (["codex", "claude"] as const).filter((p) => next.diagnostics.commands[p].available));
      setRoot(next.settings.knowledgeRoot);
      setMode(next.rootExists ? "existing" : "create");
    }
    return next;
  }, []);

  const checkTools = useCallback(async () => {
    setChecking(true);
    try { await api.setupDetect(); await refresh(); setChecks(await api.setupCheckTools()); }
    catch (e) { setError(message(e)); }
    finally { setChecking(false); }
  }, [refresh]);

  useEffect(() => {
    void refresh().catch((e) => setError(message(e)));
    const unlisten = api.onStateChanged(() => { void refresh().catch((e) => setError(message(e))); });
    return () => { void unlisten.then((stop) => stop()); };
  }, [refresh]);
  useEffect(() => { if (step === 3) void checkTools(); }, [step, checkTools]);
  useEffect(() => { if (scrollContainer.current) scrollContainer.current.scrollTop = 0; }, [step]);
  useEffect(() => { if (showTerminal) terminalContainer.current?.scrollIntoView?.({ behavior: "smooth", block: "nearest" }); }, [showTerminal, prepared]);

  const perform = async (name: string, action: () => Promise<unknown>) => {
    setBusy(name); setError(null);
    try { await action(); await refresh(); }
    catch (e) { setError(message(e)); }
    finally { setBusy(null); }
  };
  const move = async (target: number) => {
    if (!progress) return;
    const next = { ...progress, providers: selected, step: target, deferred: false };
    await api.saveSetupProgress(next);
    setProgress(next); setStep(target); setError(null); setShowTerminal(false); setDetails(false);
  };
  const prepare = async (task: SetupCommand) => {
    try { setError(null); const next = await api.setupPrepare(task); setCommand(task); setPrepared(next); setShowTerminal(true); }
    catch (e) { setError(message(e)); }
  };
  const later = () => perform("later", async () => {
    if (progress) await api.saveSetupProgress({ ...progress, step, providers: selected, deferred: true });
    await api.hideWindow();
  });

  if (!data || !progress) return <main className="onboard-loading"><BrandMark /><p>{error ?? "Getting Lore ready…"}</p>{error && <button className="btn ghost" type="button" onClick={() => void refresh().catch((e) => setError(message(e)))}>Try again</button>}</main>;

  const busyNow = busy !== null || terminalActive;
  const gitReady = data.tools.some((t) => t.name === "git" && t.path);
  const need = checks ? nextSetupNeed(data, checks, selected) : null;
  const clients = data.clients.filter((c) => selected.includes(c.provider));
  const connectionConflict = clients.find((c) => c.error || c.hooks.error || !c.plan?.canApply);
  const configured = selected.length > 0 && data.rootExists && data.diagnostics.indexConfigured && clients.every((c) => c.hooks.state === "connected" && c.plan?.resources.every((r) => ["ready", "preserved"].includes(r.state)));
  const choose = (provider: Provider) => setSelected((old) => old.includes(provider) ? old.filter((p) => p !== provider) : [...old, provider]);
  const prepareMemory = () => perform("Preparing local search…", async () => {
    const backend = selected.includes(data.settings.backend) ? data.settings.backend : selected[0];
    await api.saveSettings({ ...editable(data.settings), backend });
    await api.setupIndex();
    setBusy("Connecting your AI tools…");
    const next = await api.getSetup();
    for (const provider of selected) {
      const client = next.clients.find((c) => c.provider === provider);
      if (!client?.plan?.canApply) throw new Error(client?.error ?? "Review the existing integration conflict before connecting.");
      await api.setupConnect(provider, client.plan.token);
    }
    await api.saveSetupProgress({ ...progress, providers: selected, step: 4, completed: true, deferred: false });
    setStep(4); setShowTerminal(false);
  });

  return <main className="onboard">
    <header className="onboard-top" data-tauri-drag-region><div className="onboard-logo" data-tauri-drag-region><BrandMark /><span>Lore</span></div><button type="button" className="onboard-dismiss" disabled={busy !== null} onClick={() => void later()}>Set up later <X size={15} /></button></header>
    <div className="onboard-scroll" ref={scrollContainer}>
      <div className={`onboard-content ${step === 0 ? "onboard-welcome" : ""}`} key={step}>
        {error && <div className="onboard-error" role="alert">{error}<button type="button" aria-label="Dismiss error" onClick={() => setError(null)}><X size={14} /></button></div>}

        {step === 0 && <>
          <div className="onboard-heading"><span className="onboard-overline">A LITTLE CONTEXT GOES A LONG WAY</span><h1>Your next session<br />doesn’t start from zero.</h1><p>Keep the decisions, the reasons, and the lessons.<br />Give your AI something worth remembering.</p></div>
          <MemoryStory />
          <div className="onboard-main-action"><button type="button" className="onboard-primary" disabled={busyNow} onClick={() => void perform("next", () => move(1))}>Get started <ArrowRight size={17} /></button><span>A few choices. We’ll handle the connections.</span></div>
          <button type="button" className="onboard-text-button" onClick={() => void api.openIntegrations().catch((e) => setError(message(e)))}>Already set up? Manage your connections</button>
        </>}

        {step === 1 && <>
          <div className="onboard-heading"><span className="onboard-overline">FAMILIAR TOOLS. A BETTER MEMORY.</span><h1>Where do you work?</h1><p>Lore can remember sessions from either tool.<br />Choose the ones you want to connect.</p></div>
          <div className="onboard-provider-options">{(["codex", "claude"] as const).map((provider) => <button type="button" className={`onboard-provider ${selected.includes(provider) ? "chosen" : ""}`} key={provider} aria-pressed={selected.includes(provider)} onClick={() => choose(provider)} disabled={busyNow}><span className={`provider-art ${provider}`}><span className="provider-prompt">{provider === "claude" ? "✳" : ">_"}</span><span className="provider-code"><i /><i /><i /></span></span><span className="provider-caption"><strong>{NAMES[provider]}</strong><span className="provider-select">{selected.includes(provider) && <Check size={13} />}</span></span><span className="provider-detected">{data.diagnostics.commands[provider].available ? "Found on this PC" : "We’ll help you install it"}</span></button>)}</div>
          <p className="onboard-small">Your existing account and subscriptions stay with your coding tool. Lore uses one connected CLI to record memories from either tool.</p>
          <div className="onboard-main-action"><button type="button" className="onboard-primary" disabled={!selected.length || busyNow} onClick={() => void perform("next", () => move(2))}>Continue <ArrowRight size={17} /></button></div>
        </>}

        {step === 2 && <>
          <div className="onboard-heading"><span className="onboard-overline">YOUR WORDS. YOUR FILES.</span><h1>A home for your memory.</h1><p>Readable Markdown in a folder you choose.<br />Bring an existing memory, or start fresh.</p></div>
          <div className="onboard-folder-scene"><FolderOpen size={42} strokeWidth={1.15} /><div><strong>lore-memory</strong><span>Decisions &nbsp;·&nbsp; Lessons &nbsp;·&nbsp; Sources</span></div><span className="folder-md">.md</span></div>
          <div className="onboard-folder-modes" aria-label="Memory folder mode">{(["create", "existing", "clone"] as const).map((value) => <button type="button" key={value} aria-pressed={mode === value} onClick={() => { setMode(value); setShowTerminal(false); }}>{value === "create" ? "Start fresh" : value === "existing" ? "Use my folder" : "Bring from Git"}</button>)}</div>
          <div className="onboard-folder-fields"><label htmlFor="onboard-root">{mode === "clone" ? "Local destination" : "Memory folder"}</label><div className="onboard-folder-input"><input id="onboard-root" className="mono" value={root} onChange={(e) => setRoot(e.target.value)} /><button type="button" title="Choose folder" aria-label="Choose folder" onClick={() => void perform("folder", async () => { const picked = await api.setupPickFolder(); if (picked) setRoot(picked); })}><FolderOpen size={16} /></button></div>{mode === "clone" && <><label htmlFor="onboard-remote">Git remote</label><input id="onboard-remote" className="input mono" placeholder="git@github.com:you/lore-memory.git" value={remote} onChange={(e) => setRemote(e.target.value)} /><button type="button" className="onboard-secondary" disabled={!remote.trim() || busyNow} onClick={() => void prepare({ task: "clone", remote, path: root })}><GitBranch size={15} />Clone this memory</button><p className="onboard-small">Clone into an empty or new folder. Then select Use this folder to continue. Your usual Git credentials handle access.</p></>}</div>
          {mode !== "clone" && <p className="onboard-small">{mode === "create" ? "Choose an empty folder. Lore creates a Git repository and prepares your memory folders." : "Your existing notes stay in place. This PC gets its own search index."}</p>}
          {!gitReady && mode === "create" && <div className="onboard-small">Git is needed to create a new repository. <button type="button" className="onboard-text-button" onClick={() => void prepare({ task: "install_git" })}>Prepare Git installation</button><button type="button" className="onboard-text-button" onClick={() => void api.setupOpenDocs("git")}>Official Git instructions</button></div>}
          <div className="onboard-main-action"><button type="button" className="onboard-primary" disabled={!root.trim() || busyNow || (mode === "create" && !gitReady)} onClick={() => void perform("folder", async () => { await api.setupRepository(root, mode === "create"); await move(3); })}>{busy === "folder" ? <LoaderCircle size={16} className="setup-spin" /> : null}{mode === "create" ? "Create my memory" : "Use this folder"}<ArrowRight size={16} /></button></div>
        </>}

        {step === 3 && <>
          <div className="onboard-heading"><span className="onboard-overline">JUST WHAT THIS COMPUTER NEEDS</span><h1>{busy && !["next", "later", "resume"].includes(busy) ? "Getting your memory ready." : checking ? "Let’s see what’s ready." : need ? need.title : "Let your AI pick up the thread."}</h1><p>{checking ? "Checking your installed tools and sign-in. Existing installations are reused." : need ? need.description : "Connect memory to your tools and prepare local search.\nWe’ll keep your existing instructions and settings."}</p></div>
          {checking ? <div className="onboard-checking"><LoaderCircle size={28} /><span>Checking this PC</span></div> : need ? <>
            <div className="onboard-preparation-icon">{need.action ? <MessageSquare size={34} strokeWidth={1.3} /> : need.task?.startsWith("login") ? <ShieldCheck size={34} strokeWidth={1.3} /> : <Download size={34} strokeWidth={1.3} />}</div>
            <div className="onboard-main-action">{need.action === "resume" ? <button type="button" className="onboard-primary" disabled={busyNow} onClick={() => void perform("resume", async () => { await api.setPaused(false); await checkTools(); })}>Resume recording <ArrowRight size={16} /></button> : need.task ? <button type="button" className="onboard-primary" disabled={busyNow} onClick={() => void prepare({ task: need.task! })}>{need.task.startsWith("login") ? "Open sign-in" : "Prepare installation"}<ArrowRight size={16} /></button> : <button type="button" className="onboard-primary" onClick={() => void api.setupOpenDocs(need.docs ?? "node").catch((e) => setError(message(e)))}>Open official instructions <ExternalLink size={15} /></button>}<button type="button" className="onboard-text-button" disabled={busyNow || checking} onClick={() => void checkTools()}>I’ve done this. Check again.</button></div>
          </> : busy ? <div className="onboard-preparing" role="status"><div className="preparing-ring"><BookOpen size={30} strokeWidth={1.2} /></div><strong>{busy}</strong><p>New models may take several minutes to download.<br />You can keep using your computer while Lore prepares search.</p></div> : <>
            <div className="onboard-connection-scene"><div><MessageSquare size={25} /><span>{selected.map((p) => NAMES[p]).join(" + ")}</span></div><span className="connection-thread"><i /><i /><i /></span><div className="connection-lore"><BrandMark /><span>Project memory</span></div></div>
            <div className="onboard-connection-copy"><CheckCircle2 size={16} /><p>Find earlier decisions when work begins.</p><CheckCircle2 size={16} /><p>Record useful memories when sessions finish.</p></div>
            {connectionConflict && <div className="onboard-error" role="alert">{connectionConflict.error ?? connectionConflict.hooks.error ?? "An existing qmd entry or skill needs your review. Your version has been kept."}<button type="button" className="onboard-text-button" onClick={() => void api.openIntegrations()}>Review connections</button></div>}
            <div className="onboard-main-action"><button type="button" className="onboard-primary" disabled={busyNow || !!connectionConflict || !selected.length} onClick={() => void prepareMemory()}>Connect & prepare memory <ArrowRight size={16} /></button><span>First search model download: about 640 MB. Existing models are kept.</span></div>
            <button type="button" className="onboard-disclosure" aria-expanded={details} onClick={() => setDetails(!details)}>What will change?<ChevronDown size={14} /></button>
            {details && <div className="onboard-change-details"><p>Adds the qmd search connection, a short memory rule, a search skill, and Lore’s session hooks for your selected tools. Changes stay on this PC. Existing unrelated entries are kept. Codex hook trust remains your choice.</p>{clients.map((c) => <section key={c.provider}><strong>{NAMES[c.provider]}</strong>{c.plan?.resources.map((r) => <details key={r.path}><summary><span>{r.label}</span><small>{r.state === "preserved" ? "Your existing version is kept" : r.state === "ready" ? "Already configured" : r.state === "update" ? "Update Lore entry" : r.state === "conflict" ? "Review needed" : "Add"}</small></summary><code>{r.path}</code>{r.state !== "preserved" && <pre>{r.content}</pre>}</details>)}<p className="onboard-small">SessionEnd & PreCompact hooks: <code>{c.hooks.config}</code>. The hook file is backed up before changes.</p></section>)}</div>}
          </>}
        </>}

        {step === 4 && <>
          <div className="onboard-success-mark"><Check size={27} strokeWidth={1.7} /></div>
          <div className="onboard-heading"><span className="onboard-overline">THE NEXT STEP IS YOURS</span><h1>{configured ? "Ready to start remembering." : "Let’s finish the connection."}</h1><p>{configured ? "Your memory folder and search connections are prepared.\nHere’s how to try your first memory." : "Some connections still need attention. Return to preparation to finish setting up this PC."}</p></div>
          {configured ? <>
            {selected.includes("codex") && <div className="onboard-trust-note"><ShieldCheck size={18} /><p>Restart Codex, open <code>/hooks</code>, and trust Lore’s hooks. In <code>/mcp</code>, check that <code>qmd</code> is connected.</p></div>}
            {selected.includes("claude") && <p className="onboard-small">Restart Claude Code to load the memory skill and qmd connection.</p>}
            <div className="onboard-first-use"><div><span>1</span><p><strong>Make one decision in your project.</strong>Work in your normal coding session. When it ends, Lore records the useful parts.</p></div><div><span>2</span><p><strong>Ask about it in a new session.</strong>Your AI can search qmd, read the note, and continue with that context.</p></div></div>
            <p className="onboard-small">First recording and AI recall are not verified yet. Try them when you’re ready; you can start using Lore now.</p>
            <div className="onboard-main-action"><button type="button" className="onboard-primary" onClick={() => void api.hideWindow()}>Start using Lore <ArrowRight size={17} /></button></div>
          </> : <div className="onboard-main-action"><button type="button" className="onboard-primary" onClick={() => void perform("next", () => move(3))}>Finish preparation <ArrowRight size={16} /></button></div>}
        </>}

        <div ref={terminalContainer} hidden={!showTerminal}><SetupTerminal command={command} prepared={prepared} onRunningChange={setTerminalActive} onFinished={() => { if (step === 3) void checkTools(); else void refresh(); }} onError={(e) => setError(message(e))} /></div>
      </div>
    </div>
    <footer className="onboard-footer"><button type="button" className="onboard-back" disabled={step === 0 || busyNow} onClick={() => void perform("back", () => move(step - 1))}><ArrowLeft size={15} />Back</button><div className="onboard-progress" aria-label={`Setup: ${STAGES[step]}`}>{STAGES.map((name, i) => <span key={name} className={i === step ? "current" : i < step ? "past" : ""} title={name} />)}</div><span className="onboard-progress-caption">{step > 0 && "Your progress is saved"}</span></footer>
  </main>;
}
