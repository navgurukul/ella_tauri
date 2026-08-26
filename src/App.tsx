import { FormEvent, useEffect, useMemo, useRef, useState } from "react";
import {
  ArrowLeft,
  Check,
  CircleStop,
  Clock3,
  Heart,
  Home,
  Keyboard,
  Leaf,
  LoaderCircle,
  MessageCircle,
  Mic,
  PhoneOff,
  Play,
  RotateCcw,
  Send,
  Settings2,
  Sparkles,
  Sprout,
  Volume2,
  WifiOff,
  X,
} from "lucide-react";
import { GardenPreview } from "./components/GardenPreview";
import { VoiceMeter, ZoeMascot, type ZoeState } from "./components/ZoeMascot";
import { bridge } from "./lib/bridge";
import { createVoiceCapture, speakText } from "./lib/speech";
import { llog, logServerTimings, markTurnStart, turnElapsed } from "./lib/latency";
import type {
  AppSnapshot,
  EngineStatus,
  Message,
  Session,
  SessionSummary,
  Topic,
  TurnResult,
} from "./types";

type Screen = "onboarding" | "home" | "conversation" | "summary" | "garden";

export default function App() {
  const [snapshot, setSnapshot] = useState<AppSnapshot | null>(null);
  const [screen, setScreen] = useState<Screen>("onboarding");
  const [session, setSession] = useState<Session | null>(null);
  const [summary, setSummary] = useState<SessionSummary | null>(null);
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

  async function handleSaveLearner(name: string) {
    if (!snapshot) return;
    await run(async () => {
      const learner = await bridge.saveLearner(name);
      setSnapshot({ ...snapshot, learner });
      setScreen("home");
    });
  }

  async function handleStart(topic: Topic) {
    await run(async () => {
      const created = await bridge.startSession(topic.id);
      setSession(created);
      setSummary(null);
      setScreen("conversation");
    });
  }

  async function handleComplete(result: SessionSummary) {
    if (!snapshot) return;
    setSummary(result);
    setSnapshot({
      ...snapshot,
      garden: result.garden,
      recent_sessions: [
        {
          id: result.session_id,
          topic_label: result.topic_label,
          status: "complete" as const,
          started_at: session?.started_at ?? new Date().toISOString(),
          message_count: (result.turns * 2 + 1),
        },
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

  if (!snapshot) {
    return <LoadingScreen error={error} />;
  }

  if (screen === "onboarding") {
    return <OnboardingScreen busy={busy} error={error} onSubmit={handleSaveLearner} />;
  }

  return (
    <div className="desktop-shell">
      <Sidebar
        snapshot={snapshot}
        active={screen}
        onHome={() => setScreen("home")}
        onGarden={() => setScreen("garden")}
        onReset={handleReset}
      />
      <main className="workspace">
        {screen === "home" && (
          <HomeScreen snapshot={snapshot} busy={busy} onStart={handleStart} />
        )}
        {screen === "conversation" && session && (
          <ConversationScreen
            session={session}
            engineStatus={snapshot.engine_status}
            onSessionChange={setSession}
            onComplete={handleComplete}
            onBack={() => setScreen("home")}
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
          <GardenScreen snapshot={snapshot} onHome={() => setScreen("home")} />
        )}
      </main>
      {busy && <BusyVeil />}
      {error && <Toast message={error} onClose={() => setError(null)} />}
    </div>
  );
}

function LoadingScreen({ error }: { error: string | null }) {
  return (
    <main className="loading-screen">
      <div className="brand-mark brand-mark--large"><Sprout aria-hidden="true" /></div>
      <h1>{error ? "ZoSpeak could not start" : "Growing your garden…"}</h1>
      <p>{error ?? "Preparing Zoe and your local learning space."}</p>
      {!error && <LoaderCircle className="spin" aria-label="Loading" />}
    </main>
  );
}

function OnboardingScreen({
  busy,
  error,
  onSubmit,
}: {
  busy: boolean;
  error: string | null;
  onSubmit: (name: string) => Promise<void>;
}) {
  const [name, setName] = useState("");
  return (
    <main className="onboarding-shell">
      <section className="onboarding-copy">
        <div className="wordmark"><span className="wordmark-leaf">●</span> zoSpeak</div>
        <div className="eyebrow"><Sparkles size={16} /> Your voice grows here</div>
        <h1>Speak English.<br /><em>Grow confidence.</em></h1>
        <p>
          Meet Zoe, a friendly speaking buddy who listens without judging and helps your English garden grow.
        </p>
        <ul className="feature-list">
          <li><Check size={18} /> Short, real conversations</li>
          <li><Check size={18} /> Voice first, typing when you need it</li>
          <li><Check size={18} /> Your words become visible progress</li>
        </ul>
        <form
          className="name-form"
          onSubmit={(event) => {
            event.preventDefault();
            void onSubmit(name);
          }}
        >
          <label htmlFor="learner-name">What should Zoe call you?</label>
          <div className="name-input-row">
            <input
              id="learner-name"
              value={name}
              maxLength={40}
              autoFocus
              placeholder="Your first name"
              onChange={(event) => setName(event.target.value)}
            />
            <button className="primary-button" disabled={busy || name.trim().length < 2}>
              Meet Zoe <span aria-hidden="true">→</span>
            </button>
          </div>
          {error && <p className="form-error">{error}</p>}
          <small>No account needed for this proof of concept. Data stays on this computer.</small>
        </form>
      </section>
      <section className="onboarding-scene" aria-label="Zoe in a growing garden">
        <div className="scene-sun" />
        <span className="floating-word floating-word--one">hello!</span>
        <span className="floating-word floating-word--two">I can do this</span>
        <ZoeMascot size="large" state="speaking" />
        <div className="onboarding-hills" />
      </section>
    </main>
  );
}

function Sidebar({
  snapshot,
  active,
  onHome,
  onGarden,
  onReset,
}: {
  snapshot: AppSnapshot;
  active: Screen;
  onHome: () => void;
  onGarden: () => void;
  onReset: () => void;
}) {
  const [showEngines, setShowEngines] = useState(false);
  return (
    <aside className="sidebar">
      <div className="wordmark wordmark--sidebar"><span className="wordmark-leaf">●</span> zoSpeak</div>
      <nav aria-label="Main navigation">
        <button className={active === "home" ? "active" : ""} onClick={onHome}>
          <Home size={20} /> Home
        </button>
        <button className={active === "garden" || active === "summary" ? "active" : ""} onClick={onGarden}>
          <Leaf size={20} /> My garden
        </button>
      </nav>
      <div className="sidebar-garden-card">
        <div className="sidebar-card-heading">
          <span>Morning Meadow</span>
          <strong>{snapshot.garden.total_conversations}</strong>
        </div>
        <GardenPreview garden={snapshot.garden} compact />
        <p>Every conversation helps a skill grow.</p>
      </div>
      <div className="sidebar-bottom">
        <button className="engine-button" onClick={() => setShowEngines((value) => !value)}>
          <span className={`status-dot ${snapshot.engine_status.ready ? "ready" : "warning"}`} />
          <span><strong>{snapshot.engine_status.label}</strong><small>{snapshot.engine_status.ready ? "Ready" : "Needs attention"}</small></span>
          <Settings2 size={17} />
        </button>
        {showEngines && <EnginePopover status={snapshot.engine_status} onReset={onReset} />}
        <div className="profile-chip">
          <span>{snapshot.learner?.name.charAt(0).toUpperCase()}</span>
          <div><strong>{snapshot.learner?.name}</strong><small>Morning Meadow</small></div>
        </div>
      </div>
    </aside>
  );
}

function EnginePopover({ status, onReset }: { status: EngineStatus; onReset: () => void }) {
  return (
    <div className="engine-popover">
      <strong>{status.mode === "demo" ? "POC mode" : "Local engine mode"}</strong>
      {status.components.map((component) => (
        <div className="engine-row" key={component.name}>
          <span className={`status-dot ${component.ready ? "ready" : "warning"}`} />
          <span><b>{component.name}</b><small>{component.detail}</small></span>
        </div>
      ))}
      <button className="text-button danger-text" onClick={onReset}><RotateCcw size={14} /> Reset POC data</button>
    </div>
  );
}

function HomeScreen({
  snapshot,
  busy,
  onStart,
}: {
  snapshot: AppSnapshot;
  busy: boolean;
  onStart: (topic: Topic) => Promise<void>;
}) {
  const [selectedId, setSelectedId] = useState(snapshot.topics[0]?.id ?? "");
  const selected = snapshot.topics.find((topic) => topic.id === selectedId) ?? snapshot.topics[0];
  return (
    <div className="home-screen">
      <header className="page-header">
        <div>
          <span className="eyebrow">Good to see you</span>
          <h1>Ready to talk, {snapshot.learner?.name}?</h1>
          <p>Pick something you care about. Zoe will keep the conversation easy and natural.</p>
        </div>
        <div className="streak-card"><span>🌱</span><div><strong>{snapshot.garden.total_conversations}</strong><small>talks completed</small></div></div>
      </header>

      <section className="home-grid">
        <div className="topic-panel">
          <div className="section-heading">
            <div><span className="section-kicker"><Sparkles size={15} /> Zoe recommends</span><h2>What should we talk about?</h2></div>
            <span className="level-pill">Morning Meadow</span>
          </div>
          <div className="topic-grid" role="radiogroup" aria-label="Conversation topic">
            {snapshot.topics.map((topic) => (
              <button
                role="radio"
                aria-checked={selectedId === topic.id}
                className={`topic-card topic-card--${topic.color} ${selectedId === topic.id ? "selected" : ""}`}
                key={topic.id}
                onClick={() => setSelectedId(topic.id)}
              >
                <span className="topic-emoji">{topic.emoji}</span>
                <span><strong>{topic.label}</strong><small>{topic.prompt}</small></span>
                <span className="radio-mark">{selectedId === topic.id && <Check size={15} />}</span>
              </button>
            ))}
          </div>
          <button className="primary-button primary-button--wide" disabled={busy || !selected} onClick={() => selected && void onStart(selected)}>
            <MessageCircle size={20} /> Talk about {selected?.label.toLowerCase()}
          </button>
          <p className="privacy-note"><WifiOff size={14} /> Demo conversations and progress stay on this computer.</p>
        </div>

        <aside className="zoe-home-card">
          <div className="zoe-speech-card">
            <span className="tiny-label">A note from Zoe</span>
            <p>“There are no wrong answers here. Take your time—I’m listening.”</p>
          </div>
          <ZoeMascot state="resting" size="large" />
          <div className="home-ground" />
        </aside>
      </section>

      {snapshot.recent_sessions.length > 0 && (
        <section className="recent-section">
          <div className="section-heading"><h2>Recent practice</h2></div>
          <div className="recent-list">
            {snapshot.recent_sessions.slice(0, 3).map((item) => (
              <article key={item.id}>
                <span className="recent-icon"><MessageCircle size={18} /></span>
                <div><strong>{item.topic_label}</strong><small>{new Date(item.started_at).toLocaleDateString()}</small></div>
                <span>{Math.max(0, Math.floor((item.message_count - 1) / 2))} turns</span>
              </article>
            ))}
          </div>
        </section>
      )}
    </div>
  );
}

function ConversationScreen({
  session,
  engineStatus,
  onSessionChange,
  onComplete,
  onBack,
}: {
  session: Session;
  engineStatus: EngineStatus;
  onSessionChange: (session: Session) => void;
  onComplete: (summary: SessionSummary) => void;
  onBack: () => void;
}) {
  const [zoeState, setZoeState] = useState<ZoeState>("resting");
  const [input, setInput] = useState("");
  const [typing, setTyping] = useState(false);
  const [liveTranscript, setLiveTranscript] = useState("");
  const [voiceLevel, setVoiceLevel] = useState(0);
  const [sending, setSending] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [lastTurn, setLastTurn] = useState<TurnResult | null>(null);
  const voice = useRef(createVoiceCapture());
  const stopSpeech = useRef<() => void>(() => undefined);
  const speakingTimer = useRef<number | null>(null);

  const learnerMessages = session.messages.filter((message) => message.speaker === "learner");
  const latestLearner = learnerMessages.at(-1);
  const latestZoe = [...session.messages].reverse().find((message) => message.speaker === "zoe");
  const turnCount = learnerMessages.length;

  useEffect(() => {
    if (session.messages.length === 1 && latestZoe) playZoe(latestZoe.content);
    return () => {
      stopSpeech.current();
      if (speakingTimer.current) window.clearTimeout(speakingTimer.current);
      void voice.current.cancel();
    };
    // The opening should play once for a newly mounted conversation.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  function playZoe(text: string, result?: TurnResult) {
    stopSpeech.current();
    if (speakingTimer.current) window.clearTimeout(speakingTimer.current);
    setZoeState("speaking");
    stopSpeech.current = speakText(text, result?.audio);
    const duration = Math.max(1600, Math.min(7000, text.length * 48));
    speakingTimer.current = window.setTimeout(() => setZoeState("resting"), duration);
  }

  async function beginListening() {
    setError(null);
    stopSpeech.current();
    try {
      setLiveTranscript("");
      setVoiceLevel(0);
      await voice.current.start(setLiveTranscript, setVoiceLevel);
      setZoeState("listening");
      llog("mic:listening", "microphone open, learner is speaking");
    } catch (reason) {
      setTyping(true);
      setZoeState("resting");
      setError(`${errorMessage(reason)} You can type your answer instead.`);
    }
  }

  async function finishListening() {
    const stoppedAt = markTurnStart("VOICE TURN");
    llog("mic:stop-pressed", "learner stopped speaking; turn clock starts now");
    setSending(true);
    setZoeState("thinking");
    setError(null);
    try {
      const capture = await voice.current.stop();
      llog(
        "ipc:send_voice_turn",
        `invoking Rust with ${capture.samples.length} samples @ ${capture.sampleRate} Hz` +
          (capture.transcript ? ` (browser transcript: "${capture.transcript}")` : ""),
      );
      const ipcStarted = performance.now();
      const result = await bridge.sendVoiceTurn({
        sessionId: session.id,
        samples: capture.samples,
        sampleRate: capture.sampleRate,
        browserTranscript: capture.transcript || liveTranscript,
      });
      llog(
        "ipc:result",
        `Rust round-trip took ${(performance.now() - ipcStarted).toFixed(1)}ms ` +
          `(Rust reported total ${result.timings?.total_ms ?? "-"}ms; ` +
          `IPC serialization overhead ~${result.timings ? Math.max(0, Math.round(performance.now() - ipcStarted - result.timings.total_ms)) : "-"}ms)`,
      );
      llog("transcript", `"${result.learner_message.content}"`);
      llog("zoe-reply", `"${result.zoe_message.content}"`);
      logServerTimings(result.timings);
      applyResult(result);
      llog(
        "turn:playback-ready",
        `END-TO-END mic stop -> playback started: ${turnElapsed().toFixed(1)}ms`,
      );
      console.info(JSON.stringify({
        event: "ella_voice_playback_ready",
        schema_version: 1,
        interaction_id: result.timings?.interaction_id,
        stop_to_playback_ready_ms: Math.round(performance.now() - stoppedAt),
        server_total_ms: result.timings?.total_ms,
        stt_engine: result.timings?.stt_engine,
        stt_backend: result.timings?.stt_backend,
      }));
      setLiveTranscript("");
    } catch (reason) {
      llog("turn:error", `voice turn failed after ${turnElapsed().toFixed(1)}ms: ${errorMessage(reason)}`);
      setZoeState("resting");
      setError(errorMessage(reason));
      if (liveTranscript) setInput(liveTranscript);
      setTyping(true);
    } finally {
      setVoiceLevel(0);
      setSending(false);
    }
  }

  async function submitText(event: FormEvent) {
    event.preventDefault();
    if (!input.trim() || sending) return;
    markTurnStart("TEXT TURN");
    llog("ipc:send_text_turn", `invoking Rust with ${input.trim().length} chars`);
    setSending(true);
    setZoeState("thinking");
    setError(null);
    try {
      const ipcStarted = performance.now();
      const result = await bridge.sendTextTurn(session.id, input);
      llog(
        "ipc:result",
        `Rust round-trip took ${(performance.now() - ipcStarted).toFixed(1)}ms ` +
          `(Rust reported total ${result.timings?.total_ms ?? "-"}ms)`,
      );
      llog("zoe-reply", `"${result.zoe_message.content}"`);
      logServerTimings(result.timings);
      setInput("");
      applyResult(result);
      llog(
        "turn:playback-ready",
        `END-TO-END submit -> playback started: ${turnElapsed().toFixed(1)}ms`,
      );
    } catch (reason) {
      llog("turn:error", `text turn failed after ${turnElapsed().toFixed(1)}ms: ${errorMessage(reason)}`);
      setZoeState("resting");
      setError(errorMessage(reason));
    } finally {
      setSending(false);
    }
  }

  function applyResult(result: TurnResult) {
    onSessionChange({
      ...session,
      messages: [...session.messages, result.learner_message, result.zoe_message],
    });
    setLastTurn(result);
    playZoe(result.zoe_message.content, result);
  }

  async function endConversation() {
    setSending(true);
    setError(null);
    stopSpeech.current();
    try {
      if (zoeState === "listening") await voice.current.cancel();
      const result = await bridge.completeSession(session.id);
      onComplete(result);
    } catch (reason) {
      setError(errorMessage(reason));
    } finally {
      setSending(false);
    }
  }

  return (
    <div className="conversation-screen">
      <header className="conversation-header">
        <button className="icon-button" onClick={onBack} aria-label="Leave conversation"><ArrowLeft /></button>
        <div><small>Talking about</small><strong>{session.topic_label}</strong></div>
        <div className="turn-progress" aria-label={`${turnCount} turns completed`}>
          {[1, 2, 3].map((turn) => <span className={turn <= turnCount ? "filled" : ""} key={turn} />)}
        </div>
        <button className="end-button" onClick={() => void endConversation()} disabled={sending}><PhoneOff size={17} /> End</button>
      </header>

      <div className="conversation-stage">
        <div className="stage-background" />
        <div className="conversation-focus">
          <div className={`status-pill status-pill--${zoeState}`}>
            <span />
            {zoeState === "listening" && "I’m listening…"}
            {zoeState === "thinking" && "Zoe is thinking…"}
            {zoeState === "speaking" && "Zoe is speaking"}
            {zoeState === "resting" && (engineStatus.mode === "local" ? "Offline AI ready" : "POC demo ready")}
          </div>
          <ZoeMascot state={zoeState} size="large" />

          <div className="latest-exchange" aria-live="polite">
            {latestZoe && (
              <div className="bubble bubble--zoe">
                <span className="bubble-speaker">Zoe</span>
                <p>{latestZoe.content}</p>
                <button className="replay-button" onClick={() => playZoe(latestZoe.content, lastTurn ?? undefined)} aria-label="Replay Zoe's message"><Volume2 size={17} /></button>
              </div>
            )}
            {latestLearner && (
              <div className="bubble bubble--learner"><span className="bubble-speaker">You</span><p>{latestLearner.content}</p></div>
            )}
          </div>

          {lastTurn?.correction && <div className="coach-note"><Sparkles size={16} /><span><strong>Try this</strong>{lastTurn.correction}</span></div>}
          {error && <div className="inline-error"><X size={16} /> {error}</div>}
        </div>
      </div>

      <footer className="conversation-dock">
        {typing ? (
          <form className="typing-composer" onSubmit={submitText}>
            <button type="button" className="icon-button" onClick={() => setTyping(false)} aria-label="Use microphone"><Mic /></button>
            <input
              autoFocus
              value={input}
              maxLength={800}
              placeholder="Type what you want to say…"
              onChange={(event) => setInput(event.target.value)}
            />
            <button className="send-button" disabled={!input.trim() || sending} aria-label="Send answer">
              {sending ? <LoaderCircle className="spin" /> : <Send />}
            </button>
          </form>
        ) : (
          <div className="voice-composer">
            <button className="typing-toggle" onClick={() => setTyping(true)}><Keyboard size={19} /> Type instead</button>
            <div className="mic-area">
              {zoeState === "listening" ? (
                <button className="mic-button mic-button--recording" onClick={() => void finishListening()} disabled={sending} aria-label="Stop and send">
                  <CircleStop />
                </button>
              ) : (
                <button className="mic-button" onClick={() => void beginListening()} disabled={sending || zoeState === "thinking"} aria-label="Start speaking">
                  <Mic />
                </button>
              )}
              <div className="mic-copy">
                <strong>{zoeState === "listening" ? "Tap when you’re done" : "Tap to speak"}</strong>
                {zoeState === "listening" && <VoiceMeter level={voiceLevel} />}
                {liveTranscript && <span>{liveTranscript}</span>}
              </div>
            </div>
            <span className="turn-hint">Take your time</span>
          </div>
        )}
      </footer>
    </div>
  );
}

function SummaryScreen({
  summary,
  onGarden,
  onHome,
}: {
  summary: SessionSummary;
  onGarden: () => void;
  onHome: () => void;
}) {
  return (
    <div className="summary-screen">
      <div className="celebration-particles" aria-hidden="true"><i /><i /><i /><i /><i /></div>
      <section className="summary-copy">
        <div className="success-mark"><Sprout /></div>
        <span className="eyebrow">Conversation complete</span>
        <h1>{summary.headline}</h1>
        <p>{summary.encouragement}</p>
        <div className="summary-stats">
          <div><strong>{summary.turns}</strong><span>answers shared</span></div>
          <div><strong>{summary.best_evidence ? 1 : 0}</strong><span>skill watered</span></div>
        </div>
        {summary.best_evidence && (
          <blockquote>
            <span>Your own words</span>
            “{summary.best_evidence.evidence}”
            <footer>{summary.best_evidence.skill_label} · {summary.best_evidence.stage_label}</footer>
          </blockquote>
        )}
        <div className="summary-actions">
          <button className="primary-button" onClick={onGarden}><Leaf size={19} /> See my garden</button>
          <button className="secondary-button" onClick={onHome}><Home size={19} /> Back home</button>
        </div>
      </section>
      <section className="summary-garden"><GardenPreview garden={summary.garden} /></section>
    </div>
  );
}

function GardenScreen({ snapshot, onHome }: { snapshot: AppSnapshot; onHome: () => void }) {
  return (
    <div className="garden-screen">
      <header className="page-header garden-page-header">
        <div>
          <span className="eyebrow"><Leaf size={15} /> Your learning garden</span>
          <h1>{snapshot.garden.level_name}</h1>
          <p>Each plant grows when your own conversations show a skill.</p>
        </div>
        <button className="secondary-button" onClick={onHome}><Home size={18} /> Choose a topic</button>
      </header>
      <GardenPreview garden={snapshot.garden} />
      <section className="skill-evidence-grid">
        {snapshot.garden.skills.map((skill) => (
          <article className={`evidence-card evidence-card--${skill.strand}`} key={skill.id}>
            <div className="evidence-card-top"><span><Sprout size={19} /></span><small>{skill.strand}</small></div>
            <h2>{skill.label}</h2>
            <div className="stage-track" aria-label={`${skill.stage_label}, ${skill.evidence_count} demonstrations`}>
              {[1, 2, 3].map((stage) => <i className={stage <= skill.stage ? "filled" : ""} key={stage} />)}
            </div>
            <strong>{skill.stage_label}</strong>
            {skill.last_evidence ? <q>{skill.last_evidence}</q> : <p>Have a conversation to plant the first seed.</p>}
          </article>
        ))}
      </section>
    </div>
  );
}

function BusyVeil() {
  return <div className="busy-veil" aria-live="polite"><LoaderCircle className="spin" /><span>Getting Zoe ready…</span></div>;
}

function Toast({ message, onClose }: { message: string; onClose: () => void }) {
  return <div className="toast" role="alert"><Heart size={18} /><span>{message}</span><button onClick={onClose} aria-label="Dismiss"><X /></button></div>;
}

function errorMessage(reason: unknown): string {
  if (typeof reason === "string") return reason;
  if (reason instanceof Error) return reason.message;
  return "Something unexpected happened. Please try again.";
}
