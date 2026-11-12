import { spawnSync } from "node:child_process";
import { mkdirSync, readFileSync, writeFileSync, readdirSync, existsSync } from "node:fs";
import { dirname, join, relative, resolve } from "node:path";
import { fileURLToPath } from "node:url";

// Runs each cleanup case against the real recording model and checks what the cleanup left on
// disk. The checks catch what can be read mechanically (which notes are gone, which words
// survived); whether the remaining notes say the right thing is read by a person in the report.
const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const args = process.argv.slice(2);
const option = (name, fallback) => args.includes(name) ? args[args.indexOf(name) + 1] : fallback;
const output = resolve(option("--output", join(root, "work-cleanup-evaluation", new Date().toISOString().replaceAll(":", "-"))));
const backend = option("--backend", "codex");
if (!["codex", "claude"].includes(backend)) throw new Error("--backend must be codex or claude.");
const cli = join(root, "target", "release", `lore-cli${process.platform === "win32" ? ".exe" : ""}`);
if (!existsSync(cli)) throw new Error("Run pnpm build:cli first.");
const only = option("--only", "");
const cases = JSON.parse(readFileSync(join(root, "examples/evaluation/cleanup-cases.json"), "utf8")).filter((fixture) => !only || only.split(",").includes(fixture.id));
const reports = [];
const korean = /[가-힣]/;
// What the model wrote: the title and body, without the front matter and the program's source list.
const written = (text) => text.replace(/^---\n[\s\S]*?\n---\n/, "").split("\n## Sources\n")[0];

function noteText({ path, project, kind, authority, title, body, sources }) {
  const scope = path.startsWith("common/") ? "common" : project ?? "demo";
  const list = sources?.length ? `\n## Sources\n${sources.join("\n")}\n` : "";
  return `---\nqmd:\n  metadata:\n    project: "${scope}"\n    kind: "${kind}"\n    authority: "${authority}"\n---\n\n# ${title}\n\n${body}\n${list}`;
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
  if (existsSync(data)) throw new Error(`Evaluation data already exists and is not overwritten: ${data}`);
  mkdirSync(data, { recursive: true });
  const before = {};
  for (const note of [...(fixture.shared ?? []), ...fixture.notes]) {
    mkdirSync(dirname(join(knowledge, note.path)), { recursive: true });
    before[note.path] = noteText(note);
    writeFileSync(join(knowledge, note.path), before[note.path]);
  }
  const settings = { knowledgeRoot: knowledge, backend, modelTimeoutSecs: 600, models: {}, commands: { qmd: [process.execPath, join(root, "scripts/evaluation-index-fixture.mjs")] } };
  const model = option("--model", undefined);
  if (model) settings.models[backend] = model;
  writeFileSync(join(data, "settings.json"), JSON.stringify(settings, null, 2));
  process.stdout.write(`${fixture.id} ... `);
  const command = ["cleanup", "demo", ...(fixture.absorb ?? []).flatMap((name) => ["--absorb", name])];
  const result = spawnSync(cli, command, { env: { ...process.env, LORE_HOME: data }, encoding: "utf8", timeout: 900_000 });
  const error = result.error?.message ?? (result.status !== 0 ? result.stderr || result.stdout : undefined);
  let outcome;
  try { outcome = JSON.parse(result.stdout.slice(result.stdout.indexOf("{"))); } catch { outcome = undefined; }

  const after = Object.fromEntries(notesIn(knowledge).map((file) => [relative(knowledge, file).replaceAll("\\", "/"), readFileSync(file, "utf8")]));
  const within = (prefix) => Object.entries(after).filter(([path]) => path.startsWith(prefix));
  const texts = Object.values(after).map(written);
  // An array check lists what failed, so an empty array passes.
  const checks = {
    processed: !error && !outcome?.incomplete,
    ...(fixture.noChange ? { noChange: [...new Set([...Object.keys(before), ...Object.keys(after)])].filter((path) => after[path] !== before[path]) } : {}),
    ...(fixture.removes ? { removes: fixture.removes.filter((path) => path in after) } : {}),
    ...(fixture.keeps ? { keeps: fixture.keeps.filter((path) => !(path in after)) } : {}),
    ...(fixture.unchanged ? { unchanged: fixture.unchanged.filter((path) => after[path] !== before[path]) } : {}),
    ...(fixture.count ? { count: Object.entries(fixture.count).filter(([prefix, wanted]) => within(prefix).length !== wanted).map(([prefix, wanted]) => `${prefix} has ${within(prefix).length}, wanted ${wanted}`) } : {}),
    ...(fixture.mentions ? { mentions: fixture.mentions.filter((item) => !within(item.in).some(([, text]) => written(text).includes(item.text))).map((item) => `${item.in}: ${item.text}`) } : {}),
    ...(fixture.matches ? { matches: fixture.matches.filter((item) => !within(item.in).some(([, text]) => new RegExp(item.pattern).test(written(text)))).map((item) => `${item.in}: ${item.pattern}`) } : {}),
    // Source lists are carried by the program, so these are looked for in the whole note.
    ...(fixture.carries ? { carries: fixture.carries.filter((item) => !within(item.in).some(([, text]) => text.includes(item.text))).map((item) => `${item.in}: ${item.text}`) } : {}),
    ...(fixture.leavesOut ? { leavesOut: fixture.leavesOut.filter((word) => Object.values(after).some((text) => text.toLowerCase().includes(word.toLowerCase()))) } : {}),
    ...(fixture.korean ? { korean: texts.every((text) => korean.test(text)) } : {}),
    ...(fixture.english ? { english: texts.every((text) => !korean.test(text)) } : {}),
  };
  reports.push({ id: fixture.id, purpose: fixture.purpose, expectation: fixture.expectation, checks, error, outcome, before, after, semanticReview: "pending" });
  writeFileSync(join(output, "report.json"), JSON.stringify({ backend, model: model ?? "default", indexing: "fixture; real qmd retrieval not tested", cases: reports }, null, 2));
  const failed = Object.entries(checks).filter(([, value]) => Array.isArray(value) ? value.length : !value).map(([name, value]) => Array.isArray(value) ? `${name}: ${value.join(", ")}` : name);
  process.stdout.write(failed.length ? `needs a look (${failed.join("; ")})\n` : "passed the checks (read the notes to judge the content)\n");
}
process.stdout.write(`Report: ${join(output, "report.json")}\n`);
if (reports.some((report) => Object.values(report.checks).some((value) => Array.isArray(value) ? value.length : !value))) process.exitCode = 1;
