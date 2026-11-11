import { spawnSync } from "node:child_process";
import { mkdirSync, readFileSync, writeFileSync, readdirSync, existsSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const args = process.argv.slice(2);
const option = (name, fallback) => args.includes(name) ? args[args.indexOf(name) + 1] : fallback;
const output = resolve(option("--output", join(root, "work-evaluation", new Date().toISOString().replaceAll(":", "-"))));
const backend = option("--backend", "codex");
if (!["codex", "claude"].includes(backend)) throw new Error("--backend must be codex or claude.");
const cli = join(root, "target", "release", `lore-cli${process.platform === "win32" ? ".exe" : ""}`);
if (!existsSync(cli)) throw new Error("Run pnpm build:cli first.");
const only = option("--only", "");
const cases = JSON.parse(readFileSync(join(root, "examples/evaluation/cases.json"), "utf8")).filter((fixture) => !only || only.split(",").includes(fixture.id));
const reports = [];
const korean = /[가-힣]/;
// What the model wrote: the title and body, without the front matter and the program's source list.
const written = (text) => text.replace(/^---\n[\s\S]*?\n---\n/, "").split("\n## Sources\n")[0];
const titleOf = (text) => /^# (.*)$/m.exec(written(text))?.[1] ?? "";

function run(command, env) {
  const result = spawnSync(cli, command, { env, encoding: "utf8", timeout: 360_000 });
  if (result.error || result.status !== 0) throw new Error(result.error?.message || result.stderr || result.stdout);
  return result.stdout;
}
function notesIn(folder) {
  if (!existsSync(folder)) return [];
  return readdirSync(folder, { withFileTypes: true }).flatMap((entry) => entry.isDirectory() ? notesIn(join(folder, entry.name)) : entry.name.endsWith(".md") ? [join(folder, entry.name)] : []);
}

mkdirSync(output, { recursive: true });
for (const fixture of cases) {
  const folder = join(output, fixture.id);
  const data = join(folder, "app");
  const knowledge = join(folder, "knowledge");
  const repository = join(folder, "demo");
  if (existsSync(data)) throw new Error(`Evaluation data already exists and is not overwritten: ${data}`);
  mkdirSync(join(repository, ".git"), { recursive: true });
  mkdirSync(data, { recursive: true });
  const transcript = join(folder, "session.jsonl");
  const records = fixture.turns.map(([role, text], index) => ({
    type: role, uuid: `${fixture.id}-${index}`, sessionId: fixture.id, cwd: repository,
    userType: "external", timestamp: "2026-11-10T00:00:00Z", message: { content: text },
  }));
  writeFileSync(transcript, records.map((record) => JSON.stringify(record)).join("\n") + "\n");
  if (fixture.existing) {
    mkdirSync(join(knowledge, "projects/demo"), { recursive: true });
    writeFileSync(join(knowledge, "projects/demo/alerts.md"), fixture.existing);
  }
  const settings = { knowledgeRoot: knowledge, backend, modelTimeoutSecs: 240, models: {}, commands: { qmd: [process.execPath, join(root, "scripts/evaluation-index-fixture.mjs")] } };
  const model = option("--model", undefined);
  if (model) settings.models[backend] = model;
  writeFileSync(join(data, "settings.json"), JSON.stringify(settings, null, 2));
  const env = { ...process.env, LORE_HOME: data };
  // The evaluator can itself be started from a recorder-marked environment;
  // enqueue is explicit and doesn't execute or register a global hook.
  process.stdout.write(`${fixture.id} ... `);
  let error;
  try { run(["enqueue", "claude", transcript], env); run(["work", "--once"], env); } catch (caught) { error = caught.message; }
  const status = run(["status"], env);
  const files = [...notesIn(join(knowledge, "common")), ...notesIn(join(knowledge, "projects"))];
  const reviewArtifacts = existsSync(join(data, "reviews")) ? readdirSync(join(data, "reviews")) : [];
  const texts = files.map((file) => written(readFileSync(file, "utf8")));
  const checks = {
    processed: !error && !/"failed": [1-9]|"waiting": [1-9]|"ready": [1-9]/.test(status),
    ...(fixture.notes ? { notes: files.length > 0 } : {}),
    // Notes follow the language the user wrote in, whatever language the prompt is in.
    ...(fixture.language === "ko" ? { korean: texts.every((text) => korean.test(text)) } : {}),
    ...(fixture.language === "en" ? { english: texts.every((text) => !korean.test(text)) } : {}),
    ...(fixture.leavesOut ? { leavesOut: fixture.leavesOut.filter((word) => texts.some((text) => text.toLowerCase().includes(word.toLowerCase()))) } : {}),
    ...(fixture.undatedTitles ? { undatedTitles: files.every((file) => !/\d{4}-\d{2}-\d{2}|\d{1,2}\/\d{1,2}/.test(titleOf(readFileSync(file, "utf8")))) } : {}),
    ...(fixture.noNotes ? { noNotes: files.length === 0 } : {}),
    ...(fixture.noCommon ? { noCommon: notesIn(join(knowledge, "common")).length === 0 } : {}),
    ...(fixture.noModel ? { noModel: reviewArtifacts.length === 0 } : {}),
  };
  reports.push({ id: fixture.id, purpose: fixture.purpose, expectation: fixture.expectation, checks, error, status,
    notes: files.map((file) => ({ file, text: readFileSync(file, "utf8") })), semanticReview: "pending" });
  writeFileSync(join(output, "report.json"), JSON.stringify({ backend, model: model ?? "default", indexing: "fixture; real qmd retrieval not tested", cases: reports }, null, 2));
  const failed = Object.entries(checks).filter(([, value]) => Array.isArray(value) ? value.length : !value).map(([name, value]) => Array.isArray(value) ? `${name}: ${value.join(", ")}` : name);
  process.stdout.write(failed.length ? `needs a look (${failed.join("; ")})\n` : "processed (read the notes to judge the content)\n");
}
process.stdout.write(`Report: ${join(output, "report.json")}\n`);
if (reports.some((report) => !Object.values(report.checks).every(Boolean))) process.exitCode = 1;
