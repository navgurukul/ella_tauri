import { useEffect, useState } from "react";
import { LoaderCircle, X } from "lucide-react";
import { EllaGlyph, EllaMascot } from "./components/EllaMascot";
import { HomeScreen } from "./components/HomeScreen";
import { OnboardingFlow } from "./components/OnboardingFlow";
import { Sidebar, type NavKey } from "./components/Sidebar";
import { SummaryScreen } from "./components/SummaryScreen";
import { TalkScreen } from "./components/TalkScreen";
import { bridge } from "./lib/bridge";
import { recommendedTopicId, unfinishedSession } from "./lib/presentation";
import { formatBytes, useSetupState, type SetupState } from "./lib/setup";
import { applyUpdateIfAny, type UpdateProgress } from "./lib/updates";
import type { VoiceCaptureResult } from "./lib/speech";
import type { AppSnapshot, Session, SessionSummary, Topic } from "./types";

type Screen = "onboarding" | "home" | "talk" | "summary";

const NAV_FOR: Record<Exclude<Screen, "onboarding">, NavKey> = {
  home: "home",
  talk: "talk",
  summary: "home",
};

export default function App() {
  const [snapshot, setSnapshot] = useState<AppSnapshot | null>(null);
  const [screen, setScreen] = useState<Screen>("onboarding");
  const [session, setSession] = useState<Session | null>(null);
  const [summary, setSummary] = useState<SessionSummary | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [update, setUpdate] = useState<UpdateProgress | null>(null);
  const setup = useSetupState();

  // Once, at launch: the only point where closing the app costs nothing.
  useEffect(() => {
    void applyUpdateIfAny(setUpdate);
  }, []);

  useEffect(() => {
    let active = true;
    bridge
      .bootstrap()
      .then((data) => {
        if (!active) return;
        setSnapshot(data);
        setScreen(data.learner ? "home" : "onboarding");
      })
      .catch((reason: unknown) => active && setError(errorMessage(reason)));
    return () => {
      active = false;
    };
  }, []);

  async function run(action: () => Promise<void>) {
    setBusy(true);
    setError(null);
    try {
      await action();
    } catch (reason) {
      setError(errorMessage(reason));
    } finally {
      setBusy(false);
    }
  }

  /** Onboarding saves the learner before the placement talk, and reports back
   * whether it stuck so the flow only advances on a name the backend accepted. */
  async function handleSaveLearner(name: string, age: number | null): Promise<boolean> {
    let saved = false;
    await run(async () => {
      await bridge.saveLearner(name, age);
      // Re-read rather than patching: the backend orders topics by age, so the
      // learner's answer changes what Ella offers from here on.
      setSnapshot(await bridge.bootstrap());
      saved = true;
    });
    return saved;
  }

  /**
   * The onboarding first answer runs through the ordinary pipeline: a real
   * session, a real voice turn, a real completion. Anything that fails along
   * the way (no speech recognised, engines still warming) is swallowed —
   * nothing is graded on it, so onboarding just carries on.
   */
  async function handlePlacement(capture: VoiceCaptureResult): Promise<void> {
    if (!snapshot || capture.samples.length === 0) return;
    let startedId: string | null = null;
    try {
      const created = await bridge.startSession(recommendedTopicId(snapshot));
      startedId = created.id;
      await bridge.sendVoiceTurn({
        sessionId: created.id,
        samples: capture.samples,
        sampleRate: capture.sampleRate,
        browserTranscript: capture.transcript,
      });
      const result = await bridge.completeSession(created.id);
      setSnapshot((current) =>
        current
          ? {
              ...current,
              recent_sessions: [
                listItemFor(result, created.topic_id, created.started_at),
                ...current.recent_sessions,
              ].slice(0, 5),
            }
          : current,
      );
    } catch {
      // Never leave a half-open placement talk waiting on the home screen.
      if (startedId) await bridge.completeSession(startedId).catch(() => undefined);
    }
  }

  async function handleStart(topic: Topic) {
    await run(async () => {
      const created = await bridge.startSession(topic.id);
      setSession(created);
      setSummary(null);
      setScreen("talk");
    });
  }

  /** Starts a topic by id; a null id means "whatever Ella recommends". */
  async function handleStartTopic(topicId: string | null) {
    if (!snapshot) return;
    const wanted = topicId ?? recommendedTopicId(snapshot);
    const topic = snapshot.topics.find((candidate) => candidate.id === wanted);
    if (topic) await handleStart(topic);
  }

  /** Reopens a conversation the learner left running, messages and all. */
  async function handleResume(sessionId: string) {
    await run(async () => {
      const resumed = await bridge.getSession(sessionId);
      setSession(resumed);
      setSummary(null);
      setScreen("talk");
    });
  }

  function handleComplete(result: SessionSummary) {
    if (!snapshot) return;
    setSummary(result);
    // Drop the finished conversation. `session` is a frozen snapshot taken
    // before completion, so its status still reads "active" — leaving it set
    // let the sidebar's "Talk" reopen a session the backend had closed, and
    // every turn after that failed with "this conversation has already ended".
    setSession(null);
    setSnapshot({
      ...snapshot,
      recent_sessions: [
        listItemFor(result, session?.topic_id ?? "", session?.started_at),
        ...snapshot.recent_sessions.filter((item) => item.id !== result.session_id),
      ].slice(0, 5),
    });
    setScreen("summary");
  }

  async function handleReset() {
    await run(async () => {
      const fresh = await bridge.resetDemoData();
      setSnapshot(fresh);
      setSession(null);
      setSummary(null);
      setScreen("onboarding");
    });
  }

  /**
   * "Talk" in the sidebar resumes the live conversation when there is one, then
   * an unfinished one left over from a previous run, and otherwise starts the
   * topic Ella is recommending — the nav should never land on an empty stage.
   */
  function handleNavigate(key: NavKey) {
    if (!snapshot) return;
    if (key === "talk") {
      if (session && session.status === "active") {
        setScreen("talk");
        return;
      }
      const unfinished = unfinishedSession(snapshot);
      if (unfinished) {
        void handleResume(unfinished.id);
        return;
      }
      void handleStartTopic(null);
      return;
    }
    setScreen(key);
  }

  // An update replaces the running app, so it owns the screen while it works.
  // "checking" is deliberately not shown: it is over in a moment, and a flash
  // of update UI on every launch reads as instability.
  if (update && update.stage !== "checking") return <UpdateScreen progress={update} />;

  if (!snapshot) return <BootScreen error={error} />;

  if (screen === "onboarding") {
    return (
      <>
        <OnboardingFlow
          busy={busy}
          error={error}
          onSaveLearner={handleSaveLearner}
          onPlacement={handlePlacement}
          onDone={() => setScreen("home")}
        />
        <SetupBanner setup={setup} />
        {busy && <BusyVeil />}
      </>
    );
  }

  return (
    <div className={`shell ${screen === "talk" ? "shell--immersive" : ""}`.trim()}>
      {screen !== "talk" && (
        <Sidebar
          active={NAV_FOR[screen]}
          learnerName={snapshot.learner?.name ?? "friend"}
          onNavigate={handleNavigate}
          onReset={handleReset}
        />
      )}
      <main className="stage">
        {screen === "home" && (
          <HomeScreen
            snapshot={snapshot}
            busy={busy}
            onStart={(topic) => void handleStart(topic)}
            onResume={(sessionId) => void handleResume(sessionId)}
          />
        )}
        {screen === "talk" && session && (
          <TalkScreen
            key={session.id}
            session={session}
            onSessionChange={setSession}
            onComplete={handleComplete}
          />
        )}
        {screen === "summary" && summary && (
          <SummaryScreen summary={summary} onHome={() => setScreen("home")} />
        )}
      </main>
      <SetupBanner setup={setup} />
      {busy && <BusyVeil />}
      {error && <Toast message={error} onClose={() => setError(null)} />}
    </div>
  );
}

/** The list entry a just-finished conversation leaves on the home screen. */
function listItemFor(result: SessionSummary, topicId: string, startedAt?: string) {
  return {
    id: result.session_id,
    topic_id: topicId,
    topic_label: result.topic_label,
    status: "complete" as const,
    started_at: startedAt ?? new Date().toISOString(),
    message_count: result.turns * 2 + 1,
  };
}

function BootScreen({ error }: { error: string | null }) {
  return (
    <div className="boot">
      <div className="wordmark wordmark--lg">
        <EllaGlyph size={48} />
        <span>Ella</span>
      </div>
      <h1 className="display display--md">{error ? "Ella could not start" : "Waking Ella up…"}</h1>
      <p>{error ?? "Getting your local learning space ready."}</p>
      {!error && <LoaderCircle className="spin" aria-label="Loading" />}
      {!error && <EllaMascot className="ella--corner-boot" scale={0.55} rotate={-4} />}
    </div>
  );
}

/**
 * The first launch after an install has gigabytes to fetch before Ella can
 * speak. It runs behind the app rather than in front of it: a learner can put
 * in their name and check their microphone while it downloads, and only the
 * talking itself has to wait. Anything that needs the engine early says so in
 * its own words, because the backend returns that sentence with the failure.
 */
function SetupBanner({ setup }: { setup: SetupState | null }) {
  if (!setup || setup.stage === "ready") return null;

  const downloading = setup.stage === "downloading" && setup.total_bytes > 0;
  const percent = downloading
    ? Math.min(100, Math.round((setup.downloaded_bytes / setup.total_bytes) * 100))
    : null;

  return (
    <div className={`setup-strip ${setup.stage === "failed" ? "setup-strip--failed" : ""}`.trim()} aria-live="polite">
      {setup.stage !== "failed" && <LoaderCircle className="spin" aria-hidden="true" />}
      <div className="setup-strip__text">
        <strong>
          {setup.stage === "failed" ? "Ella could not finish setting up" : "Getting Ella ready"}
        </strong>
        <span>
          {setup.stage === "failed"
            ? setup.message
            : downloading
              ? `${setup.message} — ${formatBytes(setup.downloaded_bytes)} of ${formatBytes(setup.total_bytes)}`
              : setup.message}
        </span>
      </div>
      {percent !== null && (
        <div className="setup-strip__bar" role="progressbar" aria-valuenow={percent}>
          <span style={{ width: `${percent}%` }} />
        </div>
      )}
    </div>
  );
}

/** An update rewrites the app underneath itself, so nothing else is on screen. */
function UpdateScreen({ progress }: { progress: UpdateProgress }) {
  const percent =
    progress.totalBytes > 0
      ? Math.min(100, Math.round((progress.downloadedBytes / progress.totalBytes) * 100))
      : null;
  const heading =
    progress.stage === "restarting" ? "Starting the new Ella…" : "Updating Ella…";

  return (
    <div className="boot">
      <div className="wordmark wordmark--lg">
        <EllaGlyph size={48} />
        <span>Ella</span>
      </div>
      <h1 className="display display--md">{heading}</h1>
      <p>
        {progress.version ? `Version ${progress.version}. ` : ""}
        This only takes a moment, and your talks are kept.
      </p>
      {percent !== null && (
        <div className="setup-strip__bar setup-strip__bar--wide" role="progressbar" aria-valuenow={percent}>
          <span style={{ width: `${percent}%` }} />
        </div>
      )}
      <LoaderCircle className="spin" aria-label="Updating" />
    </div>
  );
}

function BusyVeil() {
  return (
    <div className="veil" aria-live="polite">
      <LoaderCircle className="spin" aria-hidden="true" />
      <span>One moment…</span>
    </div>
  );
}

function Toast({ message, onClose }: { message: string; onClose: () => void }) {
  return (
    <div className="toast" role="alert">
      <span>{message}</span>
      <button onClick={onClose} aria-label="Dismiss">
        <X size={16} />
      </button>
    </div>
  );
}

function errorMessage(reason: unknown): string {
  if (typeof reason === "string") return reason;
  if (reason instanceof Error) return reason.message;
  return "Something unexpected happened. Please try again.";
}
