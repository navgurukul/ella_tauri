/**
 * Signed self-update, applied at the only safe moment.
 *
 * Installing an update closes the app, so this runs once at launch — before a
 * conversation exists to interrupt — and never again during the session. A
 * learner who leaves Ella open for an afternoon gets the new version the next
 * time they open her, which is the trade every desktop app makes.
 *
 * Everything here fails soft. No network, a release that is still uploading, a
 * signature that does not verify: the current version keeps working and the
 * failure is reported to the caller rather than shown as an error the learner
 * cannot act on.
 */
import { relaunch } from "@tauri-apps/plugin-process";
import { check, type Update } from "@tauri-apps/plugin-updater";

export interface UpdateProgress {
  /** What to tell the learner, or null when nothing is happening. */
  stage: "checking" | "downloading" | "installing" | "restarting";
  version?: string;
  downloadedBytes: number;
  totalBytes: number;
}

/**
 * Checks, downloads, installs and restarts into the new version.
 *
 * Resolves `false` when there was nothing to do — the common case — so the
 * caller can carry on without a flash of update UI.
 */
export async function applyUpdateIfAny(
  report: (progress: UpdateProgress | null) => void,
): Promise<boolean> {
  // The browser preview has no updater to ask, and asking throws.
  if (typeof window === "undefined" || !("__TAURI_INTERNALS__" in window)) {
    report(null);
    return false;
  }

  let update: Update | null = null;
  try {
    report({ stage: "checking", downloadedBytes: 0, totalBytes: 0 });
    update = await check();
  } catch (reason) {
    // An unreachable release feed is not a reason to withhold the app.
    console.warn("[update] check failed", reason);
    report(null);
    return false;
  }

  if (!update) {
    report(null);
    return false;
  }

  try {
    let downloaded = 0;
    let total = 0;
    await update.downloadAndInstall((event) => {
      if (event.event === "Started") {
        total = event.data.contentLength ?? 0;
        report({
          stage: "downloading",
          version: update?.version,
          downloadedBytes: 0,
          totalBytes: total,
        });
      } else if (event.event === "Progress") {
        downloaded += event.data.chunkLength;
        report({
          stage: "downloading",
          version: update?.version,
          downloadedBytes: downloaded,
          totalBytes: total,
        });
      } else if (event.event === "Finished") {
        report({
          stage: "installing",
          version: update?.version,
          downloadedBytes: total,
          totalBytes: total,
        });
      }
    });
    report({
      stage: "restarting",
      version: update.version,
      downloadedBytes: total,
      totalBytes: total,
    });
    await relaunch();
    return true;
  } catch (reason) {
    // A half-applied update leaves the installed version intact; Tauri only
    // swaps it once the download verifies against the public key.
    console.warn("[update] install failed", reason);
    report(null);
    return false;
  }
}
