import { useEffect, useRef, useState } from "react";
import { LoaderCircle } from "lucide-react";
import { EllaMascot, LearnerAvatar, VoiceMeter, type EllaState } from "./EllaMascot";
import { MicGlyph } from "./HomeScreen";
import { avatarColorFor } from "../lib/avatar";
import { createVoiceCapture, type VoiceCaptureResult } from "../lib/speech";
import type { LearnerProfile } from "../types";

/** `welcome-back` is Log in's own step, off to the side of the five in order. */
export type ObStep = "welcome" | "welcome-back" | "name" | "age" | "miccheck" | "placement";

const ORDER: ObStep[] = ["welcome", "name", "age", "miccheck", "placement"];
/** The dots track the four steps after the welcome screen. */
const DOT_COUNT = ORDER.length - 1;

type MicState = "idle" | "requesting" | "listening" | "done" | "error";

export function OnboardingFlow({
  busy,
  error,
  savedLearner,
  onSaveLearner,
  onLogIn,
  onPlacement,
  onDone,
}: {
  busy: boolean;
  error: string | null;
  /** The learner this laptop keeps, or null while nobody has been saved. */
  savedLearner: LearnerProfile | null;
  /** Persists the learner; resolves false when the backend rejected the name. */
  onSaveLearner: (name: string, age: number | null) => Promise<boolean>;
  /** Signs the saved learner back in; resolves false when that was refused. */
  onLogIn: () => Promise<boolean>;
  /** Runs the recorded first answer as a real conversation turn. */
  onPlacement: (capture: VoiceCaptureResult) => Promise<void>;
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

  async function comeBack() {
    if (await onLogIn()) onDone();
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
            // A laptop keeps one learner, so once someone is saved, Log in can
            // only be them: Ella greets them and there is nothing to type. On
            // a laptop nobody has used yet it asks for a name and goes in.
            if (savedLearner) {
              setStep("welcome-back");
              return;
            }
            setReturning(true);
            setStep("name");
          }}
        />
      )}

      {step === "welcome-back" && savedLearner && (
        <>
          <BackButton onClick={() => setStep("welcome")} />
          <CornerElla />
          <WelcomeBackStep learner={savedLearner} busy={busy} onLogIn={() => void comeBack()} />
        </>
      )}

      {isForm && (
        <>
          <BackButton onClick={back} />
          <ol className="ob__dots" aria-label={`Step ${index} of ${DOT_COUNT}`}>
            {Array.from({ length: DOT_COUNT }, (_, dot) => (
              <li key={dot} className={dot === index - 1 ? "is-current" : dot < index - 1 ? "is-done" : ""} />
            ))}
          </ol>
        </>
      )}
      {/* The age step has its own, bigger Ella; the other two keep her in the
          corner, where she peeks up to say hello once there is a name. */}
      {(step === "name" || step === "miccheck") && (
        <CornerElla greeting={step === "name" && name.trim() ? `Hi, ${greetName}!` : null} />
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

/**
 * The mic check again, opened from the profile's settings rather than as a
 * step of onboarding: same check, but it leads back to the profile.
 */
export function MicRecheck({ onExit }: { onExit: () => void }) {
  return (
    <div className="ob" data-step="miccheck">
      <BackButton onClick={onExit} />
      <CornerElla />
      <MicCheckStep onNext={onExit} doneLabel="Back to my profile" skipLabel="Not now" />
    </div>
  );
}

function BackButton({ onClick }: { onClick: () => void }) {
  return (
    <button className="ob__back" onClick={onClick} aria-label="Go back">
      <svg viewBox="0 0 24 24" aria-hidden="true">
        <path d="M15 5l-7 7 7 7" />
      </svg>
    </button>
  );
}

function CornerElla({ greeting = null }: { greeting?: string | null }) {
  return (
    <>
      {greeting && (
        <p className="ob-bubble" aria-hidden="true">
          {greeting}
        </p>
      )}
      <EllaMascot
        variant="corner"
        className={`ella--corner-ob ${greeting ? "is-peeking" : ""}`.trim()}
        pokeable
        decorative
      />
    </>
  );
}

function Welcome({ onStart, onLogIn }: { onStart: () => void; onLogIn: () => void }) {
  return (
    <div className="ob-welcome" data-screen="onboarding-welcome">
      <h1 className="display ob-welcome__title">Hi buddy!</h1>
      <EllaMascot className="ella--ob-welcome" variant="welcome" pokeable />
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
      <div className="ob-step__column">
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
        <button className="btn btn--violet btn--block ob-step__cta" disabled={!ready}>
          {returning ? "Take me in" : "Continue"}
        </button>
        <p className="ob-step__enter">or press Enter</p>
      </div>
    </form>
  );
}

/**
 * Log in on a laptop that already knows its learner. It only ever keeps one,
 * so there is no name to type or pick: Ella greets them in their own colour,
 * and one press takes them back to everything they left.
 */
function WelcomeBackStep({
  learner,
  busy,
  onLogIn,
}: {
  learner: LearnerProfile;
  busy: boolean;
  onLogIn: () => void;
}) {
  return (
    <div className="ob-step" data-screen="onboarding-welcome-back">
      <div className="ob-step__column">
        <LearnerAvatar color={avatarColorFor(learner)} size="lg" />
        <h1 className="display ob-step__title ob-step__title--greeting">Welcome back, {learner.name}!</h1>
        <p className="ob-step__sub">Your talks and your streak are just as you left them.</p>
        {/* Focused, so Enter takes them in as it does on the other steps. */}
        <button className="btn btn--violet btn--block ob-step__cta" disabled={busy} autoFocus onClick={onLogIn}>
          Take me in
        </button>
      </div>
    </div>
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
    <>
      {/* She springs up from the corner as soon as there is an age to react to. */}
      <EllaMascot
        variant="age"
        className={`ella--ob-age ${value ? "is-up" : ""}`.trim()}
        entrance={false}
        pokeable
        decorative
      />
      <form
        className="ob-step ob-step--top"
        data-screen="onboarding-age"
        onSubmit={(event) => {
          event.preventDefault();
          if (ready) onNext();
        }}
      >
        <div className="ob-step__column">
          <h1 className="display ob-step__title">
            <label htmlFor="ob-age">And how old are you, {greetName}?</label>
          </h1>
          <p className="ob-step__sub">Ella picks topics that fit your age.</p>
          <div className="ob-step__inline">
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
          </div>
        </div>
      </form>
    </>
  );
}

const MIC_HINT: Record<MicState, string> = {
  idle: "Click, then say anything",
  requesting: "Opening your microphone…",
  listening: "Listening…",
  done: "All good! Ella hears you.",
  error: "Let’s try that once more",
};

/**
 * A real check, not a mimed one: it opens the microphone and watches the input
 * level, so "Ella hears you" only appears once she actually has.
 */
function MicCheckStep({
  onNext,
  doneLabel = "Start my first talk",
  skipLabel = "Skip this check",
}: {
  onNext: () => void;
  doneLabel?: string;
  skipLabel?: string;
}) {
  const [mic, setMic] = useState<MicState>("idle");
  const [level, setLevel] = useState(0);
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
    setFailed("I didn’t hear anything. Check your input and click the mic to try again.");
    setMic("error");
  }

  async function tap() {
    if (mic === "done" || mic === "requesting") return;
    if (mic === "listening") {
      await finishCheck();
      return;
    }
    setFailed(null);
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

  return (
    <div className="ob-step" data-screen="onboarding-miccheck">
      <div className="ob-step__column ob-step__column--wide">
        <h1 className="display ob-step__title">Quick mic check first.</h1>
        <p className="ob-step__sub">
          Ella wants to hear you loud and clear before your first talk. Say anything!
        </p>

        <div className={`ob-mic-wrap is-${mic}`} data-mic-state={mic}>
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
              <MicGlyph size={40} />
            )}
          </button>
        </div>

        <p className="ob-mic-hint" aria-live="polite">
          {MIC_HINT[mic]}
        </p>
        {mic === "listening" && <VoiceMeter level={level} />}
        {failed && <p className="inline-error ob-mic-error" role="alert">{failed}</p>}

        {mic === "done" ? (
          <button className="btn btn--green ob-mic-cta" onClick={onNext}>
            {doneLabel}
          </button>
        ) : (
          <button className="link-button link-button--muted ob-step__skip" onClick={onNext}>
            {skipLabel}
          </button>
        )}
      </div>
    </div>
  );
}

type PlacementCall = "prompt" | "listening" | "working" | "done";

const CALL_HINT: Record<PlacementCall, string> = {
  prompt: "Click to speak",
  listening: "Listening… click when you finish",
  working: "Ella is listening back…",
  done: "Talk finished",
};

/**
 * The first talk. Ella asks one open question and the recorded answer runs
 * through the ordinary pipeline as a real conversation, so the learner arrives
 * home having already spoken once. Skipping, or having no microphone, simply
 * moves on. Nothing is graded: the proficiency level this step used to
 * announce left with the garden and is being rethought.
 */
function PlacementStep({
  greetName,
  busy,
  onPlacement,
  onDone,
}: {
  greetName: string;
  busy: boolean;
  onPlacement: (capture: VoiceCaptureResult) => Promise<void>;
  onDone: () => void;
}) {
  const [call, setCall] = useState<PlacementCall>("prompt");
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
      const capture = await voice.current.stop();
      await onPlacement(capture);
      setCall("done");
      return;
    }
    try {
      await voice.current.start(() => undefined, () => undefined);
      setCall("listening");
    } catch {
      // No microphone here — the answer is optional, so move on gracefully.
      setCall("done");
    }
  }

  const ellaState: EllaState = call === "working" ? "thinking" : "resting";

  return (
    <div className="screen screen--talk ob-placement" data-screen="onboarding-placement">
      <header className="talk-head">
        <span className="pill pill--white">First talk</span>
        <button className="btn btn--quiet" onClick={onDone} disabled={busy || call === "working"}>
          Skip
        </button>
      </header>

      <div className="talk-stage">
        {call === "done" ? (
          <>
            <p className="talk-prompt">That was lovely, {greetName}!</p>
            <button className="btn btn--green ob-placement__go" onClick={onDone} disabled={busy}>
              Let&rsquo;s go!
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
          <div className="mic-stack">
            <div className="mic-wrap">
              {call === "listening" && (
                <>
                  <span className="mic-pulse" />
                  <span className="mic-pulse mic-pulse--delayed" />
                </>
              )}
              <button
                className={`mic ${call === "done" ? "is-done" : ""}`.trim()}
                disabled={call === "working" || call === "done"}
                onClick={() => void tap()}
                aria-label={
                  call === "done"
                    ? "First talk finished"
                    : call === "listening"
                      ? "Stop and finish"
                      : "Start speaking"
                }
              >
                {call === "done" ? (
                  <svg className="mic__check" viewBox="0 0 24 24" aria-hidden="true">
                    <path d="M20 6L9 17L4 12" />
                  </svg>
                ) : call === "working" ? (
                  <LoaderCircle className="spin" size={30} aria-hidden="true" />
                ) : (
                  <MicGlyph />
                )}
              </button>
            </div>
            <p className="mic-hint mic-hint--soft">{CALL_HINT[call]}</p>
          </div>
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
