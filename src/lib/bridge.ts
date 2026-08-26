import { invoke } from "@tauri-apps/api/core";
import type {
  AppSnapshot,
  EllaBridge,
  Garden,
  Learner,
  Message,
  Session,
  SessionSummary,
  SkillEvidence,
  SkillProgress,
  Topic,
  TurnResult,
  VoiceTurnInput,
} from "../types";

const topics: Topic[] = [
  {
    id: "school-life",
    label: "School life",
    prompt: "Tell Zoe about a memorable day at school.",
    emoji: "🎒",
    color: "blue",
  },
  {
    id: "food-i-love",
    label: "Food I love",
    prompt: "Describe a meal you would happily eat again.",
    emoji: "🥭",
    color: "green",
  },
  {
    id: "my-dreams",
    label: "My dreams",
    prompt: "Share something you hope to do in the future.",
    emoji: "✨",
    color: "berry",
  },
];

const seedSkills: SkillProgress[] = [
  {
    id: "descriptive-words",
    label: "Use descriptive words",
    strand: "vocabulary",
    evidence_count: 0,
    stage: 0,
    stage_label: "Bare plot",
  },
  {
    id: "past-events",
    label: "Talk about past events",
    strand: "grammar",
    evidence_count: 0,
    stage: 0,
    stage_label: "Bare plot",
  },
  {
    id: "longer-answers",
    label: "Build longer answers",
    strand: "fluency",
    evidence_count: 0,
    stage: 0,
    stage_label: "Bare plot",
  },
];

interface BrowserState {
  learner?: Learner;
  sessions: Session[];
  garden: Garden;
}

export interface StorageLike {
  getItem(key: string): string | null;
  setItem(key: string, value: string): void;
  removeItem(key: string): void;
}

const storageKey = "zospeak-tauri-poc";

function isTauriRuntime(): boolean {
  return typeof window !== "undefined" && Boolean(window.__TAURI_INTERNALS__);
}

class TauriBridge implements EllaBridge {
  bootstrap = () => invoke<AppSnapshot>("bootstrap");
  saveLearner = (name: string) => invoke<Learner>("save_learner", { name });
  startSession = (topicId: string) => invoke<Session>("start_session", { topicId });
  getSession = (sessionId: string) => invoke<Session>("get_session", { sessionId });
  sendTextTurn = (sessionId: string, text: string) =>
    invoke<TurnResult>("send_text_turn", { sessionId, text });
  sendVoiceTurn = ({ sessionId, samples, sampleRate, browserTranscript }: VoiceTurnInput) =>
    invoke<TurnResult>("send_voice_turn", {
      sessionId,
      samples,
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
    return {
      sessions: [],
      garden: {
        level_name: "Morning Meadow",
        total_conversations: 0,
        skills: structuredClone(seedSkills),
      },
    };
  };

  const write = (state: BrowserState) => storage.setItem(storageKey, JSON.stringify(state));

  const snapshot = (state: BrowserState): AppSnapshot => ({
    learner: state.learner,
    topics,
    recent_sessions: state.sessions
      .slice()
      .reverse()
      .slice(0, 5)
      .map((session) => ({
        id: session.id,
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
    garden: state.garden,
  });

  const getSessionOrThrow = (state: BrowserState, sessionId: string) => {
    const session = state.sessions.find((candidate) => candidate.id === sessionId);
    if (!session) throw new Error("That conversation could not be found.");
    return session;
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
    const zoeMessage: Message = {
      id: crypto.randomUUID(),
      speaker: "zoe",
      content: demoReply(session.topic_id, clean, turn),
      turn,
      created_at: new Date().toISOString(),
    };
    session.messages.push(learnerMessage, zoeMessage);

    const skill = state.garden.skills[(turn - 1) % state.garden.skills.length];
    skill.evidence_count += 1;
    skill.stage = Math.min(3, skill.evidence_count) as SkillProgress["stage"];
    skill.stage_label = stageLabel(skill.stage);
    skill.last_evidence = clean;
    const evidence: SkillEvidence = {
      skill_id: skill.id,
      skill_label: skill.label,
      strand: skill.strand,
      new_stage: skill.stage,
      stage_label: skill.stage_label,
      evidence: clean,
    };
    write(state);
    return {
      learner_message: learnerMessage,
      zoe_message: zoeMessage,
      correction: gentleCorrection(clean),
      evidence,
      suggested_complete: turn >= 3,
    };
  };

  return {
    async bootstrap() {
      return snapshot(read());
    },
    async saveLearner(name) {
      const clean = name.trim();
      if (clean.length < 2) throw new Error("Please enter at least two letters.");
      const state = read();
      state.learner = {
        name: clean,
        level_name: "Morning Meadow",
        created_at: new Date().toISOString(),
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
            speaker: "zoe",
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
      const session = getSessionOrThrow(state, sessionId);
      session.status = "complete";
      session.completed_at = new Date().toISOString();
      state.garden.total_conversations += 1;
      const learners = session.messages.filter((message) => message.speaker === "learner");
      const lastEvidence = state.garden.skills
        .filter((skill) => skill.last_evidence)
        .sort((left, right) => right.evidence_count - left.evidence_count)[0];
      write(state);
      return {
        session_id: session.id,
        topic_label: session.topic_label,
        turns: learners.length,
        headline: learners.length >= 3 ? "Your garden grew!" : "Every word waters the garden.",
        encouragement: `You kept a real conversation about ${session.topic_label.toLowerCase()} going. That is brave practice.`,
        best_evidence: lastEvidence
          ? {
              skill_id: lastEvidence.id,
              skill_label: lastEvidence.label,
              strand: lastEvidence.strand,
              new_stage: lastEvidence.stage,
              stage_label: lastEvidence.stage_label,
              evidence: lastEvidence.last_evidence ?? "",
            }
          : null,
        garden: structuredClone(state.garden),
      };
    },
    async resetDemoData() {
      storage.removeItem(storageKey);
      return snapshot(read());
    },
  };
}

export const bridge: EllaBridge = isTauriRuntime() ? new TauriBridge() : createBrowserBridge();

function openingFor(topicId: string, name: string): string {
  if (topicId === "food-i-love") return `Hi ${name}! Imagine your favourite meal is right here. What would be on the plate?`;
  if (topicId === "my-dreams") return `Hi ${name}! Let’s dream a little. What is something you really want to do one day?`;
  return `Hi ${name}! Tell me about a school day you still remember. What happened?`;
}

function demoReply(topicId: string, text: string, turn: number): string {
  const lead = text.split(/\s+/).slice(0, 4).join(" ");
  if (turn >= 3) return `I enjoyed hearing that, especially “${lead}”. Before we finish, what feeling does this story give you?`;
  if (topicId === "food-i-love") return `That sounds delicious! You said “${lead}”. Who would you like to share that meal with, and why?`;
  if (topicId === "my-dreams") return `That is a wonderful goal. What is one small step you could take toward it this year?`;
  return `I can picture that! What happened next, and how did you feel?`;
}

function gentleCorrection(text: string): string | null {
  if (/\bi goed\b/i.test(text)) return "Try “I went” instead of “I goed.”";
  if (/\bi am went\b/i.test(text)) return "Try “I went” when you are talking about the past.";
  return null;
}

function stageLabel(stage: SkillProgress["stage"]): SkillProgress["stage_label"] {
  return (["Bare plot", "Seedling", "Young plant", "Bloom"] as const)[stage];
}
