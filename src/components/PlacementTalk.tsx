import { useEffect, useRef, useState } from "react";
import { LoaderCircle } from "lucide-react";
import { EllaMascot, type EllaState } from "./EllaMascot";
import { MicGlyph } from "./HomeScreen";
import { TalkScreen } from "./TalkScreen";
import { bridge } from "../lib/bridge";
import { levelTone } from "../lib/curriculum";
import type { Assessment, Session, SessionSummary } from "../types";

/**
 * `starting` until the backend has opened the chat; `talking` through it;
 * `reading` while the level is read after Ella's goodbye; `placed` with the
 * result. `unstarted` and `unread` are the two ways it can fail, each with a
 * way to try again and a way past.
 */
type Phase = "starting" | "talking" | "reading" | "placed" | "unstarted" | "unread";

/**
 * The placement chat, as on Ella Mobile: a friendly first talk that climbs from
 * easy questions to harder ones until Ella has heard enough — five answers at
 * the soonest, twelve at the latest — then reads a level off it and starts the
 * learner at Step 1 of that level.
 *
 * The chat is an ordinary talk screen, with its voice, typing and replay. Its
 * last turn closes the session, and the level is read while Ella is still
 * saying goodbye, so the result is usually ready the moment she finishes.
 * Skip leaves at any point before that, and places nobody.
 */
export function PlacementTalk({
  greetName,
  onRead,
  onDone,
}: {
  greetName: string;
  /** The level has been read and kept. Told even when the learner skipped
   * past the wait and this screen is gone, so the app can catch up. */
  onRead: () => void;
  /** With the assessment once placed; with null when skipped. */
  onDone: (placed: Assessment | null) => void;
}) {
  const [phase, setPhase] = useState<Phase>("starting");
  const [session, setSession] = useState<Session | null>(null);
  const [assessment, setAssessment] = useState<Assessment | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [attempt, setAttempt] = useState(0);
  // Survive StrictMode's rehearsal unmount, so the chat is opened once.
  const started = useRef(-1);
  const mounted = useRef(false);
  const reading = useRef<Promise<Assessment> | null>(null);
  const closed = useRef(false);
  const read = useRef(onRead);
  read.current = onRead;

  useEffect(() => {
    mounted.current = true;
    if (started.current !== attempt) {
      started.current = attempt;
      bridge
        .startPlacement()
        .then((opened) => {
          if (!mounted.current) {
            // Left before it opened: close it rather than leave it waiting on
            // Home as an unfinished talk.
            void bridge.completeSession(opened.id).catch(() => undefined);
            return;
          }
          setSession(opened);
          setPhase("talking");
        })
        .catch((reason: unknown) => {
          if (!mounted.current) return;
          setError(message(reason));
          setPhase("unstarted");
        });
    }
    return () => {
      mounted.current = false;
    };
  }, [attempt]);

  function readLevel(sessionId: string): Promise<Assessment> {
    if (!reading.current) {
      reading.current = bridge.assessSession(sessionId);
      reading.current.then(
        () => read.current(),
        () => undefined,
      );
    }
    return reading.current;
  }

  /** The chat's last turn is in: read the level while Ella says goodbye. */
  function handleClosing(summary: SessionSummary) {
    closed.current = true;
    void readLevel(summary.session_id).catch(() => undefined);
  }

  /** Ella has finished — or the learner pressed Skip, which closed nothing. */
  async function handleComplete(summary: SessionSummary) {
    if (!closed.current) {
      onDone(null);
      return;
    }
    setPhase("reading");
    try {
      const result = await readLevel(summary.session_id);
      if (!mounted.current) return;
      setAssessment(result);
      setPhase("placed");
    } catch (reason) {
      reading.current = null;
      if (!mounted.current) return;
      setError(message(reason));
      setPhase("unread");
    }
  }

  function retry() {
    setError(null);
    if (phase === "unstarted") {
      setPhase("starting");
      setAttempt((count) => count + 1);
      return;
    }
    if (session) void handleComplete({ session_id: session.id } as SessionSummary);
  }

  if (phase === "talking" && session) {
    return (
      <div className="placement-frame" data-screen="onboarding-placement">
        <TalkScreen
          key={session.id}
          session={session}
          variant="placement"
          onSessionChange={setSession}
          onClosing={handleClosing}
          onComplete={(summary) => void handleComplete(summary)}
        />
      </div>
    );
  }

  const standing = assessment?.standing;
  const ella: EllaState = phase === "reading" || phase === "starting" ? "thinking" : "resting";

  return (
    <div className="screen screen--talk ob-placement" data-screen="onboarding-placement">
      <header className="talk-head">
        <span className="pill pill--white">
          First talk<span className="talk-head__tag">Your level</span>
        </span>
        {/* Reading the level can take a while on a slow laptop; leaving lets
            it finish in the background, and the level lands when it does. */}
        {phase === "reading" && (
          <button className="btn btn--quiet" onClick={() => onDone(null)}>
            Skip
          </button>
        )}
      </header>

      <div className="talk-stage">
        {phase === "placed" && standing ? (
          <>
            <p className="talk-prompt">{assessment?.closing}</p>
            <p className={`level-reveal ladder-${levelTone(standing.level_number)}`}>
              <span className="mono level-reveal__label">Your level</span>
              <strong className="display level-reveal__name">{standing.level_name}</strong>
            </p>
            <p className="level-reveal__sub">
              Level {standing.level_number} of {standing.level_count} · Step {standing.step}:{" "}
              {standing.step_title}
            </p>
            <button className="btn btn--green ob-placement__go" onClick={() => onDone(assessment)} autoFocus>
              Let&rsquo;s go!
            </button>
          </>
        ) : phase === "reading" ? (
          <p className="talk-prompt" aria-live="polite">
            <LoaderCircle className="spin" aria-hidden="true" /> Ella is finding your level…
          </p>
        ) : phase === "starting" ? (
          <p className="talk-prompt">
            So {greetName}, tell me about <em className="underline-pink">your day</em> so far!
          </p>
        ) : (
          <>
            <p className="inline-error placement-error" role="alert">
              {error}
            </p>
            <div className="placement-actions">
              <button className="btn btn--violet" onClick={retry}>
                Try again
              </button>
              <button className="link-button link-button--muted" onClick={() => onDone(null)}>
                Skip for now
              </button>
            </div>
          </>
        )}
      </div>

      <div className="talk-dock">
        <EllaMascot variant="conversation" className="ella--stage-talk" state={ella}>
          <div className="mic-stack">
            <div className="mic-wrap">
              <button
                className={`mic ${phase === "placed" ? "is-done" : ""}`.trim()}
                disabled
                aria-label={phase === "placed" ? "First talk finished" : "Getting the first talk ready"}
              >
                {phase === "placed" ? (
                  <svg className="mic__check" viewBox="0 0 24 24" aria-hidden="true">
                    <path d="M20 6L9 17L4 12" />
                  </svg>
                ) : phase === "starting" || phase === "reading" ? (
                  <LoaderCircle className="spin" size={30} aria-hidden="true" />
                ) : (
                  <MicGlyph />
                )}
              </button>
            </div>
            <p className="mic-hint mic-hint--soft">
              {phase === "placed" ? "Talk finished" : phase === "reading" ? "Ella is thinking…" : "One moment…"}
            </p>
          </div>
        </EllaMascot>
      </div>
    </div>
  );
}

function message(reason: unknown): string {
  if (typeof reason === "string") return reason;
  if (reason instanceof Error) return reason.message;
  return "Something went wrong. Please try again.";
}
