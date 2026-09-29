import { useEffect, useRef, useState } from "react";
import { CircleCheck, LoaderCircle, X } from "lucide-react";
import { CastScreen } from "./components/CastScreen";
import { HomeScreen } from "./components/HomeScreen";
import { LevelsScreen } from "./components/LevelsScreen";
import { MicRecheck, OnboardingFlow } from "./components/OnboardingFlow";
import { PlacementTalk } from "./components/PlacementTalk";
import { ProfileScreen } from "./components/ProfileScreen";
import { SetupScreen } from "./components/SetupScreen";
import { Sidebar, type NavKey } from "./components/Sidebar";
import { SummaryScreen } from "./components/SummaryScreen";
import { TalkScreen } from "./components/TalkScreen";
import { avatarColorFor, forgetLegacyAvatarColor, legacyAvatarColor } from "./lib/avatar";
import { bridge } from "./lib/bridge";
import { nextTopicLabel, recommendedTopicId, streak } from "./lib/presentation";
import { useSetup } from "./lib/setup";
import {
  downloadUpdateInBackground,
  installExitsTheApp,
  type ApplyUpdate,
  type UpdateProgress,
} from "./lib/updates";
import { levelTone } from "./lib/curriculum";
import type {
  AppSnapshot,
  Assessment,
  BadgeStart,
  CastGoal,
  LearnerProgress,
  Session,
  SessionSummary,
  Topic,
} from "./types";

/** A finished talk's summary, with what the recap needs from before it: the
 * learner's figures, which its streak counts on from, and its topic, which
 * tomorrow's suggestion steers away from. */
interface Finished {
  result: SessionSummary;
  before: LearnerProgress;
  topicId: string | null;
}

/** The assessment of one finished talk, as far as it has got. */
interface Assessing {
  sessionId: string;
  result: Assessment | null;
  error: string | null;
}

/** `miccheck` is the mic check opened again from the profile's settings;
 * `placement` is the placement chat opened from the level map by a learner
 * who never had one. */
type Screen =
  | "onboarding"
  | "home"
  | "cast"
  | "profile"
  | "levels"
  | "miccheck"
  | "placement"
  | "talk"
  | "summary";

/** Which nav item each screen that shows the sidebar lights up. */
const NAV_FOR: Partial<Record<Screen, NavKey | null>> = {
  home: "home",
  cast: "cast",
  profile: null,
  levels: null,
};

export default function App() {
  const [snapshot, setSnapshot] = useState<AppSnapshot | null>(null);
  const [screen, setScreen] = useState<Screen>("onboarding");
  const [session, setSession] = useState<Session | null>(null);
  const [finished, setFinished] = useState<Finished | null>(null);
  const summary = finished?.result ?? null;
  // What the last finished talk did for the learner. Asked for here rather
  // than by the summary, so leaving the summary before the model has read
  // the talk loses nothing: the snapshot is re-read whenever the answer
  // lands, and a step or level it finished is still celebrated (`movedOn`).
  const [assessing, setAssessing] = useState<Assessing | null>(null);
  const [movedOn, setMovedOn] = useState<Assessment | null>(null);
  const [busy, setBusy] = useState(false);
  // Where a placement chat skipped part-way goes back to: the level map, or
  // the profile when Hello, Ella's sheet started it.
  const [placementBack, setPlacementBack] = useState<"levels" | "profile">("levels");
  const [error, setError] = useState<string | null>(null);
  const [update, setUpdate] = useState<UpdateProgress | null>(null);
  const [applyUpdate, setApplyUpdate] = useState<ApplyUpdate | null>(null);
  const setup = useSetup();
  // Set once the setup screen lets the learner in, and never unset: the gate
  // is for the start of a launch, and must never close again over a talk.
  const [opened, setOpened] = useState(false);
  const gated = !opened;
  // Bumped by every change this window makes to the learner (a save, an
  // avatar colour, a log in or out), so a background re-read that started
  // before one cannot put the older state back on screen.
  const learnerEdits = useRef(0);
  // Bumped by every log out, so an assessment still on its way from the
  // learner who left cannot celebrate on the next sign-in.
  const signIns = useRef(0);
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

  // No Restart while the gate is up: a restart in the middle of the model
  // load is the one moment it costs the learner the most, and closing Ella
  // installs the update just the same.
  const updateToast = (
    <UpdateToast
      progress={update}
      onRestart={applyUpdate && !gated ? () => void applyUpdate() : undefined}
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
   * The placement chat is over: it read a level, or it was skipped. Either way
   * Home comes next, and the backend's own figures come with it — the new
   * level, and the streak and totals with the chat in them if it counted.
   */
  function handlePlacementDone() {
    setScreen("home");
    void refreshSnapshot();
  }

  async function handleStart(topic: Topic) {
    await run(async () => {
      const created = await bridge.startSession(topic.id);
      setSession(created);
      setFinished(null);
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
    if (start.kind === "chore") await handleStartChore(start.choreId);
  }

  /** A chore: from its partner's card, or again from the recap of one missed. */
  async function handleStartChore(choreId: string) {
    await run(async () => {
      const created = await bridge.startChore(choreId);
      setSession(created);
      setFinished(null);
      setScreen("talk");
    });
  }

  /** A badge sheet's mic: the partner's scenes on Talk partners, the
   * placement chat, or the talk that earns it. */
  function handleBadgeStart(start: BadgeStart) {
    if (start.kind === "partners") setScreen("cast");
    else if (start.kind === "placement") openPlacement("profile");
    else void handleStartTopic(start.topicId);
  }

  function openPlacement(from: "levels" | "profile") {
    setPlacementBack(from);
    setScreen("placement");
  }

  /** Reopens a conversation the learner left running, messages and all. */
  async function handleResume(sessionId: string) {
    await run(async () => {
      const resumed = await bridge.getSession(sessionId);
      setSession(resumed);
      setFinished(null);
      setScreen("talk");
    });
  }

  function handleComplete(result: SessionSummary) {
    if (!snapshot) return;
    // Taken before the re-read below, which already counts this talk.
    setFinished({ result, before: snapshot.progress, topicId: session?.topic_id ?? null });
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
    assess(result.session_id);
  }

  /**
   * Asks what a finished talk did: the skills it counted and any step or
   * level it finished. The first ask can take a while, as the model reads the
   * talk; the backend keeps the answer, so asking again is instant and counts
   * nothing twice. When it lands the snapshot is read again, wherever the
   * learner is by then, and if they have left the summary a step or level it
   * finished is celebrated where they are.
   */
  function assess(sessionId: string) {
    const signIn = signIns.current;
    setAssessing({ sessionId, result: null, error: null });
    bridge
      .assessSession(sessionId)
      .then((result) => {
        if (signIns.current !== signIn) return;
        setAssessing((current) => (current?.sessionId === sessionId ? { ...current, result } : current));
        // Kept until something shows it: the summary, if it is still up,
        // or else the toast wherever the learner has gone.
        if (result.advanced) setMovedOn(result);
        void refreshSnapshot();
      })
      .catch((reason: unknown) => {
        if (signIns.current !== signIn) return;
        setAssessing((current) =>
          current?.sessionId === sessionId ? { ...current, error: errorMessage(reason) } : current,
        );
      });
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
      signIns.current += 1;
      adopt(await bridge.logOut());
      setSession(null);
      setFinished(null);
      setAssessing(null);
      setMovedOn(null);
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

  // Nobody gets in until Ella can talk: a name typed or a talk started before
  // then would only meet "Ella is still downloading". The screen stands in
  // front of everything, onboarding and home alike, from the first frame
  // until setup says ready — one element throughout, so nothing behind it
  // ever flashes and its Ella only arrives once.
  if (gated || !snapshot) {
    const saved = snapshot?.learner ?? snapshot?.saved_learner ?? null;
    return (
      <>
        <SetupScreen
          setup={setup.state}
          booted={snapshot !== null}
          bootError={snapshot ? null : error}
          learnerName={saved?.name ?? null}
          retrying={setup.retrying}
          onRetry={setup.retry}
          onOpen={() => setOpened(true)}
        />
        {updateToast}
      </>
    );
  }

  const avatarColor = avatarColorFor(snapshot.learner);

  if (screen === "miccheck") {
    return (
      <>
        <MicRecheck onExit={() => setScreen("profile")} />
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
          onPlacementRead={() => void refreshSnapshot()}
          onDone={handlePlacementDone}
        />
        {updateToast}
        {busy && <BusyVeil />}
      </>
    );
  }

  const nav = NAV_FOR[screen];
  const immersive = screen === "talk" || screen === "placement" || screen === "summary";
  const openLevels = () => setScreen("levels");

  return (
    <div className={`shell ${immersive ? "shell--immersive" : ""}`.trim()}>
      {nav !== undefined && (
        <Sidebar
          active={nav}
          profileActive={screen === "profile" || screen === "levels"}
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
            onLevels={openLevels}
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
            onLevels={openLevels}
            onLogOut={() => void handleLogOut()}
            onBadgeStart={handleBadgeStart}
          />
        )}
        {screen === "levels" && (
          <LevelsScreen
            snapshot={snapshot}
            avatarColor={avatarColor}
            busy={busy}
            onBack={() => setScreen("profile")}
            onPractise={() => void handleStartTopic(null)}
            onFindLevel={() => openPlacement("levels")}
            onBadgeStart={handleBadgeStart}
          />
        )}
        {screen === "placement" && (
          <PlacementTalk
            greetName={snapshot.learner?.name ?? "friend"}
            onRead={() => void refreshSnapshot()}
            onDone={(placed) => {
              // Placed, the new level is the next thing to see; skipped, back
              // to where they asked for it.
              setScreen(placed ? "home" : placementBack);
              void refreshSnapshot();
            }}
          />
        )}
        {screen === "talk" && session && (
          <TalkScreen
            key={session.id}
            session={session}
            // A placement chat left open and picked up again from Home.
            variant={session.topic_id === "placement" ? "placement" : "talk"}
            onSessionChange={setSession}
            onComplete={handleComplete}
          />
        )}
        {screen === "summary" && finished && summary && (
          <SummaryScreen
            key={summary.session_id}
            summary={summary}
            assessment={assessing?.sessionId === summary.session_id ? assessing.result : null}
            assessError={assessing?.sessionId === summary.session_id ? assessing.error : null}
            before={finished.before}
            learnerName={snapshot.learner?.name ?? "friend"}
            standing={snapshot.standing ?? null}
            nextTopic={nextTopicLabel(snapshot, finished.topicId)}
            onRetry={() => assess(summary.session_id)}
            onCelebrated={() =>
              setMovedOn((current) => (current?.session_id === summary.session_id ? null : current))
            }
            onDone={() => setScreen("home")}
            onTryAgain={(choreId) => void handleStartChore(choreId)}
            onLevels={openLevels}
          />
        )}
      </main>
      {movedOn && !(screen === "summary" && summary?.session_id === movedOn.session_id) && (
        <MovedOnToast
          assessment={movedOn}
          onLevels={() => {
            setMovedOn(null);
            openLevels();
          }}
          onClose={() => setMovedOn(null)}
        />
      )}
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

/**
 * A new version downloading behind the app, in the corner and out of the way.
 */
function UpdateToast({
  progress,
  onRestart,
  onDismiss,
}: {
  progress: UpdateProgress | null;
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
    <div className="update-toast" aria-live="polite">
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

/**
 * A talk finished a step or a level after the learner had already left its
 * summary — the model was still reading the talk. They hear about it anyway,
 * where they are, as Ella Mobile celebrates a late move on Home.
 */
function MovedOnToast({
  assessment,
  onLevels,
  onClose,
}: {
  assessment: Assessment;
  onLevels: () => void;
  onClose: () => void;
}) {
  const { standing } = assessment;
  const level = assessment.advanced === "level";
  return (
    <div className={`moved-on-toast ladder-${levelTone(standing.level_number)}`} role="status">
      <span className="moved-on__badge moved-on__badge--small" aria-hidden="true">
        {level ? standing.level_number : "✓"}
      </span>
      <span className="moved-on-toast__text">
        <strong className="display">{level ? "Level up!" : "Step complete!"}</strong>
        <span>
          {level
            ? `You’ve reached ${standing.level_name}.`
            : `On to Step ${standing.step} of ${standing.level_name}.`}
        </span>
      </span>
      <button className="btn btn--quiet btn--compact" onClick={onLevels}>
        See levels
      </button>
      <button className="moved-on-toast__close" onClick={onClose} aria-label="Dismiss">
        <X size={16} />
      </button>
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
