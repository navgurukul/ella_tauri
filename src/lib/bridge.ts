import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { dayKey } from "./days";
import type {
  AppSnapshot,
  DayActivity,
  EllaBridge,
  Learner,
  LearnerProgress,
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

/**
 * Mirrors `chores()` and `chore_opening_for` in the Rust domain, as far as the
 * preview needs them: the preview has no characters to play, so a chore here is
 * its title and the authored line it opens on.
 */
const chores: Array<{ id: string; title: string; opening: string }> = [
  {
    id: "market-cloth-price",
    title: "Talk a stall price down",
    opening: "Come, come, look at these shirts. Best cloth in this market. Which one has caught your eye?",
  },
  {
    id: "deposit-refund",
    title: "Get a deposit refunded",
    opening: "Oh, it is you. I am rather busy this morning. What did you want to talk about?",
  },
  {
    id: "sell-me-a-pen",
    title: "Sell me a pen",
    opening: "Right, you have thirty seconds and one pen. Go on then, what have you got for me?",
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

/** The learner as the preview keeps them: what the app sees, plus the
 * `signed_out` flag the backend keeps on the same row. A preview from before
 * log out kept anything has no flag, which reads as signed in, just as the
 * backend's column defaults to 0. */
interface StoredLearner extends Learner {
  signed_out?: boolean;
}

/**
 * Mirrors the backend's tables: the one learner this laptop keeps, once
 * someone has been saved, and every session. Signing out only sets the flag.
 * v0.1.6's preview stored the same two fields (and a count of finished talks,
 * which is now summed from the sessions instead), so it is read as it is.
 */
interface BrowserState {
  learner?: StoredLearner;
  sessions: Session[];
}

/**
 * What a development preview stored while it kept several learners on one
 * laptop, each session stamped with its owner and a pointer to whoever was
 * signed in. It was never released; it is read once and converted back.
 */
interface MultiLearnerState {
  learners: Array<Learner & { id: number; last_active_at: string }>;
  signed_in: number | null;
  sessions: Array<Session & { learner_id?: number | null }>;
}

type StoredState = Partial<BrowserState> & Partial<MultiLearnerState> & { conversations?: number };

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
  startChore = (choreId: string) => invoke<Session>("start_chore", { choreId });
  getSession = (sessionId: string) => invoke<Session>("get_session", { sessionId });
  speakOpening = (sessionId: string) => invoke<SpokenLine>("speak_opening", { sessionId });
  speakRetryPrompt = (sessionId: string) =>
    invoke<SpokenLine>("speak_retry_prompt", { sessionId });
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
  logIn = () => invoke<Learner>("log_in");
  logOut = () => invoke<AppSnapshot>("log_out");
  saveAvatarColor = (color: string) => invoke<Learner>("save_avatar_color", { color });
}

/** The backend's name checks for `save_learner`. */
function cleanName(name: string): string {
  const clean = name.trim();
  const letters = [...clean].length;
  if (letters < 2) throw new Error("Please enter at least two letters.");
  if (letters > 40) throw new Error("Please use a name shorter than 40 letters.");
  return clean;
}

const AVATAR_COLOR = /^#[0-9A-Fa-f]{6}$/;

/** No talks, no days: what a signed-out snapshot and a brand-new learner show. */
function emptyProgress(): LearnerProgress {
  return { days: [], talks_finished: 0, answers: 0, finished_topics: [] };
}

/**
 * The same sums the backend's progress query makes. An answer is a learner
 * message; a talk day is a local day with at least one; each talk is counted
 * on the day of its first answer; a finished talk is one ended with at least
 * one answer, so a talk where nothing was said counts for nothing at all.
 */
function progressOf(sessions: Session[]): LearnerProgress {
  const days = new Map<string, DayActivity>();
  const on = (key: string) => {
    const known = days.get(key);
    if (known) return known;
    const fresh = { day: key, talks: 0, answers: 0 };
    days.set(key, fresh);
    return fresh;
  };
  let answers = 0;
  let finished = 0;
  const finishedTopics = new Set<string>();

  for (const session of sessions) {
    const given = session.messages
      .filter((message) => message.speaker === "learner")
      .map((message) => new Date(message.created_at));
    if (given.length === 0) continue;
    answers += given.length;
    for (const at of given) on(dayKey(at)).answers += 1;
    const first = given.reduce((earliest, at) => (at < earliest ? at : earliest));
    on(dayKey(first)).talks += 1;
    if (session.status === "complete") {
      finished += 1;
      finishedTopics.add(session.topic_id);
    }
  }

  return {
    // `YYYY-MM-DD` sorts as text in date order.
    days: [...days.values()].sort((left, right) => right.day.localeCompare(left.day)),
    talks_finished: finished,
    answers,
    finished_topics: [...finishedTopics].sort(),
  };
}

/** The stored learner without the flag, and with the fields a preview from
 * before they existed left out filled in as the backend fills them. */
function publicLearner({ signed_out: _, ...learner }: StoredLearner): Learner {
  return { ...structuredClone(learner), age: learner.age ?? null, avatar_color: learner.avatar_color ?? null };
}

/** Most recently active first; the newer learner first when two tie. */
function byRecentActivity(
  left: MultiLearnerState["learners"][number],
  right: MultiLearnerState["learners"][number],
): number {
  return Date.parse(right.last_active_at) - Date.parse(left.last_active_at) || right.id - left.id;
}

/**
 * The several-learner preview goes back to one learner the way the backend's
 * conversion does it: whoever was signed in stays, signed in, or else whoever
 * was about most recently, signed out. Every session is kept, whoever it was
 * stamped with, so no talk is lost.
 */
function fromMultiLearner(multi: MultiLearnerState): BrowserState {
  const pointed = multi.learners.find((learner) => learner.id === multi.signed_in);
  const kept = pointed ?? multi.learners.slice().sort(byRecentActivity)[0];
  return {
    learner: kept && {
      name: kept.name,
      age: kept.age ?? null,
      level_name: kept.level_name,
      created_at: kept.created_at,
      avatar_color: kept.avatar_color ?? null,
      signed_out: !pointed,
    },
    sessions: (multi.sessions ?? []).map(({ learner_id: _, ...session }) => session),
  };
}

export function createBrowserBridge(storage: StorageLike = window.localStorage): EllaBridge {
  const write = (state: BrowserState) => storage.setItem(storageKey, JSON.stringify(state));

  const read = (): BrowserState => {
    const raw = storage.getItem(storageKey);
    if (!raw) return { sessions: [] };
    const saved = JSON.parse(raw) as StoredState;
    if (Array.isArray(saved.learners)) {
      // Left by the several-learner development preview. Written straight
      // back, so it is converted once and never again.
      const single = fromMultiLearner(saved as MultiLearnerState);
      write(single);
      return single;
    }
    return { learner: saved.learner, sessions: saved.sessions ?? [] };
  };

  /** The learner, unless they have logged out. */
  const signedIn = (state: BrowserState) => (state.learner?.signed_out ? undefined : state.learner);

  /** Same refusal as `start_session` and `start_chore`: a conversation needs
   * someone to belong to. */
  const talker = (state: BrowserState) => {
    const learner = signedIn(state);
    if (!learner) throw new Error("Tell Ella your name before starting a conversation.");
    return learner;
  };

  const snapshot = (state: BrowserState): AppSnapshot => {
    const learner = signedIn(state);
    return {
      learner: learner ? publicLearner(learner) : null,
      // Signed in or not, so the welcome-back step can greet them by name.
      saved_learner: state.learner
        ? { name: state.learner.name, avatar_color: state.learner.avatar_color ?? null }
        : null,
      topics: topicsForAge(learner?.age),
      // Signed out, the talks stay saved but none of them are on show.
      // Sessions are stored in the order they started.
      recent_sessions: learner
        ? state.sessions
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
            }))
        : [],
      progress: learner ? progressOf(state.sessions) : emptyProgress(),
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
    };
  };

  /** A new conversation that has only Ella's (or her stand-in's) opening line. */
  const openSession = (state: BrowserState, topicId: string, label: string, opening: string) => {
    const session: Session = {
      id: crypto.randomUUID(),
      topic_id: topicId,
      topic_label: label,
      status: "active",
      started_at: new Date().toISOString(),
      messages: [
        {
          id: crypto.randomUUID(),
          speaker: "ella",
          content: opening,
          turn: 0,
          created_at: new Date().toISOString(),
        },
      ],
    };
    state.sessions.push(session);
    write(state);
    return structuredClone(session);
  };

  const getSessionOrThrow = (state: BrowserState, sessionId: string) => {
    const session = state.sessions.find((candidate) => candidate.id === sessionId);
    if (!session) throw new Error("That conversation could not be found.");
    return session;
  };

  /** Closes a session and builds its summary. Shared so a conversation that
   * ends on its own last turn and one the learner ends by hand produce the
   * same thing, and worded as `complete_session` words it. */
  const closeSession = (state: BrowserState, sessionId: string): SessionSummary => {
    const session = getSessionOrThrow(state, sessionId);
    session.status = "complete";
    session.completed_at = new Date().toISOString();
    const turns = session.messages.filter((message) => message.speaker === "learner").length;
    const conversations = progressOf(state.sessions).talks_finished;
    return {
      session_id: session.id,
      topic_label: session.topic_label,
      turns,
      headline: turns >= 3 ? "You kept that conversation going" : "Every answer counts",
      // A talk with nothing said in it is not counted, so it is not thanked
      // for as though it were.
      encouragement:
        turns === 0
          ? "Say a few words next time and it counts. Come back and talk to Ella again."
          : `That is ${conversations} conversation${conversations === 1 ? "" : "s"} finished. Come back and talk to Ella again.`,
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
      const clean = cleanName(name);
      if (age != null && (age < 3 || age > 120)) {
        throw new Error("Please enter an age between 3 and 120.");
      }
      const state = read();
      const kept = state.learner;
      state.learner = {
        name: clean,
        // Onboarding can be re-run without the age step; keep what we know.
        age: age ?? kept?.age ?? null,
        level_name: "Morning Meadow",
        created_at: kept?.created_at ?? new Date().toISOString(),
        avatar_color: kept?.avatar_color ?? null,
        // Saving signs in. After a log out, "Let's start" is the same learner
        // onboarding again, so every talk stays theirs.
        signed_out: false,
      };
      write(state);
      return publicLearner(state.learner);
    },
    async logIn() {
      const state = read();
      if (!state.learner) throw new Error("Tell Ella your name first.");
      state.learner.signed_out = false;
      write(state);
      return publicLearner(state.learner);
    },
    async logOut() {
      const state = read();
      // Nothing else changes: logging back in finds every talk where it was.
      if (state.learner) state.learner.signed_out = true;
      write(state);
      return snapshot(state);
    },
    async saveAvatarColor(color) {
      const state = read();
      const learner = signedIn(state);
      if (!learner) throw new Error("Tell Ella your name first.");
      if (!AVATAR_COLOR.test(color)) throw new Error("Please choose one of the colours on the profile screen.");
      learner.avatar_color = color;
      write(state);
      return publicLearner(learner);
    },
    async startSession(topicId) {
      const state = read();
      const topic = topics.find((candidate) => candidate.id === topicId);
      if (!topic) throw new Error("Choose one of the available topics.");
      const learner = talker(state);
      return openSession(state, topic.id, topic.label, openingFor(topic.id, learner.name));
    },
    async startChore(choreId) {
      const state = read();
      const chore = chores.find((candidate) => candidate.id === choreId);
      if (!chore) throw new Error(`No chore called ${choreId}.`);
      talker(state);
      return openSession(state, chore.id, chore.title, chore.opening);
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
