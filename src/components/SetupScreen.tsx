import { useEffect, useRef, useState, type ReactNode } from "react";
import { Hourglass, LoaderCircle, RotateCcw, WifiOff } from "lucide-react";
import { EllaGlyph, EllaMascot } from "./EllaMascot";
import { formatBytes, type SetupState } from "../lib/setup";

/**
 * The only thing on screen from the moment Ella opens until she can talk.
 *
 * It is the boot screen as well, so an ordinary start looks as it always has
 * — "Waking Ella up…" until the model is loaded — while the first launch
 * after an install stays on it through the whole download and the first model
 * load. There is deliberately no way past it: a name typed or a talk started
 * before Ella is ready would only meet an error. The learner's one action is
 * "Try again", and only once something has failed.
 */
export function SetupScreen({
  setup,
  booted,
  bootError,
  learnerName,
  retrying,
  onRetry,
  onOpen,
}: {
  /** Null until the backend has said where setup is. */
  setup: SetupState | null;
  /** The learner snapshot has loaded, so the app has somewhere to go. */
  booted: boolean;
  /** The snapshot could not load at all. */
  bootError: string | null;
  /** Whoever this laptop keeps, signed in or not: they are greeted by name. */
  learnerName: string | null;
  retrying: boolean;
  onRetry: () => void;
  /** Lets the learner in. Called once, when Ella is ready. */
  onOpen: () => void;
}) {
  const now = useClock();
  const download = useDownloadPace(setup, now);
  // Once the learner has seen anything but the ordinary "waking up", Ella's
  // readiness is worth a moment of its own before the app takes over.
  const shownSetup = useRef(false);
  const view = viewFor(setup, bootError, shownSetup.current);
  if (view !== "waking" && view !== "boot-error") shownSetup.current = true;
  const since = useSince(view, now);

  const open = useRef(onOpen);
  open.current = onOpen;
  const ready = setup?.stage === "ready";
  useEffect(() => {
    if (!booted || !ready || bootError) return;
    if (!shownSetup.current) {
      open.current();
      return;
    }
    // Long enough to read "ready" before the app takes over.
    const timer = window.setTimeout(() => open.current(), 900);
    return () => window.clearTimeout(timer);
  }, [booted, ready, bootError]);

  const copy = copyFor(view, setup, learnerName, since, download);

  return (
    <main className="boot gate" data-screen="setup" data-view={view}>
      <div className="gate__column">
        <div className="wordmark wordmark--lg">
          <EllaGlyph size={48} />
          <span>Ella</span>
        </div>

        {/* Keyed by the view, so the words fade in afresh only when the story
            changes, never on a progress tick. */}
        <div className="gate__copy" key={view}>
          <h1 className="display gate__title">{copy.title}</h1>
          <p className="gate__body">{copy.body}</p>
        </div>
        <p className="sr-only" role="status">
          {copy.title}
        </p>

        {copy.card}

        {(view === "failed-download" || view === "failed-load") && (
          <>
            <button
              className="btn btn--violet gate__retry"
              autoFocus
              aria-disabled={retrying || undefined}
              onClick={() => {
                if (!retrying) onRetry();
              }}
            >
              {retrying ? (
                <>
                  <LoaderCircle className="spin" size={18} aria-hidden="true" /> Starting again…
                </>
              ) : (
                <>
                  <RotateCcw size={18} aria-hidden="true" /> Try again
                </>
              )}
            </button>
            {setup?.message && (
              <details className="gate__details">
                <summary>Details</summary>
                <pre>{setup.message}</pre>
              </details>
            )}
          </>
        )}

        {view === "waking" && <LoaderCircle className="spin gate__spinner" aria-hidden="true" />}
      </div>

      <EllaMascot
        variant="corner"
        className={`ella--corner-boot ${view === "ready" ? "is-peeking" : ""}`.trim()}
        decorative
      />
    </main>
  );
}

export type GateView = "boot-error" | "waking" | "downloading" | "loading" | "ready" | "failed-download" | "failed-load";

/**
 * Which story the screen tells. A load that is not the end of an install is
 * just Ella waking up, as on any launch, and so is "ready" when nothing else
 * was ever shown: that start goes straight in.
 */
export function viewFor(setup: SetupState | null, bootError: string | null, shownSetup: boolean): GateView {
  if (bootError) return "boot-error";
  if (!setup) return "waking";
  switch (setup.stage) {
    case "downloading":
      return "downloading";
    case "loading":
      return setup.first_run && !setup.fell_back ? "loading" : "waking";
    case "ready":
      return shownSetup ? "ready" : "waking";
    case "failed":
      return setup.failure === "download" ? "failed-download" : "failed-load";
  }
}

type DownloadTrouble = NonNullable<SetupState["trouble"]>;

/**
 * What stopped a download. The backend says, from the error itself; the
 * message text is only read when it did not.
 */
export function troubleFor(setup: Pick<SetupState, "trouble" | "message">): DownloadTrouble {
  if (setup.trouble) return setup.trouble;
  const message = setup.message;
  if (/os error (28|39|112)\b|no space|not enough space|disk (is )?full/i.test(message)) return "disk";
  if (/checksum/i.test(message)) return "broken";
  if (/\b(408|429|50[0-9])\b|too many requests/i.test(message)) return "busy";
  if (/\bHTTP 4\d\d\b/.test(message)) return "blocked";
  return "network";
}

/** A title, one short line, and at most a progress card: nothing more. */
interface Copy {
  title: string;
  body: string;
  card?: ReactNode;
}

function copyFor(
  view: GateView,
  setup: SetupState | null,
  learnerName: string | null,
  since: number,
  download: DownloadPace,
): Copy {
  switch (view) {
    case "boot-error":
      return { title: "Ella could not start", body: "Close Ella and open her again." };

    case "waking":
      return {
        title: "Waking Ella up…",
        body: setup?.fell_back
          ? "Her update will wait for next time."
          : since >= 60_000
            ? "This can take a few minutes."
            : since >= 15_000
              ? "Almost there…"
              : learnerName
                ? `Hi ${learnerName}! She’ll be ready in a moment.`
                : "She’ll be ready in a moment.",
      };

    case "downloading":
      return {
        title: "Getting Ella ready",
        // Someone Ella already knows is downloading again: an update.
        body: learnerName ? `Hi ${learnerName}! Ella needs an update first.` : "This happens only once. Keep Ella open.",
        card: setup && <DownloadCard setup={setup} pace={download} />,
      };

    case "loading":
      return {
        title: "Almost ready!",
        body: since >= 120_000 ? "Still waking up. Please keep Ella open." : "Her first wake-up can take a few minutes.",
        card: (
          <div className="gate-card">
            <div className="gate-bar gate-bar--sweep" role="progressbar" aria-label="Ella is waking up">
              <span />
            </div>
          </div>
        ),
      };

    case "ready":
      return {
        title: "Ella is ready!",
        body: learnerName ? `Let’s talk, ${learnerName}!` : "Let’s say hello.",
        card: (
          <div className="gate-card">
            <div
              className="gate-bar gate-bar--done"
              role="progressbar"
              aria-label="Setup"
              aria-valuemin={0}
              aria-valuemax={100}
              aria-valuenow={100}
            >
              <span style={{ width: "100%" }} />
            </div>
          </div>
        ),
      };

    case "failed-download": {
      const trouble = troubleFor(setup ?? { trouble: null, message: "" });
      const saved = setup && trouble !== "broken" ? <SavedCard setup={setup} /> : null;
      const left = setup && setup.total_bytes > 0 ? setup.total_bytes - setup.downloaded_bytes : 0;
      switch (trouble) {
        case "disk":
          return {
            title: "This laptop is full",
            body:
              left > 0
                ? `Free up about ${formatBytes(left + 512 * 1024 ** 2)}, then try again.`
                : "Free up some space, then try again.",
            card: saved,
          };
        case "busy":
          return { title: "The download is busy", body: "Wait a few minutes, then try again.", card: saved };
        case "broken":
          return { title: "The download got mixed up", body: "Try again to fetch that part once more." };
        case "blocked":
          return { title: "This network blocks Ella", body: "Try another Wi-Fi or a phone hotspot.", card: saved };
        case "network":
          return {
            title: setup && setup.downloaded_bytes > 0 ? "The download paused" : "Ella can’t reach the internet",
            body: "Check your Wi-Fi, then try again.",
            card: saved,
          };
      }
      break;
    }

    case "failed-load":
      return { title: "Ella couldn’t wake up", body: "Close other apps, then try again." };
  }
  return { title: "Waking Ella up…", body: "She’ll be ready in a moment." };
}

/** Whole percent of the download, held under 100 until it is really done. */
function percentOf(setup: SetupState): number | null {
  if (setup.total_bytes <= 0) return null;
  return Math.max(0, Math.min(99, Math.floor((setup.downloaded_bytes / setup.total_bytes) * 100)));
}

function DownloadCard({ setup, pace }: { setup: SetupState; pace: DownloadPace }) {
  const percent = percentOf(setup);
  const tone = pace.state === "flowing" ? "" : "gate-bar--waiting";
  return (
    <div className="gate-card">
      <div className="gate-card__head">
        <span className="gate-card__percent">{percent === null ? "…" : `${percent}%`}</span>
        {setup.total_bytes > 0 && (
          <span className="gate-card__amount">{amountOf(setup)}</span>
        )}
      </div>
      <div
        className={`gate-bar ${tone}`.trim()}
        role="progressbar"
        aria-label="Download"
        aria-valuemin={0}
        aria-valuemax={100}
        aria-valuenow={percent ?? undefined}
        aria-valuetext={setup.total_bytes > 0 ? amountOf(setup) : undefined}
      >
        <span style={{ width: `${percent ?? 0}%` }} />
      </div>
      <p className={`gate-card__meta ${pace.state === "flowing" ? "" : "gate-card__meta--waiting"}`.trim()}>
        {pace.state === "reconnecting" ? (
          <>
            <RotateCcw size={16} aria-hidden="true" />{" "}
            {setup.trouble === "busy"
              ? "Server busy. Trying again…"
              : setup.trouble === "network" || setup.trouble === null
                ? "Connection dropped. Trying again…"
                : "Trying again…"}
          </>
        ) : pace.state === "stalled" ? (
          <>
            <WifiOff size={16} aria-hidden="true" /> Waiting for the internet…
          </>
        ) : (
          <>
            <Hourglass size={16} aria-hidden="true" /> {timeLeft(pace.secondsLeft)}
          </>
        )}
      </p>
    </div>
  );
}

/** A stopped download's bar, frozen where it stopped, with what is kept. */
function SavedCard({ setup }: { setup: SetupState }) {
  const percent = percentOf(setup);
  if (percent === null || setup.downloaded_bytes <= 0) return null;
  return (
    <div className="gate-card">
      <div className="gate-card__head">
        <span className="gate-card__percent gate-card__percent--paused">{percent}%</span>
        <span className="gate-card__amount">{amountOf(setup)} saved</span>
      </div>
      <div
        className="gate-bar gate-bar--paused"
        role="progressbar"
        aria-label="Download, paused"
        aria-valuemin={0}
        aria-valuemax={100}
        aria-valuenow={percent}
      >
        <span style={{ width: `${percent}%` }} />
      </div>
    </div>
  );
}

/** "1.3 of 2.3 GB": the unit once, where both sides share it. */
export function amountOf(setup: Pick<SetupState, "downloaded_bytes" | "total_bytes">): string {
  const done = formatBytes(setup.downloaded_bytes);
  const total = formatBytes(setup.total_bytes);
  const [doneValue, doneUnit] = done.split(" ");
  const [, totalUnit] = total.split(" ");
  return doneUnit === totalUnit ? `${doneValue} of ${total}` : `${done} of ${total}`;
}

/** "About 12 minutes left", from the recent pace; honest when it cannot tell. */
export function timeLeft(seconds: number | null): string {
  if (seconds === null) return "Working out the time left…";
  if (seconds < 60) return "Less than a minute left";
  const minutes = Math.ceil(seconds / 60);
  if (minutes < 90) return minutes === 1 ? "About 1 minute left" : `About ${minutes} minutes left`;
  const hours = Math.round(minutes / 60);
  return hours === 1 ? "About 1 hour left" : `About ${hours} hours left`;
}

interface DownloadPace {
  state: "flowing" | "reconnecting" | "stalled";
  secondsLeft: number | null;
}

/** No new bytes for this long reads as waiting for the internet. */
const STALL_MS = 30_000;
/** The pace is judged over this much recent history. */
const PACE_WINDOW_MS = 30_000;
/** And not before there is this much of it. */
const PACE_MIN_MS = 8_000;
/** A moving download reports every second; a longer gap was a pause, which
 * the pace should not average in. */
const PAUSE_MS = 5_000;

/**
 * How the download is going, from the byte counts as they arrive: the time
 * left at the recent pace, or that it has stopped moving. The backend retries
 * by itself; this only says so, so a paused bar never looks like a hang.
 */
function useDownloadPace(setup: SetupState | null, now: number): DownloadPace {
  const samples = useRef<Array<{ at: number; bytes: number }>>([]);
  const attempt = useRef(1);
  const grewAt = useRef(0);
  // Whether bytes have moved since the samples were last reset. Until they
  // do, the only sample is from before the connection opened (the start, or
  // a retry and its pause), and measuring from it would count the waiting
  // as downloading: "About 6 hours left" the moment a reconnect lands.
  const flowing = useRef(false);
  const downloading = setup?.stage === "downloading";
  const bytes = setup?.downloaded_bytes ?? 0;
  const tries = setup?.attempt ?? 1;

  useEffect(() => {
    if (!downloading) {
      samples.current = [];
      flowing.current = false;
      return;
    }
    const at = Date.now();
    let last: { at: number; bytes: number } | undefined = samples.current[samples.current.length - 1];
    // A new attempt, or a restart from zero, makes the old pace meaningless.
    if (tries !== attempt.current || (last && bytes < last.bytes)) {
      samples.current = [];
      flowing.current = false;
      attempt.current = tries;
      last = undefined;
    }
    if (!last || bytes > last.bytes) grewAt.current = at;
    if (last && bytes > last.bytes && (!flowing.current || at - last.at > PAUSE_MS)) {
      // The first bytes after a wait: the pace is measured from here.
      samples.current = [];
      flowing.current = true;
    }
    samples.current.push({ at, bytes });
    // Recent history only, but never fewer than the two newest samples, so
    // a slow link with long gaps between reports still has a pace.
    const recent = samples.current.filter((sample) => at - sample.at <= PACE_WINDOW_MS);
    samples.current = recent.length >= 2 ? recent : samples.current.slice(-2);
  }, [downloading, bytes, tries]);

  if (!setup || !downloading) return { state: "flowing", secondsLeft: null };

  const recent = samples.current;
  const first = recent[0];
  const last = recent[recent.length - 1];
  if (setup.attempt > 1 && !flowing.current) return { state: "reconnecting", secondsLeft: null };
  if (grewAt.current > 0 && now - grewAt.current > STALL_MS) return { state: "stalled", secondsLeft: null };

  let secondsLeft: number | null = null;
  if (first && last && last.at - first.at >= PACE_MIN_MS && last.bytes > first.bytes && setup.total_bytes > 0) {
    const perSecond = (last.bytes - first.bytes) / ((last.at - first.at) / 1000);
    secondsLeft = Math.max(0, (setup.total_bytes - setup.downloaded_bytes) / perSecond);
  }
  return { state: "flowing", secondsLeft };
}

/** The time, once a second, for copy that changes the longer a wait runs. */
function useClock(): number {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    const timer = window.setInterval(() => setNow(Date.now()), 1000);
    return () => window.clearInterval(timer);
  }, []);
  return now;
}

/** How long the screen has been telling its current story. */
function useSince(view: GateView, now: number): number {
  const started = useRef({ view, at: Date.now() });
  if (started.current.view !== view) started.current = { view, at: Date.now() };
  return Math.max(0, now - started.current.at);
}
