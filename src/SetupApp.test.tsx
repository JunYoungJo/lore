import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { SetupSnapshot, ToolChecks } from "@/ipc/contract";

const api = vi.hoisted(() => ({
  getSetup: vi.fn(), saveSetupProgress: vi.fn(), setupPrepare: vi.fn(), setupConnect: vi.fn(), setupDisconnect: vi.fn(),
  setupDetect: vi.fn(), setupRepository: vi.fn(), setupIndex: vi.fn(), setupBeginTest: vi.fn(), saveSettings: vi.fn(),
  hideWindow: vi.fn(), setupOpenDocs: vi.fn(), setupCheckTools: vi.fn(), setupPickFolder: vi.fn(), setPaused: vi.fn(),
  openIntegrations: vi.fn(), openSetup: vi.fn(), onStateChanged: vi.fn(() => Promise.resolve(() => {})),
}));
vi.mock("@/ipc/api", () => api);
vi.mock("@/components/SetupTerminal", () => ({ SetupTerminal: ({ prepared }: { prepared: { command: string } | null }) => <section aria-label="Setup terminal">{prepared?.command ?? "Choose command"}</section> }));
import { nextSetupNeed, SetupApp } from "@/SetupApp";
import { IntegrationManager } from "@/IntegrationManager";

function fixture(step = 0): SetupSnapshot {
  return {
    progress: { step, completed: false, deferred: false, providers: ["codex"], testStartedAt: null, recallConfirmed: false },
    settings: {
      knowledgeRoot: "/Users/test/Documents/lore-memory", backend: "claude", models: { codex: null, claude: "opus" },
      modelTimeoutSecs: 1800, qmdTimeoutSecs: 900, claudeProjectsDir: "/sessions/claude", codexSessionsDir: "/sessions/codex",
      idleSecs: 600, since: 42, paused: false, maxChars: 50000, notesBudget: 100000, qmdIndex: "index",
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
  };
}
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
    await userEvent.click(screen.getByRole("button", { name: "Next example" }));
    expect(screen.getByText("The reason stays with the decision")).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Get started" }));
    expect(await screen.findByRole("heading", { name: "Where do you work?" })).toBeInTheDocument();
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
  it("shows a file preview and applies the reviewed token without replacing existing guidance", async () => {
    render(<IntegrationManager />);
    await userEvent.click((await screen.findAllByRole("button", { name: "Review connection" }))[0]);
    const preview = screen.getByLabelText("Codex connection preview");
    expect(preview).toHaveTextContent("Your existing Lore section will be kept as written.");
    expect(api.setupConnect).not.toHaveBeenCalled();
    await userEvent.click(within(preview).getByRole("button", { name: "Apply connection" }));
    await waitFor(() => expect(api.setupConnect).toHaveBeenCalledWith("codex", "codex-review-token"));
    expect(screen.queryByRole("button", { name: "Continue later" })).not.toBeInTheDocument();
  });
});
