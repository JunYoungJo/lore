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
if (!["codex", "claude"].includes(backend)) throw new Error("backend는 codex 또는 claude여야 합니다.");
if (existsSync(output)) throw new Error(`기존 평가 폴더를 덮어쓰지 않습니다: ${output}`);
mkdirSync(output, { recursive: true });
const cli = join(root, "target/release", `qmd-memory-cli${process.platform === "win32" ? ".exe" : ""}`);
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
  const base = "# 오류 알림\n\n## 현재 방침\n\n오류 안내 방식은 미정이다.\n\n## 변경하지 않는 사항\n\n장비의 측정 동작은 그대로 유지한다.\n\n모든 모드의 데이터 보존 정책도 유지한다.\n\n새로운 오류 유형을 추가하지 않는다.\n\n기존 문서의 다른 절은 보존한다.\n\n## 현장 운영\n\n현장 안내 방식은 미정이다.\n\n## 원문 근거\n- [Windows L1](<../../sources/claude/windows-session.md#L1>)\n- [Mac L1](<../../sources/codex/mac-session.md#L1>)\n";
  write(local, note, base);
  write(local, "sources/claude/windows-session.md", '# Windows 예제\n\n<a id="L1"></a>\n## L1 · 사용자 · 2026-11-10T01:00:00Z\n\n오류 팝업은 빼고 로그만 남겨. 현장 작업자의 조작을 방해하면 안 돼.\n');
  write(local, "sources/codex/mac-session.md", '# Mac 예제\n\n<a id="L1"></a>\n## L1 · 사용자 · 2026-11-10T02:00:00Z\n\n오류를 즉시 알아차려야 해. 오류가 나면 팝업을 띄워.\n');
  commit(local); git(local, "push", "-u", "origin", "main");
  git(folder, "clone", "remote.git", "other"); const other = join(folder, "other"); config(other);
  write(local, note, base.replace("오류 안내 방식은 미정이다.", "오류 팝업 없이 로그만 남긴다. 현장 조작을 방해하지 않기 위한 방침이다.")); commit(local);
  const remoteText = kind === "text-conflict" ? base.replace("오류 안내 방식은 미정이다.", "오류가 나면 팝업을 표시한다. 현장에서 즉시 오류를 알아차리기 위한 방침이다.") : base.replace("현장 안내 방식은 미정이다.", "오류가 나면 팝업을 항상 표시한다. 현장에서 즉시 오류를 알아차려야 한다.");
  write(other, note, remoteText); commit(other); git(other, "push");
  const data = join(folder, "app"); mkdirSync(data);
  writeFileSync(join(data, "settings.json"), JSON.stringify({ knowledgeRoot: local, backend, modelTimeoutSecs: 240, commands: { qmd: [process.execPath, join(root, "scripts/evaluation-index-fixture.mjs")] } }, null, 2));
  const env = { ...process.env, QMD_MEMORY_HOME: data };
  const command = (...argv) => JSON.parse(run(cli, ["sync", ...argv], local, env));
  const before = git(local, "rev-parse", "HEAD"); const beforeText = readFileSync(join(local, note), "utf8");
  const prepared = command("check");
  const analyzed = command("analyze");
  const plan = analyzed.plan;
  if (!plan.analysis.issues.length) throw new Error("모델이 상반된 방침의 검토 항목을 제시하지 않았습니다.");
  const choices = plan.analysis.issues.map((_, issue) => ({ issue, option: 0, customText: null }));
  const choicesPath = join(folder, "choices.json"); writeFileSync(choicesPath, JSON.stringify(choices));
  const preview = command("preview", choicesPath);
  const isolated = git(local, "rev-parse", "HEAD") === before && readFileSync(join(local, note), "utf8") === beforeText;
  const applied = command("apply", plan.id, choicesPath);
  const finalText = readFileSync(join(local, note), "utf8");
  git(other, "pull", "--ff-only");
  const checks = { isolatedBeforeApproval: isolated, completed: applied.plan === null, bothHistories: git(local, "show", "-s", "--format=%P").split(/\s+/).length === 2, exactPreview: finalText === preview[note], unchangedParagraph: finalText.includes("장비의 측정 동작은 그대로 유지한다."), bothEvidenceLinks: finalText.includes("windows-session.md#L1") && finalText.includes("mac-session.md#L1"), otherPcMatches: finalText === readFileSync(join(other, note), "utf8") };
  reports.push({ kind, backend, gitTextConflict: prepared.plan.files.some((file) => file.textConflict), analysis: plan.analysis, fixtureOnlyChoices: choices, finalText, checks, actualQmd: false });
  writeFileSync(join(output, "report.json"), JSON.stringify(reports, null, 2));
  process.stdout.write(`${Object.values(checks).every(Boolean) ? "통과" : "확인 필요"} · 모델 검토 ${plan.analysis.issues.length}건\n`);
  if (!Object.values(checks).every(Boolean)) process.exitCode = 1;
}
process.stdout.write(`결과: ${join(output, "report.json")}\n`);
