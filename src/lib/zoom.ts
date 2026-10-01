import { invoke, isTauri } from "@tauri-apps/api/core";

/**
 * Sends the page's size to the backend when it loads and whenever it changes,
 * so a window smaller than the design zooms the page out to fit (see
 * `window_fit.rs`). Only the page knows its real size: on a Mac the window's
 * inner size counts the title bar too.
 */
export function fitPageZoom(): void {
  if (!isTauri()) return;
  let frame = 0;
  const report = () => {
    cancelAnimationFrame(frame);
    frame = requestAnimationFrame(() => {
      invoke("fit_page_zoom", { width: window.innerWidth, height: window.innerHeight }).catch(() => {});
    });
  };
  window.addEventListener("resize", report);
  report();
}
