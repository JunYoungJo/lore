import { act, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { SetupSnapshot, ToolChecks } from "@/ipc/contract";

const api = vi.hoisted(() => ({
  getSetup: vi.fn(), saveSetupProgress: vi.fn(), setupPrepare: vi.fn(), setupConnect: vi.fn(), setupDisconnect: vi.fn(),
  setupDetect: vi.fn(), setupRepository: vi.fn(), setupIndex: vi.fn(), setupBeginTest: vi.fn(), saveSettings: vi.fn(),
  hideWindow: vi.fn(), setupOpenDocs: vi.fn(), setupCheckTools: vi.fn(), setupPickFolder: vi.fn(), setPaused: vi.fn(),
  openIntegrations: vi.fn(), openSetup: vi.fn(), openFolder: vi.fn(), setupEmbedModel: vi.fn(), onStateChanged: vi.fn(() => Promise.resolve(() => {})),
  onIntegrationsSection: vi.fn((_handler: (section: string) => void) => Promise.resolve(() => {})),
}));
vi.mock("@/ipc/api", () => api);
vi.mock("@/components/SetupTerminal", () => ({ SetupTerminal: ({ prepared }: { prepared: { command: string } | null }) => <section aria-label="Setup terminal">{prepared?.command ?? "Choose command"}</section> }));
import { nextSetupNeed, SetupApp } from "@/SetupApp";
import { IntegrationManager } from "@/IntegrationManager";
import { setThemePreference } from "@/theme";

function fixture(step = 0): SetupSnapshot {
  return {
    progress: { step, completed: false, deferred: false, providers: ["codex"], testStartedAt: null, recallConfirmed: false },
    settings: {
      knowledgeRoot: "/Users/test/Documents/lore-memory", backend: "claude", models: { codex: null, claude: "opus" }, efforts: { codex: null, claude: null },
      modelTimeoutSecs: 1800, qmdTimeoutSecs: 900, claudeProjectsDir: "/sessions/claude", codexSessionsDir: "/sessions/codex",
      idleSecs: 600, since: 42, paused: false, syncEnabled: false, maxChars: 50000, notesBudget: 100000, qmdIndex: "index",
      commands: { codex: ["/tools/codex"], claude: ["/tools/claude"], qmd: ["/tools/qmd"] },
    },
    diagnostics: { commands: { codex: { command: ["/tools/codex"], available: true }, claude: { command: ["/tools/claude"], available: true }, qmd: { command: ["/tools/qmd"], available: true } }, indexConfigured: true, problems: [], qmdConfigDir: "/app/qmd/config", qmdCacheDir: "/app/qmd/cache", hookCli: "/app/lore-cli" },
    tools: ["git", "node", "npm", "brew", "sqlite3"].map((name) => ({ name, path: `/tools/${name}` })),
    clients: ["codex", "claude"].map((p) => ({
      provider: p as "codex" | "claude", error: null,
      hooks: { provider: p as "codex" | "claude", state: "connected", config: `/home/.${p}/hooks.json`, command: `/app/lore-cli hook ${p}`, found: null, error: null },
      plan: { token: `${p}-review-token`, canApply: true, resources: [
        { label: "qmd MCP", path: `/home/.${p}/config`, state: "add", detail: "Local index", content: "qmd config" },
        { label: "Memory guidance", path: `/home/.${p}/rules`, state: "preserved", detail: "Keep user rules", content: "New rules" },
      ] },
    })),
    platform: "macos", recorded: 0, testRecorded: false, rootExists: true, qwenModel: "qwen-model",
    embedModel: QWEN, embedModels: [
      { uri: QWEN, name: "Qwen3-Embedding-0.6B", size: "about 640 MB", note: "Multilingual, including Korean." },
      { uri: GEMMA, name: "embeddinggemma-300M", size: "under 350 MB", note: "qmd’s default." },
    ],
    collections: ["common", "project-demo"], repository: true, tracking: "main ↔ origin/main",
  };
}
const QWEN = "hf:qwen/embed.gguf";
const GEMMA = "hf:gemma/embed.gguf";
const checks: ToolChecks = { nodeReady: true, nodeVersion: "v24.15.0", auth: { codex: "ready", claude: "ready" } };
let current: SetupSnapshot;

beforeEach(() => {
  vi.clearAllMocks();
  current = fixture();
  api.getSetup.mockImplementation(async () => structuredClone(current));
  api.saveSetupProgress.mockImplementation(async (progress) => { current.progress = progress; });
  api.setupCheckTools.mockResolvedValue(checks);
  api.setupDetect.mockResolvedValue(undefined);
  api.setupIndex.mockResolvedValue(undefined);
  api.saveSettings.mockResolvedValue(undefined);
  api.setupConnect.mockImplementation(async (provider) => { current.clients.find((c) => c.provider === provider)!.plan!.resources.forEach((r) => { r.state = "ready"; }); });
  api.setupRepository.mockResolvedValue(undefined);
});

describe("first-run experience", () => {
  it("starts with a memory example and no settings sidebar or exposed terminal", async () => {
    render(<SetupApp />);
    expect(await screen.findByRole("heading", { name: /Your next session/ })).toBeInTheDocument();
    expect(screen.queryByRole("navigation", { name: "Setup steps" })).not.toBeInTheDocument();
    expect(screen.queryByRole("region", { name: "Setup terminal" })).not.toBeInTheDocument();
    expect(screen.queryByText("Your progress is saved")).not.toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Next example" }));
    expect(screen.getByText("The reason stays with the decision")).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Get started" }));
    expect(await screen.findByRole("heading", { name: "Where do you work?" })).toBeInTheDocument();
    expect(screen.getByText("Your progress is saved")).toBeInTheDocument();
    expect(api.setupConnect).not.toHaveBeenCalled();
    expect(api.setupPrepare).not.toHaveBeenCalled();
  });

  it("resumes the folder choice and creates only after the explicit action", async () => {
    current = fixture(2);
    render(<SetupApp />);
    await userEvent.click(await screen.findByRole("button", { name: "Start fresh" }));
    expect(api.setupRepository).not.toHaveBeenCalled();
    await userEvent.clear(screen.getByLabelText("Memory folder"));
    await userEvent.type(screen.getByLabelText("Memory folder"), "/new/memory");
    await userEvent.click(screen.getByRole("button", { name: "Create my memory" }));
    expect(api.setupRepository).toHaveBeenCalledWith("/new/memory", true);
  });

  it("uses a native folder picker without changing settings until the folder action", async () => {
    current = fixture(2);
    api.setupPickFolder.mockResolvedValue("/picked/memory");
    render(<SetupApp />);
    await userEvent.click(await screen.findByRole("button", { name: "Choose folder" }));
    expect(screen.getByLabelText("Memory folder")).toHaveValue("/picked/memory");
    expect(api.setupRepository).not.toHaveBeenCalled();
  });

  it("shows only the missing sign-in and opens a prepared command after the user chooses it", async () => {
    current = fixture(3);
    api.setupCheckTools.mockResolvedValue({ ...checks, auth: { ...checks.auth, codex: "signed_out" } });
    api.setupPrepare.mockResolvedValue({ command: "/tools/codex login", argv: ["/tools/codex", "login"], cwd: "/home", env: [], token: "reviewed" });
    render(<SetupApp />);
    expect(await screen.findByRole("heading", { name: "Sign in to Codex" })).toBeInTheDocument();
    expect(api.setupPrepare).not.toHaveBeenCalled();
    await userEvent.click(screen.getByRole("button", { name: "Open sign-in" }));
    expect(api.setupPrepare).toHaveBeenCalledWith({ task: "login_codex" });
    expect(screen.getByRole("region", { name: "Setup terminal" })).toHaveTextContent("/tools/codex login");
  });

  it("prepares search and connects only selected tools, without blocking completion on an external session", async () => {
    current = fixture(3);
    render(<SetupApp />);
    const connect = await screen.findByRole("button", { name: "Connect & prepare memory" });
    expect(api.setupIndex).not.toHaveBeenCalled();
    await userEvent.click(connect);
    expect(await screen.findByRole("heading", { name: "Ready to start remembering." })).toBeInTheDocument();
    expect(api.setupIndex).toHaveBeenCalledTimes(1);
    expect(api.setupConnect).toHaveBeenCalledTimes(1);
    expect(api.setupConnect).toHaveBeenCalledWith("codex", "codex-review-token");
    expect(api.saveSetupProgress).toHaveBeenLastCalledWith(expect.objectContaining({ completed: true, step: 4 }));
    expect(screen.getByText(/First recording and AI recall are not verified yet/)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Start using Lore" })).toBeEnabled();
  });

  it("can defer without calling setup complete", async () => {
    current = fixture(2);
    render(<SetupApp />);
    await userEvent.click(await screen.findByRole("button", { name: "Set up later" }));
    expect(api.saveSetupProgress).toHaveBeenCalledWith(expect.objectContaining({ step: 2, deferred: true, completed: false }));
    expect(api.hideWindow).toHaveBeenCalledTimes(1);
  });

  it("keeps conflicting settings and sends the user to connection management", async () => {
    current = fixture(3);
    current.clients[0].plan!.canApply = false;
    current.clients[0].plan!.resources[0].state = "conflict";
    render(<SetupApp />);
    expect(await screen.findByRole("button", { name: "Connect & prepare memory" })).toBeDisabled();
    await userEvent.click(screen.getByRole("button", { name: "Review connections" }));
    expect(api.openIntegrations).toHaveBeenCalledOnce();
    expect(api.setupConnect).not.toHaveBeenCalled();
  });

  it("checks runtime compatibility instead of treating an existing old Node as ready", () => {
    const need = nextSetupNeed(fixture(3), { ...checks, nodeReady: false, nodeVersion: "v20.0.0" }, ["codex"]);
    expect(need?.task).toBe("install_node");
    expect(need?.description).toContain("v20.0.0");
  });
});

describe("separate integration management", () => {
  const OTHER_CLI = "C:/Users/me/AppData/Local/Lore/lore-cli.exe";
  function managed(): SetupSnapshot {
    const data = fixture(3);
    data.platform = "windows";
    for (const client of data.clients) client.plan!.resources.forEach((resource) => { resource.state = "ready"; });
    const claude = data.clients.find((client) => client.provider === "claude")!;
    claude.hooks = { ...claude.hooks, state: "other_path", found: `${OTHER_CLI} hook claude` };
    return data;
  }
  const rail = () => screen.getByRole("complementary", { name: "Manage" });
  const pane = (name: string) => screen.getByRole("region", { name });

  beforeEach(() => {
    current = managed();
    api.setupConnect.mockImplementation(async (provider) => {
      const client = current.clients.find((c) => c.provider === provider)!;
      client.plan!.resources.forEach((r) => { r.state = "ready"; });
      client.hooks = { ...client.hooks, state: "connected", found: client.hooks.command };
    });
  });

  it("follows the appearance chosen in the tray settings", async () => {
    setThemePreference("dark");
    try {
      render(<IntegrationManager />);
      await screen.findByRole("heading", { name: "Claude Code" });
      expect(document.documentElement).toHaveAttribute("data-theme", "dark");
      act(() => setThemePreference("light"));
      expect(document.documentElement).toHaveAttribute("data-theme", "light");
    } finally {
      setThemePreference("system");
    }
  });

  it("opens on the tool that needs repair, says why and repairs it with the reviewed token", async () => {
    render(<IntegrationManager />);
    expect(await screen.findByRole("heading", { name: "Claude Code" })).toBeInTheDocument();
    expect(screen.getByText("1 needs attention")).toBeInTheDocument();
    expect(within(rail()).getByRole("button", { name: /Codex.*Connected/ })).toBeInTheDocument();
    expect(within(rail()).getByRole("button", { name: /Claude Code.*Needs repair/ })).toBeInTheDocument();
    const claude = pane("Claude Code");
    expect(claude).toHaveTextContent(`They run ${OTHER_CLI}, another copy of Lore. This copy runs from /app. Repair points the hooks here.`);
    expect(claude).toHaveTextContent("Another copy");
    expect(screen.queryByRole("checkbox")).not.toBeInTheDocument();
    await userEvent.click(within(claude).getByRole("button", { name: "Repair connection" }));
    await waitFor(() => expect(api.setupConnect).toHaveBeenCalledWith("claude", "claude-review-token"));
    expect(await screen.findByText("All set")).toBeInTheDocument();
    expect(screen.getByRole("status")).toHaveTextContent("Restart Claude Code so it loads the skill and the search connection.");
  });

  it("lists the files Lore writes before connecting a tool that is not connected", async () => {
    const codex = current.clients.find((client) => client.provider === "codex")!;
    codex.plan!.resources.forEach((resource) => { resource.state = "add"; });
    codex.hooks = { ...codex.hooks, state: "missing", found: null };
    render(<IntegrationManager />);
    await userEvent.click(within(await screen.findByRole("complementary", { name: "Manage" })).getByRole("button", { name: /Codex.*Not connected/ }));
    const page = pane("Codex");
    await userEvent.click(within(page).getByRole("button", { name: "Review changes" }));
    const files = within(page).getByLabelText("Codex connection files");
    expect(files).toHaveTextContent("/home/.codex/config");
    expect(files).toHaveTextContent("/app/lore-cli hook codex");
    expect(api.setupConnect).not.toHaveBeenCalled();
    await userEvent.click(within(page).getByRole("button", { name: "Connect" }));
    await waitFor(() => expect(api.setupConnect).toHaveBeenCalledWith("codex", "codex-review-token"));
  });

  it("keeps a conflicting file and does not offer the repair", async () => {
    const claude = current.clients.find((client) => client.provider === "claude")!;
    claude.plan!.canApply = false;
    claude.plan!.resources[0].state = "conflict";
    render(<IntegrationManager />);
    const page = await screen.findByRole("region", { name: "Claude Code" });
    expect(page).toHaveTextContent("Edited outside Lore");
    expect(within(page).getByRole("button", { name: "Repair connection" })).toBeDisabled();
  });

  it("shows the sign-in state and prepares sign-in in a terminal only when asked", async () => {
    api.setupCheckTools.mockResolvedValue({ ...checks, auth: { ...checks.auth, codex: "signed_out" } });
    api.setupPrepare.mockResolvedValue({ command: "/tools/codex login", argv: ["/tools/codex", "login"], cwd: "/home", env: [], token: "reviewed" });
    render(<IntegrationManager />);
    await userEvent.click(within(await screen.findByRole("complementary", { name: "Manage" })).getByRole("button", { name: /Codex/ }));
    const page = pane("Codex");
    expect(await within(page).findByText("Signed out")).toBeInTheDocument();
    expect(page).toHaveTextContent("Codex asks you to trust new hooks.");
    expect(screen.queryByRole("region", { name: "Setup terminal" })).not.toBeInTheDocument();
    await userEvent.click(within(page).getByRole("button", { name: "Sign in" }));
    expect(api.setupPrepare).toHaveBeenCalledWith({ task: "login_codex" });
    expect(screen.getByRole("region", { name: "Setup terminal" })).toHaveTextContent("/tools/codex login");
  });

  it("opens the memory folder with the change form when settings asked to change the folder", async () => {
    window.history.replaceState(null, "", "/?section=folder");
    try {
      render(<IntegrationManager />);
      expect(await screen.findByRole("heading", { name: "Memory folder" })).toBeInTheDocument();
      expect(pane("Memory folder")).toHaveTextContent("Git repository · main ↔ origin/main");
      const change = screen.getByRole("region", { name: "Use a different folder" });
      await userEvent.click(within(change).getByRole("button", { name: /Start a new one/ }));
      await userEvent.clear(within(change).getByLabelText("Folder"));
      await userEvent.type(within(change).getByLabelText("Folder"), "/new/memory");
      expect(api.setupRepository).not.toHaveBeenCalled();
      await userEvent.click(within(change).getByRole("button", { name: "Create & use this folder" }));
      expect(api.setupRepository).toHaveBeenCalledWith("/new/memory", true);
    } finally {
      window.history.replaceState(null, "", "/");
    }
  });

  it("keeps the folder form closed when the folder is opened from the list", async () => {
    render(<IntegrationManager />);
    await userEvent.click(within(await screen.findByRole("complementary", { name: "Manage" })).getByRole("button", { name: /Memory folder/ }));
    expect(screen.queryByRole("region", { name: "Use a different folder" })).not.toBeInTheDocument();
    await userEvent.click(within(pane("Memory folder")).getByRole("button", { name: "Change…" }));
    expect(screen.getByRole("region", { name: "Use a different folder" })).toBeInTheDocument();
  });

  it("moves an open window to the memory folder on request", async () => {
    render(<IntegrationManager />);
    expect(await screen.findByRole("heading", { name: "Claude Code" })).toBeInTheDocument();
    const [[showSection]] = api.onIntegrationsSection.mock.calls;
    act(() => showSection("folder"));
    expect(screen.getByRole("heading", { name: "Memory folder" })).toBeInTheDocument();
    expect(screen.getByRole("region", { name: "Use a different folder" })).toBeInTheDocument();
  });

  it("rebuilds the index with the other embedding model only after it is chosen and confirmed", async () => {
    api.setupEmbedModel.mockImplementation(async (model: string) => { current.embedModel = model; });
    render(<IntegrationManager />);
    await userEvent.click(within(await screen.findByRole("complementary", { name: "Manage" })).getByRole("button", { name: /Search index/ }));
    const page = pane("Search index");
    expect(page).toHaveTextContent("Qwen3-Embedding-0.6B");
    expect(page).toHaveTextContent("common and 1 project");
    await userEvent.click(within(page).getByRole("button", { name: "Change…" }));
    const change = within(page).getByRole("region", { name: "Change the embedding model" });
    expect(within(change).getByRole("button", { name: "Rebuild with Qwen3-Embedding-0.6B" })).toBeDisabled();
    await userEvent.click(within(change).getByRole("button", { name: /embeddinggemma-300M/ }));
    expect(change).toHaveTextContent("rebuilds the whole index on this PC");
    expect(api.setupEmbedModel).not.toHaveBeenCalled();
    await userEvent.click(within(change).getByRole("button", { name: "Rebuild with embeddinggemma-300M" }));
    await waitFor(() => expect(api.setupEmbedModel).toHaveBeenCalledWith(GEMMA));
    expect(await screen.findByRole("status")).toHaveTextContent("Index rebuilt with embeddinggemma-300M.");
    expect(pane("Search index")).toHaveTextContent("embeddinggemma-300M");
  });

  it("points a missing qmd to Tools and opens the installer only when asked", async () => {
    current.diagnostics.commands.qmd.available = false;
    api.setupPrepare.mockResolvedValue({ command: "npm install -g @tobilu/qmd@2.8.3", argv: ["npm"], cwd: "/home", env: [], token: "t" });
    render(<IntegrationManager />);
    const list = await screen.findByRole("complementary", { name: "Manage" });
    expect(within(list).getByRole("button", { name: /Search index.*Needs qmd/ })).toBeInTheDocument();
    await userEvent.click(within(list).getByRole("button", { name: /Tools.*qmd missing/ }));
    const page = pane("Tools");
    expect(page).toHaveTextContent("v24.15.0");
    expect(screen.queryByRole("region", { name: "Setup terminal" })).not.toBeInTheDocument();
    await userEvent.click(within(page).getByRole("button", { name: "Install" }));
    expect(api.setupPrepare).toHaveBeenCalledWith({ task: "install_qmd" });
    expect(screen.getByRole("region", { name: "Setup terminal" })).toHaveTextContent("npm install -g @tobilu/qmd@2.8.3");
  });
});
