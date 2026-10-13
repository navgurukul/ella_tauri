/**
 * Signed self-update, downloaded in the background while Ella stays usable.
 *
 * The check and the download run behind the app, so a learner can talk the
 * whole time. What differs by platform is the install, because installing is
 * the one step that can interrupt:
 *
 * - macOS swaps the new app bundle into place and returns; the running copy is
 *   untouched, so it installs the moment the download verifies, and the new
 *   version starts next launch (or on Restart).
 * - Windows runs the installer and exits the app on the spot, so it must never
 *   happen mid-conversation. It waits for the learner to press Restart, or for
 *   the window to close — the moment closing costs nothing.
 *
 * Everything here fails soft. No network, a release that is still uploading, a
 * signature that does not verify: the current version keeps working and the
 * toast simply never appears, because there is nothing the learner can act on.
 *
 * Each step is kept in latency.jsonl as an `ella_update` event, because a
 * learner who never sees the toast cannot say what went wrong: on 2026-10-12 a
 * tester ran 0.1.16 for ten launches with 0.1.19 out, and nothing showed why.
 */
import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { relaunch } from "@tauri-apps/plugin-process";
import { check, type Update } from "@tauri-apps/plugin-updater";

export interface UpdateProgress {
  stage: "downloading" | "ready";
  version: string;
  downloadedBytes: number;
  totalBytes: number;
}

/** Applies a downloaded update now: installs and restarts into it. On Windows,
 * `via` says what asked: the toast's Restart, or the window closing. */
export type ApplyUpdate = (via?: InstallVia) => Promise<void>;

export type InstallVia = "restart" | "close";

/** Mirrors `UPDATE_STEPS` in telemetry.rs. */
type UpdateStepName =
  | "up_to_date"
  | "found"
  | "check_failed"
  | "downloaded"
  | "download_failed"
  | "installed"
  | "installing"
  | "install_failed";

interface UpdateStep {
  step: UpdateStepName;
  version?: string;
  ms?: number;
  bytes?: number;
  via?: InstallVia;
  error?: string;
}

/** Keeps a step in latency.jsonl (`record_update`). Never throws: a step that
 * cannot be written must not stop the update it describes. */
async function record({ step, version, ms, bytes, via, error }: UpdateStep): Promise<void> {
  try {
    await invoke("record_update", {
      step,
      version: version ?? null,
      ms: ms === undefined ? null : Math.max(0, Math.round(ms)),
      bytes: bytes ?? null,
      via: via ?? null,
      error: error ?? null,
    });
  } catch (reason) {
    console.warn("[update] could not record a step", reason);
  }
}

function why(reason: unknown): string {
  return reason instanceof Error ? reason.message : String(reason);
}

/** At most `ms` of waiting for `task`, which carries on regardless. */
function atMost(task: Promise<void>, ms: number): Promise<void> {
  return Promise.race([task, new Promise<void>((resolve) => setTimeout(resolve, ms))]);
}

/** Windows installs by running the installer and exiting, so it waits for close. */
export const installExitsTheApp = () => navigator.userAgent.includes("Windows");

/**
 * Checks for an update and downloads it in the background, reporting progress
 * for the toast. Resolves with a function that applies the update now, or
 * `null` when there is nothing to apply.
 */
export async function downloadUpdateInBackground(
  report: (progress: UpdateProgress | null) => void,
): Promise<ApplyUpdate | null> {
  // The browser preview has no updater to ask, and asking throws.
  if (typeof window === "undefined" || !("__TAURI_INTERNALS__" in window)) return null;

  let update: Update | null;
  const checking = Date.now();
  try {
    update = await check();
  } catch (reason) {
    // An unreachable release feed is not a reason to show anything.
    console.warn("[update] check failed", reason);
    void record({ step: "check_failed", ms: Date.now() - checking, error: why(reason) });
    return null;
  }
  if (!update) {
    void record({ step: "up_to_date", ms: Date.now() - checking });
    return null;
  }

  const version = update.version;
  void record({ step: "found", version, ms: Date.now() - checking });
  let downloaded = 0;
  let total = 0;
  const downloading = Date.now();
  try {
    report({ stage: "downloading", version, downloadedBytes: 0, totalBytes: 0 });
    await update.download((event) => {
      if (event.event === "Started") {
        total = event.data.contentLength ?? 0;
      } else if (event.event === "Progress") {
        downloaded += event.data.chunkLength;
      }
      report({ stage: "downloading", version, downloadedBytes: downloaded, totalBytes: total });
    });
  } catch (reason) {
    // Tauri only swaps the installed app once the download verifies against
    // the public key, so a failure here leaves the current version intact.
    console.warn("[update] download or install failed", reason);
    void record({
      step: "download_failed",
      version,
      ms: Date.now() - downloading,
      bytes: downloaded,
      error: why(reason),
    });
    report(null);
    return null;
  }
  void record({ step: "downloaded", version, ms: Date.now() - downloading, bytes: downloaded || total });

  if (!installExitsTheApp()) {
    try {
      await update.install();
      void record({ step: "installed", version });
    } catch (reason) {
      console.warn("[update] download or install failed", reason);
      void record({ step: "install_failed", version, error: why(reason) });
      report(null);
      return null;
    }
  }

  const ready = update;
  let applied = false;
  const apply: ApplyUpdate = async (via = "restart") => {
    if (applied) return;
    applied = true;
    if (installExitsTheApp()) {
      // Written before the installer closes Ella, but never held up for long.
      await atMost(record({ step: "installing", version, via }), 1000);
      try {
        // Runs the installer and exits; the installer relaunches Ella itself.
        await ready.install();
      } catch (reason) {
        void record({ step: "install_failed", version, via, error: why(reason) });
        throw reason;
      }
    } else {
      await relaunch();
    }
  };

  if (installExitsTheApp()) {
    // A learner who never presses Restart still gets the update, installed as
    // the window closes. A failure must not trap the window open.
    void getCurrentWindow().onCloseRequested(async () => {
      try {
        await apply("close");
      } catch (reason) {
        console.warn("[update] install on close failed", reason);
      }
    });
  }

  report({ stage: "ready", version, downloadedBytes: total, totalBytes: total });
  return apply;
}
