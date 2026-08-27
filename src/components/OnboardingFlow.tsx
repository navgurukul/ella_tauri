import { useEffect, useRef, useState } from "react";
import { LoaderCircle } from "lucide-react";
import { EllaMascot, VoiceMeter, type EllaState } from "./EllaMascot";
import { MicGlyph } from "./HomeScreen";
import { createVoiceCapture, type VoiceCaptureResult } from "../lib/speech";
import type { PlacementResult } from "../types";

export type ObStep = "welcome" | "name" | "age" | "miccheck" | "placement";

const ORDER: ObStep[] = ["welcome", "name", "age", "miccheck", "placement"];
/** The dots track the four steps after the welcome screen. */
const DOT_COUNT = ORDER.length - 1;

type MicState = "idle" | "requesting" | "listening" | "done" | "error";

export function OnboardingFlow({
  busy,
  error,
  onSaveLearner,
  onPlacement,
  onDone,
}: {
  busy: boolean;
  error: string | null;
  /** Persists the learner; resolves false when the backend rejected the name. */
  onSaveLearner: (name: string, age: number | null) => Promise<boolean>;
  /** Runs the recorded first answer as a real conversation turn. */
  onPlacement: (capture: VoiceCaptureResult) => Promise<PlacementResult | null>;
  onDone: () => void;
}) {
  const [step, setStep] = useState<ObStep>("welcome");
  const [name, setName] = useState("");
  const [age, setAge] = useState("");
  // "Log in" is for someone who has met Ella before: it collects the name and
  // goes straight in, skipping the age, mic check and placement talk.
  const [returning, setReturning] = useState(false);

  const index = Math.max(0, ORDER.indexOf(step));
  const next = () => setStep(ORDER[Math.min(index + 1, ORDER.length - 1)]);
  const back = () => setStep(ORDER[Math.max(index - 1, 0)]);

  const greetName = name.trim() || "dost";
  const isForm = step === "name" || step === "age" || step === "miccheck";

  async function submitName() {
    if (!returning) {
      next();
      return;
    }
    if (await onSaveLearner(name, null)) onDone();
  }

  /**
   * The learner is saved here rather than at the end, because the placement
   * talk that follows is a real conversation and the backend will not start a
   * session for someone it does not know yet.
   */
  async function submitAge() {
    if (await onSaveLearner(name, age ? Number(age) : null)) next();
  }

  return (
    <div className="ob" data-step={step}>
      {step === "welcome" && (
        <Welcome
          onStart={() => {
            setReturning(false);
            next();
          }}
          onLogIn={() => {
            setReturning(true);
            setStep("name");
          }}
        />
      )}

      {isForm && (
        <>
          <button className="ob__back" onClick={back} aria-label="Go back">
            <svg viewBox="0 0 24 24" aria-hidden="true">
              <path d="M15 5l-7 7 7 7" />
            </svg>
          </button>
          <ol className="ob__dots" aria-label={`Step ${index} of ${DOT_COUNT}`}>
            {Array.from({ length: DOT_COUNT }, (_, dot) => (
              <li key={dot} className={dot === index - 1 ? "is-current" : dot < index - 1 ? "is-done" : ""} />
            ))}
          </ol>
          {step !== "miccheck" && <EllaMascot className="ella--corner-ob" scale={0.7} rotate={-5} />}
        </>
      )}

      {step === "name" && (
        <NameStep
          value={name}
          returning={returning}
          busy={busy}
          onChange={setName}
          onNext={() => void submitName()}
        />
      )}
      {step === "age" && (
        <AgeStep
          greetName={greetName}
          value={age}
          busy={busy}
          onChange={setAge}
          onNext={() => void submitAge()}
        />
      )}
      {step === "miccheck" && <MicCheckStep onNext={next} />}
      {step === "placement" && (
        <PlacementStep
          greetName={greetName}
          busy={busy}
          onPlacement={onPlacement}
          onDone={onDone}
        />
      )}

      {error && <p className="ob__error inline-error">{error}</p>}
    </div>
  );
}

function Welcome({ onStart, onLogIn }: { onStart: () => void; onLogIn: () => void }) {
  return (
    <div className="ob-welcome" data-screen="onboarding-welcome">
      <h1 className="display ob-welcome__title">Hi buddy!</h1>
      <EllaMascot className="ella--ob-hero" variant="hero" />
      <div className="ob-welcome__foot">
        <button className="btn btn--light ob-welcome__cta" onClick={onStart}>
          Let&rsquo;s start
        </button>
        <p className="ob-welcome__login">
          Already met Ella?{" "}
          <button className="link-button" onClick={onLogIn}>
            Log in
          </button>
        </p>
      </div>
    </div>
  );
}

function NameStep({
  value,
  returning,
  busy,
  onChange,
  onNext,
}: {
  value: string;
  returning: boolean;
  busy: boolean;
  onChange: (value: string) => void;
  onNext: () => void;
}) {
  const ready = value.trim().length >= 2 && !busy;
  return (
    <form
      className="ob-step"
      data-screen="onboarding-name"
      onSubmit={(event) => {
        event.preventDefault();
        if (ready) onNext();
      }}
    >
      <h1 className="display ob-step__title">
        <label htmlFor="ob-name">
          {returning ? "Welcome back! What is your name?" : "What should Ella call you?"}
        </label>
      </h1>
      <input
        id="ob-name"
        className="ob-step__field"
        value={value}
        maxLength={40}
        autoFocus
        placeholder="Your name"
        onChange={(event) => onChange(event.target.value)}
      />
      <p className="ob-step__hint">
        {returning
          ? "Use the same name as before and Ella picks up where you left off."
          : "A nickname works too. This stays between you two."}
      </p>
      <button className="btn btn--violet ob-step__cta" disabled={!ready}>
        {returning ? "Take me in" : "Continue"}
      </button>
    </form>
  );
}

function AgeStep({
  greetName,
  value,
  busy,
  onChange,
  onNext,
}: {
  greetName: string;
  value: string;
  busy: boolean;
  onChange: (value: string) => void;
  onNext: () => void;
}) {
  const parsed = Number(value);
  const ready = value !== "" && parsed >= 3 && parsed <= 120 && !busy;
  return (
    <form
      className="ob-step"
      data-screen="onboarding-age"
      onSubmit={(event) => {
        event.preventDefault();
        if (ready) onNext();
      }}
    >
      <h1 className="display ob-step__title">
        <label htmlFor="ob-age">And how old are you, {greetName}?</label>
      </h1>
      <p className="ob-step__sub">Ella picks topics that fit your age.</p>
      <input
        id="ob-age"
        className="ob-step__field ob-step__field--short"
        value={value}
        inputMode="numeric"
        maxLength={3}
        autoFocus
        placeholder="Age"
        onChange={(event) => onChange(event.target.value.replace(/[^0-9]/g, ""))}
      />
      <button className="btn btn--violet ob-step__cta" disabled={!ready}>
        Continue
      </button>
    </form>
  );
}

const MIC_HINT: Record<MicState, string> = {
  idle: "Tap the mic and say hello",
  requesting: "Opening your microphone…",
  listening: "Listening… say anything",
  done: "Perfect — Ella can hear you.",
  error: "Let’s try that once more",
};

/**
 * A real check, not a mimed one: it opens the microphone and watches the input
 * level, so "Ella hears you" only appears once she actually has.
 */
function MicCheckStep({ onNext }: { onNext: () => void }) {
  const [mic, setMic] = useState<MicState>("idle");
  const [level, setLevel] = useState(0);
  const [heardSignal, setHeardSignal] = useState(false);
  const [failed, setFailed] = useState<string | null>(null);
  const voice = useRef(createVoiceCapture());
  const heard = useRef(false);
  const active = useRef(false);
  const autoFinishTimer = useRef<number | null>(null);

  useEffect(() => {
    return () => {
      active.current = false;
      if (autoFinishTimer.current !== null) window.clearTimeout(autoFinishTimer.current);
      void voice.current.cancel();
    };
  }, []);

  async function finishCheck(didHear = heard.current) {
    if (!active.current) return;
    active.current = false;
    if (autoFinishTimer.current !== null) window.clearTimeout(autoFinishTimer.current);
    autoFinishTimer.current = null;
    await voice.current.cancel().catch(() => undefined);
    setLevel(0);
    if (didHear) {
      setFailed(null);
      setMic("done");
      return;
    }
    setFailed("I didn’t hear anything. Check your input and tap the mic to try again.");
    setMic("error");
  }

  async function tap() {
    if (mic === "done" || mic === "requesting") return;
    if (mic === "listening") {
      await finishCheck();
      return;
    }
    setFailed(null);
    setHeardSignal(false);
    heard.current = false;
    active.current = true;
    setMic("requesting");
    try {
      await voice.current.start(
        () => undefined,
        (value) => {
          setLevel(value);
          if (value > 0.06 && !heard.current) {
            heard.current = true;
            setHeardSignal(true);
            autoFinishTimer.current = window.setTimeout(() => void finishCheck(true), 1100);
          }
        },
      );
      if (active.current) setMic("listening");
    } catch (reason) {
      active.current = false;
      setLevel(0);
      setMic("error");
      setFailed(
        `I could not open the microphone (${message(reason).toLowerCase().replace(/\.$/, "")}). You can still talk to Ella by typing.`,
      );
    }
  }

  const ellaState: EllaState =
    mic === "requesting" ? "thinking" : mic === "listening" ? "listening" : "resting";
  const hint = mic === "listening" && heardSignal ? "I hear you — that sounds clear!" : MIC_HINT[mic];

  return (
    <div className="ob-step" data-screen="onboarding-miccheck">
      <h1 className="display ob-step__title">Quick mic check first.</h1>
      <p className="ob-step__sub">
        Ella wants to hear you loud and clear before your first talk. Say anything!
      </p>

      <div className={`ob-mic-stage is-${mic}`} data-mic-state={mic}>
        <EllaMascot
          variant="celebration"
          className="ella--mic-check"
          scale={0.5}
          state={ellaState}
          reaction={mic === "done" ? "success" : mic === "error" ? "error" : null}
          activity={level}
          decorative
        />
        <div className="ob-mic-wrap">
          {mic === "listening" && (
            <>
              <span className="ob-mic-pulse" />
              <span className="ob-mic-pulse ob-mic-pulse--delayed" />
            </>
          )}
          <button
            className={`ob-mic is-${mic}`}
            onClick={() => void tap()}
            disabled={mic === "requesting" || mic === "done"}
            aria-pressed={mic === "listening"}
            aria-label={
              mic === "requesting"
                ? "Opening the microphone"
                : mic === "listening"
                  ? "Stop the mic check"
                  : mic === "done"
                    ? "Microphone check complete"
                    : mic === "error"
                      ? "Try the mic check again"
                      : "Start the mic check"
            }
          >
            {mic === "requesting" ? (
              <LoaderCircle className="spin ob-mic__loader" aria-hidden="true" />
            ) : mic === "done" ? (
              <svg className="ob-mic__check" viewBox="0 0 24 24" aria-hidden="true">
                <path d="M20 6L9 17L4 12" />
              </svg>
            ) : (
              <MicGlyph size={34} />
            )}
          </button>
        </div>
      </div>

      <p className="ob-mic-hint" aria-live="polite">
        <span className="ob-mic-hint__dot" aria-hidden="true" />
        {hint}
      </p>
      {mic === "listening" && <VoiceMeter level={level} />}
      {failed && <p className="inline-error ob-mic-error" role="alert">{failed}</p>}

      {mic === "done" ? (
        <button className="btn btn--green ob-step__cta" onClick={onNext}>
          Start my first talk
        </button>
      ) : (
        <button className="link-button link-button--muted ob-step__skip" onClick={onNext}>
          Skip this check
        </button>
      )}
    </div>
  );
}

type PlacementCall = "prompt" | "listening" | "working" | "done";

/**
 * The placement talk. Ella asks one open question and the recorded answer runs
 * through the ordinary pipeline as a real first conversation, so the level
 * shown afterwards reflects a turn the learner actually took. Skipping, or
 * having no microphone, falls back to the app's default band.
 */
function PlacementStep({
  greetName,
  busy,
  onPlacement,
  onDone,
}: {
  greetName: string;
  busy: boolean;
  onPlacement: (capture: VoiceCaptureResult) => Promise<PlacementResult | null>;
  onDone: () => void;
}) {
  const [call, setCall] = useState<PlacementCall>("prompt");
  const [level, setLevel] = useState(0);
  const [result, setResult] = useState<PlacementResult | null>(null);
  const voice = useRef(createVoiceCapture());

  useEffect(
    () => () => {
      void voice.current.cancel();
    },
    [],
  );

  async function tap() {
    if (call === "done" || call === "working") return;
    if (call === "listening") {
      setCall("working");
      setLevel(0);
      const capture = await voice.current.stop();
      setResult(await onPlacement(capture));
      setCall("done");
      return;
    }
    try {
      await voice.current.start(() => undefined, setLevel);
      setCall("listening");
    } catch {
      // No microphone here — the answer is optional, so move on gracefully.
      setCall("done");
    }
  }

  const ellaState: EllaState = call === "listening" ? "listening" : call === "working" ? "thinking" : "resting";

  return (
    <div className="screen screen--talk ob-placement" data-screen="onboarding-placement">
      <header className="talk-head">
        <span className="pill pill--white">
          First talk
          <span className="pill__dot">·</span>
          <b className="pill__accent">Ella finds your level</b>
        </span>
        <button className="btn btn--quiet" onClick={onDone} disabled={busy || call === "working"}>
          Skip for now
        </button>
      </header>

      <div className="talk-stage">
        {call === "done" ? (
          <>
            <p className="talk-prompt">That was lovely, {greetName}!</p>
            <p className="level-badge">
              <span className="mono">YOUR LEVEL</span>
              <b>{result?.level ?? "A2"}</b>
            </p>
            <button className="btn btn--green" onClick={onDone} disabled={busy}>
              Start talking
            </button>
          </>
        ) : call === "working" ? (
          <p className="talk-prompt" aria-live="polite">
            <LoaderCircle className="spin" aria-hidden="true" /> Ella is listening back…
          </p>
        ) : (
          <p className="talk-prompt">
            So {greetName}, tell me about <em className="underline-pink">your day</em> so far!
          </p>
        )}
      </div>

      <div className="talk-dock">
        <EllaMascot variant="conversation" className="ella--stage-talk" state={ellaState}>
          {call !== "done" && (
            <div className="mic-stack">
              <div className="mic-wrap">
                {call === "listening" && (
                  <>
                    <span className="mic-pulse" />
                    <span className="mic-pulse mic-pulse--delayed" />
                  </>
                )}
                <button
                  className={`mic ${call === "listening" ? "is-live" : ""}`}
                  disabled={call === "working"}
                  onClick={() => void tap()}
                  aria-label={call === "listening" ? "Stop and finish" : "Start speaking"}
                >
                  <MicGlyph />
                </button>
              </div>
              <p className="mic-hint">
                {call === "listening" ? "Listening… tap when you finish" : "Tap to speak"}
              </p>
              {call === "listening" && <VoiceMeter level={level} />}
            </div>
          )}
        </EllaMascot>
      </div>
    </div>
  );
}

function message(reason: unknown): string {
  if (typeof reason === "string") return reason;
  if (reason instanceof Error) return reason.message;
  return "something went wrong";
}
