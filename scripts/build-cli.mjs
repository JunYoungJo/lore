import { execFileSync } from "node:child_process";
import { copyFileSync, mkdirSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const host = execFileSync("rustc", ["-vV"], { encoding: "utf8" })
  .split("\n")
  .find((line) => line.startsWith("host:"))
  ?.slice("host:".length)
  .trim();
if (!host) throw new Error("rustc 대상 플랫폼을 알 수 없습니다.");

// Tauri exports the requested target to beforeBuildCommand. Build the sidecar
// for that target as well (including the two halves of a universal macOS app).
const target = process.env.TAURI_ENV_TARGET_TRIPLE || host;
const targets = target === "universal-apple-darwin"
  ? ["aarch64-apple-darwin", "x86_64-apple-darwin"] : [target];
const binaries = join(root, "src-tauri", "binaries");
mkdirSync(binaries, { recursive: true });

for (const triple of targets) {
  const args = ["build", "--release", "-p", "qmd-memory-cli"];
  if (triple !== host) args.push("--target", triple);
  execFileSync("cargo", args, { cwd: root, stdio: "inherit" });
  const extension = triple.includes("windows") ? ".exe" : "";
  const source = triple === host ? join(root, "target", "release") : join(root, "target", triple, "release");
  copyFileSync(join(source, `qmd-memory-cli${extension}`), join(binaries, `qmd-memory-cli-${triple}${extension}`));
}
if (target === "universal-apple-darwin") {
  execFileSync("lipo", ["-create", ...targets.map((triple) => join(binaries, `qmd-memory-cli-${triple}`)), "-output", join(binaries, `qmd-memory-cli-${target}`)]);
}
