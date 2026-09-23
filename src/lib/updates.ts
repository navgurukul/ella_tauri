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
 */
import { getCurrentWindow } from "@tauri-apps/api/window";
import { relaunch } from "@tauri-apps/plugin-process";
import { check, type Update } from "@tauri-apps/plugin-updater";

export interface UpdateProgress {
  stage: "downloading" | "ready";
  version: string;
  downloadedBytes: number;
  totalBytes: number;
}

/** Applies a downloaded update now: installs and restarts into it. */
export type ApplyUpdate = () => Promise<void>;

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
  try {
    update = await check();
  } catch (reason) {
    // An unreachable release feed is not a reason to show anything.
    console.warn("[update] check failed", reason);
    return null;
  }
  if (!update) return null;

  const version = update.version;
  let downloaded = 0;
  let total = 0;
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

    if (!installExitsTheApp()) {
      await update.install();
    }
  } catch (reason) {
    // Tauri only swaps the installed app once the download verifies against
    // the public key, so a failure here leaves the current version intact.
    console.warn("[update] download or install failed", reason);
    report(null);
    return null;
  }

  const ready = update;
  let applied = false;
  const apply: ApplyUpdate = async () => {
    if (applied) return;
    applied = true;
    if (installExitsTheApp()) {
      // Runs the installer and exits; the installer relaunches Ella itself.
      await ready.install();
    } else {
      await relaunch();
    }
  };

  if (installExitsTheApp()) {
    // A learner who never presses Restart still gets the update, installed as
    // the window closes. A failure must not trap the window open.
    void getCurrentWindow().onCloseRequested(async () => {
      try {
        await apply();
      } catch (reason) {
        console.warn("[update] install on close failed", reason);
      }
    });
  }

  report({ stage: "ready", version, downloadedBytes: total, totalBytes: total });
  return apply;
}
