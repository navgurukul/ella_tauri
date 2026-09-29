/**
 * First-run setup, as the window sees it.
 *
 * The Rust side downloads ~2.3 GB of weights and then loads a 2 GB model
 * before Ella can say anything. That happens on a background thread with the
 * window already open, on the setup screen, and nobody gets past that screen
 * until it is done. So this is how the window learns where setup has got to.
 */
import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

const SETUP_EVENT = "ella://setup";

/** How often a window still waiting asks again, in case an event was lost. */
const POLL_MS = 3000;

export interface SetupState {
  stage: "downloading" | "loading" | "ready" | "failed";
  /** The backend's own words: what is happening, or what went wrong. */
  message: string;
  /** Across every file this launch fetches, so the bar only moves forward. */
  downloaded_bytes: number;
  total_bytes: number;
  /** Which file of how many. */
  index: number;
  of: number;
  /** Above 1, the transfer dropped and is being retried. */
  attempt: number;
  /** This launch had something to download, so it is the end of an install
   * rather than an ordinary start. */
  first_run: boolean;
  /** An update could not download, so Ella is loading the models she had. */
  fell_back: boolean;
  /** Which half of setup a failure happened in. */
  failure: "download" | "load" | null;
  /** Why a download is being retried, or why it stopped. */
  trouble: "network" | "busy" | "blocked" | "disk" | "broken" | null;
  /** Counts announcements; the higher of two is the newer. */
  seq: number;
}

/** What a window with no backend to wait for — a browser, a test — is at. */
const READY: SetupState = {
  stage: "ready",
  message: "",
  downloaded_bytes: 0,
  total_bytes: 0,
  index: 0,
  of: 0,
  attempt: 1,
  first_run: false,
  fell_back: false,
  failure: null,
  trouble: null,
  seq: 0,
};

function inTauri(): boolean {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}

export interface Setup {
  /** Null only until the backend has answered, which takes a moment. */
  state: SetupState | null;
  /** True from a "Try again" press until setup says anything newer. */
  retrying: boolean;
  retry: () => void;
}

export function useSetup(): Setup {
  const [state, setState] = useState<SetupState | null>(() => (inTauri() ? null : READY));
  // The announcement "Try again" was pressed on; anything newer ends the wait.
  const [retryingFrom, setRetryingFrom] = useState<number | null>(null);
  const mounted = useRef(true);

  /** Events and answers can arrive out of order; the higher number wins. */
  const keep = useCallback((next: SetupState) => {
    setState((current) => (current && current.seq >= next.seq ? current : next));
  }, []);

  useEffect(() => {
    mounted.current = true;
    if (!inTauri()) return;
    let active = true;
    let stop: (() => void) | undefined;
    // Listen first and only then ask, so no announcement can fall between
    // the two: one made before the question is in the answer, and one made
    // after it arrives as an event.
    listen<SetupState>(SETUP_EVENT, (event) => active && keep(event.payload))
      .then((unlisten) => {
        // The listener can resolve after the component is gone on a fast reload.
        if (!active) {
          unlisten();
          return;
        }
        stop = unlisten;
        return invoke<SetupState>("setup_state").then((answer) => active && keep(answer));
      })
      .catch((reason: unknown) => active && keep(unreachable(reason)));
    return () => {
      active = false;
      mounted.current = false;
      stop?.();
    };
  }, [keep]);

  // An event can still go missing (a reload at the wrong moment, a dropped
  // eval), and the one that goes missing may be the "ready" that lets the
  // learner in. So a window that is still waiting keeps asking, which costs
  // the backend a lock and a clone.
  const waiting = state !== null && state.stage !== "ready";
  useEffect(() => {
    if (!waiting || !inTauri()) return;
    let active = true;
    const timer = window.setInterval(() => {
      invoke<SetupState>("setup_state")
        .then((answer) => active && keep(answer))
        .catch(() => undefined);
    }, POLL_MS);
    return () => {
      active = false;
      window.clearInterval(timer);
    };
  }, [waiting, keep]);

  const retry = useCallback(() => {
    if (!state || state.stage !== "failed" || retryingFrom === state.seq) return;
    const pressedOn = state.seq;
    setRetryingFrom(pressedOn);
    invoke<SetupState>("retry_setup")
      .then((answer) => {
        if (!mounted.current) return;
        keep(answer);
        // A new run answers with the attempt it began. The same failure back
        // means the press was refused (one was already finishing), so the
        // button comes back rather than spinning on nothing.
        if (answer.seq <= pressedOn) setRetryingFrom(null);
      })
      .catch((reason: unknown) => {
        if (!mounted.current) return;
        setRetryingFrom(null);
        keep(unreachable(reason, pressedOn));
      });
  }, [keep, retryingFrom, state]);

  const retrying = state !== null && retryingFrom !== null && state.seq === retryingFrom;
  return { state, retrying, retry };
}

/**
 * The backend did not answer at all. That leaves nothing to go on, so the
 * screen stays shut, says so, and offers "Try again", which asks once more.
 */
function unreachable(reason: unknown, after = 0): SetupState {
  return {
    ...READY,
    stage: "failed",
    failure: "load",
    message: reason instanceof Error ? reason.message : String(reason),
    // Between two real announcements, so it can never outrank the next one.
    seq: Math.floor(after) + 0.5,
  };
}

/** "1.4 GB of 2.3 GB" — the only two numbers worth showing during a long wait. */
export function formatBytes(bytes: number): string {
  if (bytes >= 1024 ** 3) return `${(bytes / 1024 ** 3).toFixed(1)} GB`;
  if (bytes >= 1024 ** 2) return `${Math.round(bytes / 1024 ** 2)} MB`;
  return `${Math.max(0, Math.round(bytes / 1024))} KB`;
}
