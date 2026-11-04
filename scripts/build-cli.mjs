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

execFileSync("cargo", ["build", "--release", "-p", "qmd-memory-cli"], { cwd: root, stdio: "inherit" });

const extension = host.includes("windows") ? ".exe" : "";
const binaries = join(root, "src-tauri", "binaries");
mkdirSync(binaries, { recursive: true });
copyFileSync(join(root, "target", "release", `qmd-memory-cli${extension}`), join(binaries, `qmd-memory-cli-${host}${extension}`));
