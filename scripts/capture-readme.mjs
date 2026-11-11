// Render the real React UI with synthetic, public-safe IPC responses.
// Run against `pnpm exec vite --host 127.0.0.1 --port 1462`.
// This captures frontend illustrations, not a native/model/Git integration test.
import { spawn } from 'node:child_process';
import { existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';

const origin = process.env.LORE_SCREENSHOT_URL ?? 'http://127.0.0.1:1462';
const executable = process.env.LORE_SCREENSHOT_BROWSER ?? '/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge';
if (!existsSync(executable)) throw new Error('Set LORE_SCREENSHOT_BROWSER to a Chromium-based browser executable.');
const output = resolve('docs/screenshots');
mkdirSync(output, { recursive: true });
const profile = mkdtempSync(join(tmpdir(), 'lore-readme-'));
const portFile = join(profile, 'DevToolsActivePort');
const browser = spawn(executable, ['--headless', '--disable-gpu', '--disable-background-networking', '--no-first-run', '--no-default-browser-check', `--user-data-dir=${profile}`, '--remote-debugging-port=0', 'about:blank'], { stdio: 'ignore' });
const delay = ms => new Promise(resolve => setTimeout(resolve, ms));
const now = Math.floor(Date.now() / 1000);
const root = '~/Documents/lore-memory';
const hook = provider => ({ provider, state: 'connected', config: provider === 'codex' ? '~/.codex/hooks.json' : '~/.claude/settings.json', command: `lore-cli hook ${provider}`, found: `lore-cli hook ${provider}`, error: null });
const snapshot = {
  state: 'working', current: { jobId: 'sample-active', provider: 'claude', project: 'demo-app', startedAt: now - 34 },
  counts: { ready: 2, waiting: 0, failed: 0, done: 12, skipped: 0 },
  recent: [
    { id: 3, at: now - 240, jobId: 'sample-3', provider: 'codex', project: 'demo-app', kind: 'recorded', summary: 'Keep the first release focused on email sign-in.', notes: ['projects/demo-app/sign-in.md', 'projects/demo-app/release-scope.md'], error: null },
    { id: 2, at: now - 1200, jobId: 'sample-2', provider: 'claude', project: 'log-viewer', kind: 'recorded', summary: 'Search highlights matches; filters control visible rows.', notes: ['projects/log-viewer/search.md'], error: null },
    { id: 1, at: now - 2100, jobId: 'sample-1', provider: 'codex', project: 'demo-app', kind: 'no_change', summary: 'No new decisions to record.', notes: [], error: null },
  ],
  attention: [], problem: null,
  update: { phase: 'idle', version: null, notes: null, downloaded: 0, total: null, error: null },
  settings: { knowledgeRoot: root, backend: 'codex', models: { codex: null, claude: null }, efforts: { codex: null, claude: null }, modelTimeoutSecs: 600, qmdTimeoutSecs: 900, claudeProjectsDir: '~/.claude/projects', codexSessionsDir: '~/.codex/sessions', idleSecs: 600, since: now - 86400, paused: false, syncEnabled: false, maxChars: 30000, notesBudget: 60000, qmdIndex: 'index', commands: { codex: ['codex'], claude: ['claude'], qmd: ['qmd'] } },
  autostart: false, hooks: [hook('claude'), hook('codex')], connections: [{ provider: 'claude', state: 'connected' }, { provider: 'codex', state: 'connected' }],
  review: 3, reviewing: false, version: '0.1.1', sync: { phase: 'idle', message: null, issues: 0, blocked: false, lastSyncedAt: now - 300 }, syncing: false,
  catalog: { codex: [{ value: 'gpt-6.1-sol', label: 'GPT-6.1 Sol', resolved: null, efforts: ['low', 'medium', 'high'] }], claude: [{ value: 'haiku', label: 'Haiku', resolved: null, efforts: [] }] },
  defaultModels: { codex: 'gpt-6.1-sol', claude: 'haiku' }, defaultEfforts: { codex: 'low', claude: null },
};
const setup = {
  progress: { step: 0, completed: false, deferred: false, providers: ['codex', 'claude'], testStartedAt: null, recallConfirmed: false },
  settings: snapshot.settings,
  diagnostics: { commands: Object.fromEntries(['codex', 'claude', 'qmd'].map(name => [name, { command: [name], available: true }])), indexConfigured: true, problems: [], qmdConfigDir: '~/Library/Application Support/io.github.junyoungjo.lore/qmd/config', qmdCacheDir: '~/Library/Caches/lore/qmd', hookCli: '/Applications/Lore.app/Contents/MacOS/lore-cli' },
  tools: ['git', 'node', 'npm', 'brew', 'sqlite3'].map(name => ({ name, path: `/opt/homebrew/bin/${name}` })),
  clients: ['codex', 'claude'].map(provider => ({ provider, error: null, hooks: hook(provider), plan: { token: `example-${provider}`, canApply: true, resources: [
    { label: 'qmd MCP', path: provider === 'codex' ? '~/.codex/config.toml' : '~/.claude.json', state: 'ready', detail: 'Search this PC’s memory index.', content: 'Example managed qmd connection.' },
    { label: 'Memory guidance', path: provider === 'codex' ? '~/.codex/AGENTS.md' : '~/.claude/CLAUDE.md', state: 'ready', detail: 'Search relevant project and shared memory before working.', content: 'Example managed guidance.' },
    { label: 'Memory search skill', path: `~/.${provider}/skills/lore-memory/SKILL.md`, state: 'ready', detail: 'Search notes and inspect supporting conversations.', content: 'Example search skill.' },
  ] } })),
  platform: 'macos', recorded: 0, testRecorded: false, rootExists: true, qwenModel: 'example:qwen3-embedding', embedModel: 'example:qwen3-embedding',
  embedModels: [{ uri: 'example:qwen3-embedding', name: 'Qwen3-Embedding-0.6B', size: 'about 640 MB', note: 'Multilingual, including Korean.' }, { uri: 'example:embeddinggemma', name: 'embeddinggemma-300M', size: 'under 350 MB', note: 'qmd’s default.' }],
  collections: ['common', 'project-demo-app', 'project-log-viewer'], repository: true, tracking: 'main ↔ origin/main',
};
const notePath = 'projects/demo-app/sign-in.md';
const local = '# First-release sign-in\n\nUse email sign-in for the first release.\n\n## Sources\n- [Local decision](../../sources/claude/example.md#L1)\n- [Other PC](../../sources/codex/example.md#L1)\n';
const final = local.replace('Use email sign-in for the first release.', 'Use email sign-in for the first release. Revisit passkeys after launch.');
const plan = {
  id: 'illustrative-comparison', root, branch: 'main', remoteName: 'origin', remoteRef: 'refs/heads/main', local: 'aaaa1111', remote: 'bbbb2222', base: 'cccc3333', tree: 'dddd4444', phase: 'review', approved: null, message: null,
  files: [{ path: notePath, base: '# First-release sign-in\n\nSign-in method is undecided.\n', local, remote: local.replace('email sign-in', 'passkeys'), merged: local, textConflict: false }],
  sources: [{ id: 'local:example', side: 'local', path: 'sources/claude/example.md', text: 'User: Let’s start with email sign-in to keep the first release simple.' }, { id: 'remote:example', side: 'remote', path: 'sources/codex/example.md', text: 'User: Use passkeys for sign-in.\nAssistant: We should confirm whether this changes the first release.' }],
  unsupported: [], otherFiles: ['sources/claude/example.md', 'sources/codex/example.md'],
  analysis: { summary: 'The two PCs recorded different sign-in plans for the first release.', issues: [{ title: 'First-release sign-in', explanation: 'One note keeps email sign-in for a simpler first release; the other asks for passkeys. Confirm which plan to share before continuing.', evidence: [{ source: 'local:example', quote: 'Let’s start with email sign-in to keep the first release simple.' }, { source: 'remote:example', quote: 'Use passkeys for sign-in.' }], options: [
    { label: 'Email first, revisit passkeys', description: 'Keep the initial release focused and retain the later request as a follow-up.', edits: [{ path: notePath, find: 'Use email sign-in for the first release.', text: 'Use email sign-in for the first release. Revisit passkeys after launch.' }] },
    { label: 'Use passkeys in the first release', description: 'Update the current plan while retaining both source conversations.', edits: [{ path: notePath, find: 'Use email sign-in for the first release.', text: 'Use passkeys for the first release.' }] },
  ] }] },
};
const preview = { files: { [notePath]: final }, diffs: { [notePath]: local.trimEnd().split('\n').map((line, i) => ({ kind: i === 2 ? 'changed' : 'same', leftNo: i + 1, rightNo: i + 1, left: [{ kind: i === 2 ? 'removed' : 'same', text: line }], right: [{ kind: i === 2 ? 'added' : 'same', text: final.trimEnd().split('\n')[i] }] })) }, token: 'illustrative-preview' };
const sync = { root, enabled: false, busy: false, summary: { phase: 'review', message: null, blocked: true, issues: 1, lastSyncedAt: now - 7200 }, plan, tracking: 'main ↔ origin/main', model: 'Codex', review: 0, changes: [{ path: notePath, local: { added: 2, removed: 1 }, remote: { added: 2, removed: 1 } }] };
const queue = ['demo-app', 'log-viewer'].map((project, i) => ({ id: `sample-queue-${i}`, provider: i ? 'claude' : 'codex', transcript: `/example/${i}.jsonl`, status: 'pending', cursorLine: 0, cursorHash: '', project, sessionId: `example-${i}`, attempts: 0, nextAttemptAt: 0, lastError: null, updatedAt: now - 60 }));
const sockets = [];
const targets = [];
const reports = [];

try {
  for (let attempt = 0; attempt < 200 && !existsSync(portFile); attempt++) await delay(50);
  const port = Number(readFileSync(portFile, 'utf8').split('\n')[0]);
  async function open(label, width, height, theme = 'dark') {
    const target = await (await fetch(`http://127.0.0.1:${port}/json/new?about:blank`, { method: 'PUT' })).json();
    targets.push(target.id);
    const socket = new WebSocket(target.webSocketDebuggerUrl); sockets.push(socket);
    await new Promise((resolve, reject) => { socket.onopen = resolve; socket.onerror = reject; });
    let nextId = 0;
    const pending = new Map(), errors = [];
    socket.onmessage = event => {
      const message = JSON.parse(event.data);
      if (message.id) {
        const request = pending.get(message.id); pending.delete(message.id);
        message.error ? request.reject(new Error(JSON.stringify(message.error))) : request.resolve(message.result);
      }
      if (message.method === 'Runtime.exceptionThrown') errors.push(message.params.exceptionDetails.text);
    };
    const send = (method, params = {}) => new Promise((resolve, reject) => { const id = ++nextId; pending.set(id, { resolve, reject }); socket.send(JSON.stringify({ id, method, params })); });
    const evaluate = async expression => {
      const result = await send('Runtime.evaluate', { expression, returnByValue: true, awaitPromise: true });
      if (result.exceptionDetails) throw new Error(JSON.stringify(result.exceptionDetails));
      return result.result.value;
    };
    const wait = expression => evaluate(`(async()=>{for(let i=0;i<200;i++){if(${expression})return true;await new Promise(r=>setTimeout(r,50));}throw Error('Not ready: '+${JSON.stringify(expression)}+'; '+document.body.innerText.slice(0,500));})()`);
    await send('Page.enable'); await send('Runtime.enable');
    await send('Emulation.setDeviceMetricsOverride', { width, height, deviceScaleFactor: label === 'tray' ? 2 : 1.5, mobile: false });
    await send('Page.addScriptToEvaluateOnNewDocument', { source: `
      localStorage.setItem('lore.theme',${JSON.stringify(theme)});
      const snapshot=${JSON.stringify(snapshot)},setup=${JSON.stringify(setup)},sync=${JSON.stringify(sync)},preview=${JSON.stringify(preview)},queue=${JSON.stringify(queue)};
      window.__TAURI_EVENT_PLUGIN_INTERNALS__={unregisterListener(){}};
      window.__TAURI_INTERNALS__={metadata:{currentWindow:{label:${JSON.stringify(label)}},currentWebview:{label:${JSON.stringify(label)}}},transformCallback(){return 1},unregisterCallback(){},invoke:async(command,args)=>{
        if(command==='get_snapshot')return structuredClone(snapshot);
        if(command==='get_queue')return queue;
        if(command==='get_setup')return structuredClone(setup);
        if(command==='get_sync')return structuredClone(sync);
        if(command==='preview_sync')return preview;
        if(command==='setup_check_tools')return {nodeReady:true,nodeVersion:'v24.15.0',auth:{codex:'ready',claude:'ready'}};
        if(command==='save_settings'){snapshot.settings={...snapshot.settings,...args.settings};return;}
        if(command==='save_setup_progress'){setup.progress=args.progress;return;}
        if(command==='setup_detect')return;
        if(command==='setup_terminal_read')return {bytes:[],offset:0,running:false,exitCode:null};
        if(command.startsWith('plugin:event|'))return 0;
        throw Error('Unexpected screenshot IPC: '+command);
      }};
    ` });
    await send('Page.navigate', { url: origin });
    await wait('document.querySelector(".head,.int-window,.sync-window,.onboard")');
    await evaluate('document.fonts.ready');
    await evaluate(`(()=>{const style=document.createElement('style');style.textContent='*,*::before,*::after{animation:none!important;transition:none!important;caret-color:transparent!important}';document.head.append(style)})()`);
    const capture = async name => {
      const content = await evaluate('document.body.innerText');
      if (/[가-힣]/.test(content)) throw new Error(`Unexpected Korean content in ${name}`);
      if (await evaluate('document.documentElement.scrollWidth>innerWidth')) throw new Error(`Page overflow in ${name}`);
      if (errors.length) throw new Error(errors.join('\n'));
      if (await evaluate('Boolean(document.querySelector("[role=alert]"))')) throw new Error(`Unexpected visible error in ${name}`);
      const shot = await send('Page.captureScreenshot', { format: 'png' });
      writeFileSync(join(output, `${name}.png`), Buffer.from(shot.data, 'base64'));
      reports.push({ name, width, height, theme, browserErrors: errors.length });
    };
    const click = async expression => { await evaluate(`(${expression}).click()`); };
    return { evaluate, wait, capture, click };
  }
  const dark = await open('tray', 400, 560);
  await dark.wait(`document.querySelector('[aria-label="Recent activity"]')`);
  await dark.capture('main');
  await dark.click('document.querySelector(".notes-btn")');
  await dark.wait('document.querySelector(".notes-card")');
  await dark.capture('notes');

  const light = await open('tray', 400, 560, 'light');
  await light.wait(`document.querySelector('[aria-label="Recent activity"]')`);
  await light.capture('main-light');
  await light.click(`document.querySelector('[aria-label="Settings"]')`);
  await light.wait('document.querySelector("#model")');
  await light.capture('settings');
  await light.click(`document.querySelector('[aria-label="Back"]')`);
  await light.wait('document.querySelector(".now-queue")');
  await light.click('document.querySelector(".now-queue")');
  await light.wait(`document.querySelector('[aria-label="Pending"]')`);
  await light.capture('queue');

  const integrations = await open('integrations', 1040, 760);
  await integrations.wait('document.querySelector(".int-pane h1")');
  await integrations.capture('integrations');
  await integrations.click('Array.from(document.querySelectorAll(".int-item")).find(button=>button.textContent.includes("Search index"))');
  await integrations.wait('document.querySelector(".int-pane").getAttribute("aria-label")==="Search index"');
  await integrations.capture('search-index');

  const setupUi = await open('setup', 760, 640, 'light');
  await setupUi.wait('document.querySelector(".memory-story")');
  await setupUi.capture('onboarding');
  await setupUi.click('Array.from(document.querySelectorAll("button")).find(button=>button.textContent.trim()==="Get started")');
  await setupUi.wait('document.body.innerText.includes("Where do you work?")');
  await setupUi.capture('onboarding-tools');

  const syncUi = await open('sync', 1120, 800);
  await syncUi.wait('document.querySelector("input[type=radio]")');
  await syncUi.click('document.querySelector("input[type=radio]")');
  await syncUi.wait('Array.from(document.querySelectorAll("button")).some(button=>button.textContent.includes("Check result")&&!button.disabled)');
  await syncUi.capture('sync-review');
  await syncUi.click('Array.from(document.querySelectorAll("button")).find(button=>button.textContent.includes("Check result"))');
  await syncUi.wait(`document.querySelector('[aria-label="Final check"]')&&Array.from(document.querySelectorAll("button")).some(button=>button.textContent==="Apply & share"&&!button.disabled)`);
  await syncUi.capture('sync-preview');
  console.log(JSON.stringify({ ui: 'actual React components', ipc: 'synthetic fixtures', nativeIntegrationTest: false, captures: reports }, null, 2));
} finally {
  sockets.forEach(socket => socket.close());
  browser.kill('SIGTERM');
  await new Promise(resolve => browser.once('exit', resolve));
  rmSync(profile, { recursive: true, force: true });
}
