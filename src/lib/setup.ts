/**
 * First-run setup, as the window sees it.
 *
 * The Rust side downloads ~2.3 GB of weights and then loads a 2 GB model
 * before Ella can say anything. That happens on a background thread with the
 * window already open, so this is how the screen learns where it has got to.
 */
import { useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";

const SETUP_EVENT = "ella://setup";

export interface SetupState {
  stage: "downloading" | "loading" | "ready" | "failed";
  message: string;
  downloaded_bytes: number;
  total_bytes: number;
  index: number;
  of: number;
  /** Above 1, the transfer dropped and is being retried. */
  attempt?: number;
}

/** Null until the backend says something, which on a warm start it never does. */
export function useSetupState(): SetupState | null {
  const [state, setState] = useState<SetupState | null>(null);

  useEffect(() => {
    if (typeof window === "undefined" || !("__TAURI_INTERNALS__" in window)) return;
    let stop: (() => void) | undefined;
    let live = true;
    void listen<SetupState>(SETUP_EVENT, (event) => setState(event.payload)).then((unlisten) => {
      // The listener can resolve after the component is gone on a fast reload.
      if (live) stop = unlisten;
      else unlisten();
    });
    return () => {
      live = false;
      stop?.();
    };
  }, []);

  return state;
}

/** "1.4 GB of 2.3 GB" — the only two numbers worth showing during a long wait. */
export function formatBytes(bytes: number): string {
  if (bytes >= 1024 ** 3) return `${(bytes / 1024 ** 3).toFixed(1)} GB`;
  if (bytes >= 1024 ** 2) return `${Math.round(bytes / 1024 ** 2)} MB`;
  return `${Math.max(0, Math.round(bytes / 1024))} KB`;
}
