import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import path from "node:path";

const projectDir = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const mode = process.argv[2];
if (mode !== "install" && mode !== "validate") {
  console.error("Usage: node scripts/models.mjs install|validate");
  process.exit(2);
}

const command = process.platform === "win32" ? "py" : "python3";
const args = process.platform === "win32" ? ["-3"] : [];
args.push(
  path.resolve(projectDir, "tooling/fetch_models.py"),
  "--dest",
  path.resolve(projectDir, "engines"),
  "--only",
  "stt,stt_fallback",
);
if (mode === "validate") args.push("--verify-only");

const result = spawnSync(command, args, { cwd: projectDir, stdio: "inherit" });
if (result.error) {
  console.error(
    `Could not start Python with ${command}: ${result.error.message}. ` +
      "Install Python 3 and ensure it is available on PATH.",
  );
  process.exit(1);
}
process.exit(result.status ?? 1);
