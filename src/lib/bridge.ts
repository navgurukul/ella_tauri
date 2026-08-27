import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type {
  AppSnapshot,
  EllaBridge,
  Learner,
  Message,
  Session,
  SessionSummary,
  SpeechSegment,
  SpokenLine,
  Topic,
  TurnResult,
  VoiceStreamFinishInput,
  VoiceTurnInput,
} from "../types";

const topics: Topic[] = [
  {
    id: "street-food",
    label: "Street food stories",
    prompt: "Describe tastes, smells and your favourite stall.",
    emoji: "🍛",
    color: "violet",
  },
  {
    id: "restaurant-order",
    label: "Ordering at a restaurant",
    prompt: "Order a meal, ask about the menu, and settle the bill.",
    emoji: "🍽",
    color: "pink",
  },
  {
    id: "booking-a-cab",
    label: "Booking a cab",
    prompt: "Give an address, agree a fare, and ask how long it takes.",
    emoji: "🚕",
    color: "green",
  },
  {
    id: "job-interview",
    label: "A job interview",
    prompt: "Introduce yourself and answer questions about your work.",
    emoji: "💼",
    color: "lilac",
  },
  {
    id: "doctor-clinic",
    label: "At the doctor's clinic",
    prompt: "Explain how you feel and understand what to do next.",
    emoji: "🩺",
    color: "violet",
  },
  {
    id: "asking-directions",
    label: "Asking for directions",
    prompt: "Find your way and repeat the directions back.",
    emoji: "🗺",
    color: "ink",
  },
  {
    id: "market-bargaining",
    label: "Bargaining at the market",
    prompt: "Ask the price, bargain kindly, and agree a deal.",
    emoji: "🛒",
    color: "orange",
  },
];

/** Mirrors `topics_for_age` in the Rust domain so the preview matches the app. */
const MIN_AGE: Record<string, number> = { "job-interview": 14, "market-bargaining": 10 };

function topicsForAge(age?: number | null): Topic[] {
  if (age == null) return topics;
  return topics
    .map((topic, index) => ({ topic, index }))
    .sort(
      (left, right) =>
        Number((MIN_AGE[left.topic.id] ?? 0) > age) - Number((MIN_AGE[right.topic.id] ?? 0) > age) ||
        left.index - right.index,
    )
    .map((entry) => entry.topic);
}

interface BrowserState {
  learner?: Learner;
  sessions: Session[];
  /** Conversations finished, the one progress number the preview still keeps. */
  conversations: number;
}

export interface StorageLike {
  getItem(key: string): string | null;
  setItem(key: string, value: string): void;
  removeItem(key: string): void;
}

const storageKey = "ella-desktop-state";

function isTauriRuntime(): boolean {
  return typeof window !== "undefined" && Boolean(window.__TAURI_INTERNALS__);
}

class TauriBridge implements EllaBridge {
  bootstrap = () => invoke<AppSnapshot>("bootstrap");
  saveLearner = (name: string, age?: number | null) =>
    invoke<Learner>("save_learner", { name, age: age ?? null });
  startSession = (topicId: string) => invoke<Session>("start_session", { topicId });
  getSession = (sessionId: string) => invoke<Session>("get_session", { sessionId });
  speakOpening = (sessionId: string) => invoke<SpokenLine>("speak_opening", { sessionId });
  sendTextTurn = (sessionId: string, text: string) =>
    invoke<TurnResult>("send_text_turn", { sessionId, text });
  sendVoiceTurn = ({ sessionId, samples, sampleRate, browserTranscript }: VoiceTurnInput) =>
    invoke<TurnResult>("send_voice_turn", {
      sessionId,
      samples,
      sampleRate,
      browserTranscript,
    });
  /** Matches SPEECH_STREAM_EVENT in src-tauri/src/lib.rs. */
  onSpeechSegment = async (handler: (segment: SpeechSegment) => void) => {
    const stop = await listen<SpeechSegment>("ella://speech-segment", (event) =>
      handler(event.payload),
    );
    return stop;
  };
  beginVoiceStream = (sessionId: string) => invoke<string>("begin_voice_stream", { sessionId });
  pushVoiceStream = (streamId: string, samples: number[], sampleRate: number) =>
    invoke<void>("push_voice_stream", { streamId, samples, sampleRate });
  cancelVoiceStream = (streamId: string) => invoke<void>("cancel_voice_stream", { streamId });
  finishVoiceStreamTurn = ({ streamId, tailSamples, sampleRate, browserTranscript }: VoiceStreamFinishInput) =>
    invoke<TurnResult>("finish_voice_stream_turn", {
      streamId,
      tailSamples,
      sampleRate,
      browserTranscript,
    });
  completeSession = (sessionId: string) =>
    invoke<SessionSummary>("complete_session", { sessionId });
  resetDemoData = () => invoke<AppSnapshot>("reset_demo_data");
}

export function createBrowserBridge(storage: StorageLike = window.localStorage): EllaBridge {
  const read = (): BrowserState => {
    const raw = storage.getItem(storageKey);
    if (raw) return JSON.parse(raw) as BrowserState;
    return { sessions: [], conversations: 0 };
  };

  const write = (state: BrowserState) => storage.setItem(storageKey, JSON.stringify(state));

  const snapshot = (state: BrowserState): AppSnapshot => ({
    learner: state.learner,
    topics: topicsForAge(state.learner?.age),
    recent_sessions: state.sessions
      .slice()
      .reverse()
      .slice(0, 5)
      .map((session) => ({
        id: session.id,
        topic_id: session.topic_id,
        topic_label: session.topic_label,
        status: session.status,
        started_at: session.started_at,
        message_count: session.messages.length,
      })),
    engine_status: {
      mode: "demo",
      label: "Browser demo",
      ready: true,
      components: [
        { name: "Conversation", ready: true, detail: "Deterministic local tutor" },
        { name: "Voice", ready: true, detail: "System speech when available" },
        { name: "Storage", ready: true, detail: "Browser-local preview data" },
      ],
    },
  });

  const getSessionOrThrow = (state: BrowserState, sessionId: string) => {
    const session = state.sessions.find((candidate) => candidate.id === sessionId);
    if (!session) throw new Error("That conversation could not be found.");
    return session;
  };

  /** Closes a session and builds its summary. Shared so a conversation that
   * ends on its own last turn and one the learner ends by hand produce the
   * same thing. */
  const closeSession = (state: BrowserState, sessionId: string): SessionSummary => {
    const session = getSessionOrThrow(state, sessionId);
    session.status = "complete";
    session.completed_at = new Date().toISOString();
    state.conversations += 1;
    const learners = session.messages.filter((message) => message.speaker === "learner");
    return {
      session_id: session.id,
      topic_label: session.topic_label,
      turns: learners.length,
      headline:
        learners.length >= 3 ? "You kept that conversation going" : "Every answer counts",
      encouragement: `You kept a real conversation about ${session.topic_label.toLowerCase()} going. That is brave practice.`,
    };
  };

  const send = async (sessionId: string, text: string): Promise<TurnResult> => {
    const clean = text.trim();
    if (!clean) throw new Error("Say or type something first.");
    const state = read();
    const session = getSessionOrThrow(state, sessionId);
    if (session.status !== "active") throw new Error("This conversation has already ended.");
    const turn = session.messages.filter((message) => message.speaker === "learner").length + 1;
    const now = new Date().toISOString();
    const learnerMessage: Message = {
      id: crypto.randomUUID(),
      speaker: "learner",
      content: clean,
      turn,
      created_at: now,
    };
    const ellaMessage: Message = {
      id: crypto.randomUUID(),
      speaker: "ella",
      content: demoReply(session.topic_id, clean, turn),
      turn,
      created_at: new Date().toISOString(),
    };
    session.messages.push(learnerMessage, ellaMessage);

    // Mirrors FREE_TOPIC_TURNS in engines.rs: the sixth turn is the last one,
    // and the session closes itself rather than saying goodbye again.
    const sessionSummary = turn >= 6 ? closeSession(state, session.id) : null;
    write(state);
    return {
      learner_message: learnerMessage,
      ella_message: ellaMessage,
      correction: gentleCorrection(clean),
      suggested_complete: turn >= 3,
      session_summary: sessionSummary,
      // The browser bridge has no Piper, so nothing streams ahead of the turn
      // and there are no timings to highlight against.
      streamed_segments: 0,
      speech_words: [],
    };
  };

  return {
    async bootstrap() {
      return snapshot(read());
    },
    async saveLearner(name, age) {
      const clean = name.trim();
      if (clean.length < 2) throw new Error("Please enter at least two letters.");
      if (age != null && (age < 3 || age > 120)) {
        throw new Error("Please enter an age between 3 and 120.");
      }
      const state = read();
      state.learner = {
        name: clean,
        // Onboarding can be re-run without the age step; keep what we know.
        age: age ?? state.learner?.age ?? null,
        level_name: "Morning Meadow",
        created_at: state.learner?.created_at ?? new Date().toISOString(),
      };
      write(state);
      return state.learner;
    },
    async startSession(topicId) {
      const state = read();
      const topic = topics.find((candidate) => candidate.id === topicId);
      if (!topic) throw new Error("Choose one of the available topics.");
      const session: Session = {
        id: crypto.randomUUID(),
        topic_id: topic.id,
        topic_label: topic.label,
        status: "active",
        started_at: new Date().toISOString(),
        messages: [
          {
            id: crypto.randomUUID(),
            speaker: "ella",
            content: openingFor(topic.id, state.learner?.name ?? "friend"),
            turn: 0,
            created_at: new Date().toISOString(),
          },
        ],
      };
      state.sessions.push(session);
      write(state);
      return structuredClone(session);
    },
    async getSession(sessionId) {
      return structuredClone(getSessionOrThrow(read(), sessionId));
    },
    sendTextTurn: send,
    async sendVoiceTurn(input) {
      if (!input.browserTranscript?.trim()) {
        throw new Error(
          "I captured your voice, but speech recognition is unavailable in this preview. Try typing your answer.",
        );
      }
      return send(input.sessionId, input.browserTranscript);
    },
    async completeSession(sessionId) {
      const state = read();
      const summary = closeSession(state, sessionId);
      write(state);
      return summary;
    },
    async resetDemoData() {
      storage.removeItem(storageKey);
      return snapshot(read());
    },
  };
}

export const bridge: EllaBridge = isTauriRuntime() ? new TauriBridge() : createBrowserBridge();

function openingFor(topicId: string, name: string): string {
  switch (topicId) {
    case "restaurant-order":
      return `Hi ${name}! We are at a restaurant and I am your waiter. What would you like to order today?`;
    case "booking-a-cab":
      return `Hi ${name}! I am the cab driver. Where would you like to go, and where should I pick you up?`;
    case "job-interview":
      return `Hello ${name}! Thank you for coming in. To start, could you tell me a little about yourself?`;
    case "doctor-clinic":
      return `Hi ${name}! I am the doctor here. Please sit down and tell me, how have you been feeling?`;
    case "asking-directions":
      return `Hi ${name}! You look a little lost. Where are you trying to go? I know this area well.`;
    case "market-bargaining":
      return `Hi ${name}! Come, come, best prices here. What are you looking for today?`;
    default:
      return `Hi ${name}! Tell me about the tastiest thing you ate this week. Where did you find it?`;
  }
}

function demoReply(topicId: string, text: string, turn: number): string {
  const lead = text.split(/\s+/).slice(0, 4).join(" ");
  if (turn >= 3) {
    return `I enjoyed hearing that, especially \u201C${lead}\u201D. Before we finish, what feeling does this story give you?`;
  }
  if (topicId === "street-food") {
    return `That sounds delicious! You said \u201C${lead}\u201D. Who would you like to share that with, and why?`;
  }
  if (topicId === "job-interview") {
    return "Good, that is a clear answer. What part of that work do you enjoy the most?";
  }
  return "I can picture that! What happened next, and how did you feel?";
}

function gentleCorrection(text: string): string | null {
  if (/\bi goed\b/i.test(text)) return "Try “I went” instead of “I goed.”";
  if (/\bi am went\b/i.test(text)) return "Try “I went” when you are talking about the past.";
  return null;
}
