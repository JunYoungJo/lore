// Real model integration exercise using local-only Git remotes and an indexing
// fixture. Option 0 is selected only in these synthetic test repositories.
import { spawnSync } from "node:child_process";
import { existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const args = process.argv.slice(2);
const option = (name, fallback) => args.includes(name) ? args[args.indexOf(name) + 1] : fallback;
const output = resolve(option("--output", join(root, "work-sync-evaluation", new Date().toISOString().replaceAll(":", "-"))));
const backend = option("--backend", "codex");
if (!["codex", "claude"].includes(backend)) throw new Error("--backend must be codex or claude.");
if (existsSync(output)) throw new Error(`The evaluation folder already exists and is not overwritten: ${output}`);
mkdirSync(output, { recursive: true });
const cli = join(root, "target/release", `lore-cli${process.platform === "win32" ? ".exe" : ""}`);
function run(program, argv, cwd, env) {
  const result = spawnSync(program, argv, { cwd, env, encoding: "utf8", timeout: 360000, maxBuffer: 16 * 1024 * 1024 });
  if (result.error || result.status !== 0) throw new Error(result.error?.message || result.stderr || result.stdout);
  return result.stdout.trim();
}
const git = (cwd, ...argv) => run("git", ["-c", "commit.gpgsign=false", ...argv], cwd, process.env);
function write(folder, file, text) { const path = join(folder, file); mkdirSync(dirname(path), { recursive: true }); writeFileSync(path, text); }
function commit(folder) { git(folder, "add", "."); git(folder, "commit", "-m", "synthetic fixture"); }
function config(folder) { git(folder, "config", "user.name", "Model evaluation"); git(folder, "config", "user.email", "evaluation@example.invalid"); git(folder, "config", "core.autocrlf", "false"); }
const note = "projects/demo/error-policy.md";
const reports = [];
for (const kind of ["text-conflict", "semantic-conflict"]) {
  process.stdout.write(`${kind}: `);
  const folder = join(output, kind); mkdirSync(folder);
  git(folder, "init", "--bare", "--initial-branch=main", "remote.git");
  git(folder, "clone", "remote.git", "local");
  const local = join(folder, "local"); config(local);
  const base = "# Error notifications\n\n## Current policy\n\nThe error notification policy is undecided.\n\n## Unchanged behavior\n\nKeep the measurement behavior unchanged.\n\nKeep data retention unchanged in every mode.\n\nDo not add new error types.\n\nPreserve the other sections of this document.\n\n## Field operation\n\nThe field notification policy is undecided.\n\n## Sources\n- [Windows L1](<../../sources/claude/windows-session.md#L1>)\n- [Mac L1](<../../sources/codex/mac-session.md#L1>)\n";
  write(local, note, base);
  write(local, "sources/claude/windows-session.md", '# Windows example\n\n<a id="L1"></a>\n## L1 · User · 2026-11-10T01:00:00Z\n\nSkip error popups and write to the log only. Do not interrupt the operator.\n');
  write(local, "sources/codex/mac-session.md", '# Mac example\n\n<a id="L1"></a>\n## L1 · User · 2026-11-10T02:00:00Z\n\nOperators must notice errors immediately. Show a popup when an error occurs.\n');
  commit(local); git(local, "push", "-u", "origin", "main");
  git(folder, "clone", "remote.git", "other"); const other = join(folder, "other"); config(other);
  write(local, note, base.replace("The error notification policy is undecided.", "Log errors without popups to avoid interrupting field operation.")); commit(local);
  const remoteText = kind === "text-conflict" ? base.replace("The error notification policy is undecided.", "Show an error popup so operators notice errors immediately.") : base.replace("The field notification policy is undecided.", "Always show an error popup. Operators must notice errors immediately.");
  write(other, note, remoteText); commit(other); git(other, "push");
  const data = join(folder, "app"); mkdirSync(data);
  writeFileSync(join(data, "settings.json"), JSON.stringify({ knowledgeRoot: local, backend, modelTimeoutSecs: 240, commands: { qmd: [process.execPath, join(root, "scripts/evaluation-index-fixture.mjs")] } }, null, 2));
  const env = { ...process.env, LORE_HOME: data };
  const command = (...argv) => JSON.parse(run(cli, ["sync", ...argv], local, env));
  // Sync merges and shares without asking; the model comparison runs afterwards.
  const synced = command("now");
  const mergedText = readFileSync(join(local, note), "utf8");
  const bothHistories = git(local, "show", "-s", "--format=%P").split(/\s+/).length === 2;
  const sharedBeforeComparison = git(local, "rev-parse", "HEAD") === git(local, "ls-remote", "origin", "refs/heads/main").split(/\s+/)[0];
  const compared = command("compare");
  const decisions = compared.state.items.filter((item) => item.kind === "decision");
  if (!decisions.length) throw new Error("The model did not report the conflicting decisions as an item to check.");
  for (const item of decisions) command("resolve", item.id, "0");
  const resolved = command("now");
  const finalText = readFileSync(join(local, note), "utf8");
  git(other, "pull", "--ff-only");
  const checks = { sharedBeforeComparison, bothHistories, noConflictMarkers: !/^(<<<<<<<|>>>>>>>)/m.test(mergedText), decisionsCleared: resolved.state.items.every((item) => item.kind !== "decision"), unchangedParagraph: finalText.includes("Keep the measurement behavior unchanged."), bothEvidenceLinks: finalText.includes("windows-session.md#L1") && finalText.includes("mac-session.md#L1"), otherPcMatches: finalText === readFileSync(join(other, note), "utf8") };
  reports.push({ kind, backend, listedAfterSync: synced.state.items, decisions, mergedText, finalText, checks, actualQmd: false });
  writeFileSync(join(output, "report.json"), JSON.stringify(reports, null, 2));
  process.stdout.write(`${Object.values(checks).every(Boolean) ? "passed" : "needs a look"} · ${decisions.length} item(s) found by the model\n`);
  if (!Object.values(checks).every(Boolean)) process.exitCode = 1;
}
process.stdout.write(`Report: ${join(output, "report.json")}\n`);
