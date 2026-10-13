#!/usr/bin/env node
/**
 * The GitHub release's body for a version, which the release workflow hands to
 * tauri-action. tauri-action also writes it into the release's latest.json as
 * `notes`, which the updater gives the app (`Update.body`), and the update card
 * shows the bullets under its "What's new" heading (`whatsNewFrom` in
 * src/lib/whatsNew.ts). So what the learner reads there comes from
 * shared/whats-new.json, written for them: what changed for them, in plain
 * words, never how.
 *
 *   node scripts/release-notes.mjs [version]   # default: package.json's
 */
import { readFileSync } from "node:fs";

const root = new URL("../", import.meta.url);
const read = (path) => JSON.parse(readFileSync(new URL(path, root), "utf8"));
const version = process.argv[2] ?? read("package.json").version;
const notes = read("shared/whats-new.json")[version] ?? [];

const lines = [];
if (notes.length > 0) {
  lines.push(`## What's new in Ella ${version}`, "", ...notes.map((note) => `- ${note}`), "");
}
lines.push(
  "## Installing",
  "",
  "Windows installer (`-setup.exe`) and Mac apps for Ella: `aarch64.dmg`",
  "for Apple Silicon, `x64.dmg` for Intel Macs.",
  "",
  "On first launch Ella downloads about 2.3 GB of speech and language",
  "models, once, resuming if the connection drops. Keep about 3 GB free.",
  "",
  "Neither build is code-signed, so the first launch is blocked once:",
  "- Windows: on the SmartScreen warning, choose **More info** then **Run anyway**.",
  "- Mac: open the app once, then choose **Open Anyway** in System Settings → Privacy & Security.",
);
console.log(lines.join("\n"));
