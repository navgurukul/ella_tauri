import { useEffect, useState } from "react";
import { LoaderCircle, X } from "lucide-react";
import { EllaGlyph, EllaMascot } from "./components/EllaMascot";
import { GardenScreen } from "./components/GardenScreen";
import { HomeScreen } from "./components/HomeScreen";
import { OnboardingFlow } from "./components/OnboardingFlow";
import { Sidebar, type NavKey } from "./components/Sidebar";
import { SummaryScreen } from "./components/SummaryScreen";
import { TalkScreen } from "./components/TalkScreen";
import { bridge } from "./lib/bridge";
import { levelInfo, recommendedTopicId, unfinishedSession } from "./lib/presentation";
import type { VoiceCaptureResult } from "./lib/speech";
import type {
  AppSnapshot,
  PlacementResult,
  Session,
  SessionSummary,
  SkillEvidence,
  Topic,
} from "./types";

type Screen = "onboarding" | "home" | "talk" | "summary" | "garden";

const NAV_FOR: Record<Exclude<Screen, "onboarding">, NavKey> = {
  home: "home",
  talk: "talk",
  summary: "garden",
  garden: "garden",
};

export default function App() {
  const [snapshot, setSnapshot] = useState<AppSnapshot | null>(null);
  const [screen, setScreen] = useState<Screen>("onboarding");
  const [session, setSession] = useState<Session | null>(null);
  const [summary, setSummary] = useState<SessionSummary | null>(null);
  const [reveal, setReveal] = useState<SkillEvidence | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

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
   * The onboarding placement answer runs through the ordinary pipeline: a real
   * session, a real voice turn, a real completion. Anything that fails along
   * the way (no speech recognised, engines still warming) resolves to null and
   * onboarding falls back to the default band.
   */
  async function handlePlacement(capture: VoiceCaptureResult): Promise<PlacementResult | null> {
    if (!snapshot || capture.samples.length === 0) return null;
    let startedId: string | null = null;
    try {
      const created = await bridge.startSession(recommendedTopicId(snapshot));
      startedId = created.id;
      const turn = await bridge.sendVoiceTurn({
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
              garden: result.garden,
              recent_sessions: [
                listItemFor(result, created.topic_id, created.started_at),
                ...current.recent_sessions,
              ].slice(0, 5),
            }
          : current,
      );
      return {
        level: levelInfo(result.garden, snapshot.learner?.level_name).code,
        transcript: turn.learner_message.content,
      };
    } catch {
      // Never leave a half-open placement talk waiting on the home screen.
      if (startedId) await bridge.completeSession(startedId).catch(() => undefined);
      return null;
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
    setReveal(result.best_evidence ?? null);
    setSnapshot({
      ...snapshot,
      garden: result.garden,
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
      setReveal(null);
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
        {busy && <BusyVeil />}
      </>
    );
  }

  const level = levelInfo(snapshot.garden, snapshot.learner?.level_name);

  return (
    <div className={`shell ${screen === "talk" ? "shell--immersive" : ""}`.trim()}>
      {screen !== "talk" && (
        <Sidebar
          active={NAV_FOR[screen]}
          learnerName={snapshot.learner?.name ?? "friend"}
          level={level}
          engineStatus={snapshot.engine_status}
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
            onGarden={() => setScreen("garden")}
          />
        )}
        {screen === "talk" && session && (
          <TalkScreen
            key={session.id}
            session={session}
            snapshot={snapshot}
            onSessionChange={setSession}
            onComplete={handleComplete}
          />
        )}
        {screen === "summary" && summary && (
          <SummaryScreen
            summary={summary}
            onGarden={() => setScreen("garden")}
            onHome={() => setScreen("home")}
          />
        )}
        {screen === "garden" && (
          <GardenScreen
            snapshot={snapshot}
            reveal={reveal}
            onTalk={(topicId) => void handleStartTopic(topicId)}
          />
        )}
      </main>
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
      <h1 className="display display--md">{error ? "Ella could not start" : "Waking up the garden…"}</h1>
      <p>{error ?? "Getting your local learning space ready."}</p>
      {!error && <LoaderCircle className="spin" aria-label="Loading" />}
      {!error && <EllaMascot className="ella--corner-boot" scale={0.55} rotate={-4} />}
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
