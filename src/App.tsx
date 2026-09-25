import { useEffect, useRef, useState } from "react";
import { CircleCheck, LoaderCircle, X } from "lucide-react";
import { CastScreen } from "./components/CastScreen";
import { EllaGlyph, EllaMascot } from "./components/EllaMascot";
import { HomeScreen } from "./components/HomeScreen";
import { MicRecheck, OnboardingFlow } from "./components/OnboardingFlow";
import { ProfileScreen } from "./components/ProfileScreen";
import { Sidebar, type NavKey } from "./components/Sidebar";
import { SummaryScreen } from "./components/SummaryScreen";
import { TalkScreen } from "./components/TalkScreen";
import { avatarColorFor, forgetLegacyAvatarColor, legacyAvatarColor } from "./lib/avatar";
import { bridge } from "./lib/bridge";
import { recommendedTopicId, streak } from "./lib/presentation";
import { formatBytes, useSetupState, type SetupState } from "./lib/setup";
import {
  downloadUpdateInBackground,
  installExitsTheApp,
  type ApplyUpdate,
  type UpdateProgress,
} from "./lib/updates";
import type { VoiceCaptureResult } from "./lib/speech";
import type { AppSnapshot, CastGoal, Session, SessionSummary, Topic } from "./types";

/** `miccheck` is the mic check opened again from the profile's settings. */
type Screen = "onboarding" | "home" | "cast" | "profile" | "miccheck" | "talk" | "summary";

/** Which nav item each screen that shows the sidebar lights up. */
const NAV_FOR: Partial<Record<Screen, NavKey | null>> = {
  home: "home",
  cast: "cast",
  profile: null,
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
  const [applyUpdate, setApplyUpdate] = useState<ApplyUpdate | null>(null);
  const setup = useSetupState();
  // Bumped by every change this window makes to the learner (a save, an
  // avatar colour, a log in or out), so a background re-read that started
  // before one cannot put the older state back on screen.
  const learnerEdits = useRef(0);
  // The avatar colour the backend last confirmed, which is what a refused
  // save goes back to — not whatever an earlier, unsaved press showed.
  const savedAvatarColor = useRef<string | null>(null);

  // Once, at launch, and entirely in the background: the learner keeps using
  // Ella while it downloads, and nothing installs mid-conversation.
  useEffect(() => {
    void downloadUpdateInBackground(setUpdate).then((apply) => {
      if (apply) setApplyUpdate(() => apply);
    });
  }, []);

  const updateToast = (
    <UpdateToast
      progress={update}
      raised={setup !== null && setup.stage !== "ready"}
      onRestart={applyUpdate ? () => void applyUpdate() : undefined}
      onDismiss={() => setUpdate(null)}
    />
  );

  useEffect(() => {
    let active = true;
    bridge
      .bootstrap()
      .then((data) => {
        if (!active) return;
        savedAvatarColor.current = data.learner?.avatar_color ?? null;
        setSnapshot(data);
        setScreen(data.learner ? "home" : "onboarding");
        void adoptLegacyAvatarColor(data);
      })
      .catch((reason: unknown) => active && setError(errorMessage(reason)));
    return () => {
      active = false;
    };
  }, []);

  /**
   * Earlier versions kept the avatar colour for the whole device, in the
   * webview's storage. The upgrade leaves their learner signed in, so the
   * first launch that finds the old colour gives it to them, unless they have
   * picked one since. Either way it is then forgotten, and from then on the
   * colour lives on the learner. With nobody signed in it belonged to nobody,
   * because logging out of those versions deleted the learner and cleared it.
   * A save that fails leaves it in place for the next launch to try again.
   */
  async function adoptLegacyAvatarColor(data: AppSnapshot) {
    const legacy = legacyAvatarColor();
    if (!legacy) return;
    if (data.learner && !data.learner.avatar_color) {
      try {
        learnerEdits.current += 1;
        const learner = await bridge.saveAvatarColor(legacy);
        savedAvatarColor.current = learner.avatar_color ?? null;
        setSnapshot((current) => withAvatarColor(current, learner.avatar_color ?? null));
      } catch {
        return;
      }
    }
    forgetLegacyAvatarColor();
  }

  /** Re-reads the snapshot in the background, so the streak, totals and
   * badges are the backend's own sums over the learner's whole history. If it
   * fails, the screen keeps what it has; if the learner was changed, or logged
   * out or in, while it was on its way, it is stale and dropped. */
  async function refreshSnapshot() {
    const edits = learnerEdits.current;
    try {
      const fresh = await bridge.bootstrap();
      if (learnerEdits.current === edits) adopt(fresh);
    } catch {
      // The next re-read catches up.
    }
  }

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
      learnerEdits.current += 1;
      await bridge.saveLearner(name, age);
      // Re-read rather than patching: the backend orders topics by age, so the
      // learner's answer changes what Ella offers from here on.
      adopt(await bridge.bootstrap());
      saved = true;
    });
    return saved;
  }

  /** "Take me in" on the welcome-back step signs the laptop's learner back in,
   * to every talk, the streak and the avatar colour as they left them.
   * Resolves false when that was refused, so the step stays open. */
  async function handleLogIn(): Promise<boolean> {
    let signedIn = false;
    await run(async () => {
      learnerEdits.current += 1;
      await bridge.logIn();
      adopt(await bridge.bootstrap());
      signedIn = true;
    });
    return signedIn;
  }

  /**
   * The onboarding first answer runs through the ordinary pipeline: a real
   * session, a real voice turn, a real completion. Anything that fails along
   * the way (no speech recognised, engines still warming) is swallowed —
   * nothing is graded on it, so onboarding just carries on. A talk that
   * failed before any answer was heard is closed but counts for nothing: the
   * backend only counts talks with something said in them.
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
      await bridge.completeSession(created.id);
    } catch {
      // Never leave a half-open placement talk waiting on the home screen.
      if (startedId) await bridge.completeSession(startedId).catch(() => undefined);
    }
    // Home opens on the backend's own streak and totals, with this talk in
    // them if it counted.
    await refreshSnapshot();
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

  /** A talk partner's goal: a chore the backend plays and scores, or the free
   * topic that stands in for one. */
  async function handleStartGoal(goal: CastGoal) {
    const { start } = goal;
    if (start.kind === "topic") {
      await handleStartTopic(start.topicId);
      return;
    }
    if (start.kind !== "chore") return;
    await run(async () => {
      const created = await bridge.startChore(start.choreId);
      setSession(created);
      setSummary(null);
      setScreen("talk");
    });
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
    // The summary shows at once, and the finished talk leaves the unfinished
    // card straight away; the streak, totals and badges follow a moment later,
    // from the backend.
    setSnapshot({
      ...snapshot,
      recent_sessions: [
        listItemFor(result, session?.topic_id ?? "", session?.started_at),
        ...snapshot.recent_sessions.filter((item) => item.id !== result.session_id),
      ].slice(0, 5),
    });
    setScreen("summary");
    void refreshSnapshot();
  }

  /**
   * "Log out" only signs out. Nothing is deleted: every talk, the streak and
   * the avatar colour stay saved on this laptop, and "Log in" brings them all
   * back. So does "Let's start": a laptop keeps one learner, so onboarding
   * again is that learner again, with their history.
   */
  async function handleLogOut() {
    await run(async () => {
      learnerEdits.current += 1;
      adopt(await bridge.logOut());
      setSession(null);
      setSummary(null);
      setScreen("onboarding");
    });
  }

  /**
   * The avatar changes the moment a swatch is pressed, with no veil over the
   * profile, and the backend saves it behind that. If the save is refused,
   * the avatar goes back to the colour that is actually saved — unless a
   * newer press has already replaced it — and the toast says why.
   */
  function handleAvatarColor(color: string) {
    if (!snapshot?.learner) return;
    learnerEdits.current += 1;
    setSnapshot((current) => withAvatarColor(current, color));
    bridge
      .saveAvatarColor(color)
      .then((saved) => {
        savedAvatarColor.current = saved.avatar_color ?? null;
      })
      .catch((reason: unknown) => {
        setSnapshot((current) =>
          current?.learner?.avatar_color === color
            ? withAvatarColor(current, savedAvatarColor.current)
            : current,
        );
        setError(errorMessage(reason));
      });
  }

  /** A snapshot straight from the backend, which is also the last word on
   * which avatar colour is saved. */
  function adopt(fresh: AppSnapshot) {
    savedAvatarColor.current = fresh.learner?.avatar_color ?? null;
    setSnapshot(fresh);
  }

  if (!snapshot) return <BootScreen error={error} />;

  const avatarColor = avatarColorFor(snapshot.learner);

  if (screen === "miccheck") {
    return (
      <>
        <MicRecheck onExit={() => setScreen("profile")} />
        <SetupBanner setup={setup} />
        {updateToast}
      </>
    );
  }

  if (screen === "onboarding") {
    return (
      <>
        <OnboardingFlow
          busy={busy}
          error={error}
          savedLearner={snapshot.saved_learner ?? null}
          onSaveLearner={handleSaveLearner}
          onLogIn={handleLogIn}
          onPlacement={handlePlacement}
          onDone={() => setScreen("home")}
        />
        <SetupBanner setup={setup} />
        {updateToast}
        {busy && <BusyVeil />}
      </>
    );
  }

  const nav = NAV_FOR[screen];

  return (
    <div className={`shell ${screen === "talk" ? "shell--immersive" : ""}`.trim()}>
      {nav !== undefined && (
        <Sidebar
          active={nav}
          learnerName={snapshot.learner?.name ?? "friend"}
          streakDays={streak(snapshot.progress).days}
          avatarColor={avatarColor}
          onNavigate={setScreen}
          onProfile={() => setScreen("profile")}
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
        {screen === "cast" && (
          <CastScreen
            learner={snapshot.learner}
            busy={busy}
            onStartGoal={(goal) => void handleStartGoal(goal)}
          />
        )}
        {screen === "profile" && (
          <ProfileScreen
            snapshot={snapshot}
            avatarColor={avatarColor}
            busy={busy}
            onSave={handleSaveLearner}
            onAvatarColor={handleAvatarColor}
            onMicCheck={() => setScreen("miccheck")}
            onLogOut={() => void handleLogOut()}
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
      {updateToast}
      {busy && <BusyVeil />}
      {error && <Toast message={error} onClose={() => setError(null)} />}
    </div>
  );
}

/** The snapshot with the signed-in learner's avatar colour changed, both
 * where it is drawn and on the welcome-back step that greets them after a log
 * out. Signed out, there is nobody whose colour it could be. */
function withAvatarColor(current: AppSnapshot | null, color: string | null): AppSnapshot | null {
  if (!current?.learner) return current;
  return {
    ...current,
    learner: { ...current.learner, avatar_color: color },
    saved_learner: { name: current.learner.name, avatar_color: color },
  };
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
      {!error && <EllaMascot variant="corner" className="ella--corner-boot" />}
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
              ? `${setup.message} — ${formatBytes(setup.downloaded_bytes)} of ${formatBytes(setup.total_bytes)}${
                  (setup.attempt ?? 1) > 1 ? " — connection dropped, retrying" : ""
                }`
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

/**
 * A new version downloading behind the app, in the corner and out of the way.
 * It wears the first-run strip's styling so both read as the same kind of
 * background work, and it sits above that strip whenever both are showing.
 */
function UpdateToast({
  progress,
  raised,
  onRestart,
  onDismiss,
}: {
  progress: UpdateProgress | null;
  raised: boolean;
  onRestart?: () => void;
  onDismiss: () => void;
}) {
  if (!progress) return null;

  const ready = progress.stage === "ready";
  const percent =
    progress.totalBytes > 0
      ? Math.min(100, Math.round((progress.downloadedBytes / progress.totalBytes) * 100))
      : null;
  const detail = ready
    ? installExitsTheApp()
      ? "Installs when you close Ella"
      : "Starts next time you open Ella"
    : percent !== null
      ? `Version ${progress.version} — ${percent}%`
      : `Version ${progress.version}`;

  return (
    <div className={`update-toast ${raised ? "update-toast--raised" : ""}`.trim()} aria-live="polite">
      {ready ? (
        <CircleCheck className="update-toast__icon" size={20} aria-hidden="true" />
      ) : (
        <LoaderCircle className="update-toast__icon spin" size={20} aria-hidden="true" />
      )}
      <div className="setup-strip__text">
        <strong>{ready ? "Update ready" : "Updating Ella"}</strong>
        <span>{detail}</span>
      </div>
      {ready ? (
        <>
          {onRestart && (
            <button className="update-toast__restart" onClick={onRestart}>
              Restart
            </button>
          )}
          <button className="update-toast__dismiss" onClick={onDismiss} aria-label="Dismiss">
            <X size={14} />
          </button>
        </>
      ) : (
        percent !== null && (
          <div className="setup-strip__bar" role="progressbar" aria-valuenow={percent}>
            <span style={{ width: `${percent}%` }} />
          </div>
        )
      )}
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
