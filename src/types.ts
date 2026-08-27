export type Speaker = "learner" | "ella";
export type SkillStrand = "vocabulary" | "grammar" | "fluency";

export interface Learner {
  name: string;
  /** Collected during onboarding so Ella can pick age-appropriate topics. */
  age?: number | null;
  level_name: string;
  created_at: string;
}

export interface Topic {
  id: string;
  label: string;
  prompt: string;
  emoji: string;
  /** Palette hint from the backend; the home bento uses `Tone` from below. */
  color: string;
}

export interface Message {
  id: string;
  speaker: Speaker;
  content: string;
  turn: number;
  created_at: string;
}

export interface Session {
  id: string;
  topic_id: string;
  topic_label: string;
  status: "active" | "complete";
  started_at: string;
  completed_at?: string | null;
  messages: Message[];
}

export interface SessionListItem {
  id: string;
  topic_id: string;
  topic_label: string;
  status: "active" | "complete";
  started_at: string;
  message_count: number;
}

export interface EngineComponent {
  name: string;
  ready: boolean;
  detail: string;
}

export interface EngineStatus {
  mode: "demo" | "local";
  label: string;
  ready: boolean;
  components: EngineComponent[];
}

export interface AppSnapshot {
  learner?: Learner | null;
  topics: Topic[];
  recent_sessions: SessionListItem[];
  engine_status: EngineStatus;
}

export interface AudioPayload {
  mime_type: string;
  base64: string;
}

/** What the live chore progress bar draws. Only sent for ledger chores. */
export interface LedgerView {
  unit: string;
  current: number;
  target: number;
  opening: number;
  /** 0-1, from the chore's opening figure to its target. */
  progress: number;
  agreed: boolean;
  reached_target: boolean;
  regenerated: boolean;
}

/** How a chore character signed off. */
export type TurnSignal = "deal" | "walk";

export interface TurnResult {
  learner_message: Message;
  ella_message: Message;
  correction?: string | null;
  suggested_complete: boolean;
  audio?: AudioPayload | null;
  timings?: TurnTimings | null;
  ledger?: LedgerView | null;
  signal?: TurnSignal | null;
}

export interface TurnTimings {
  interaction_id: string;
  kind: "voice" | "text";
  audio_input_ms?: number | null;
  audio_after_vad_ms?: number | null;
  vad_ms?: number | null;
  stt_ms?: number | null;
  stt_engine?: string | null;
  stt_backend?: string | null;
  stt_fallback_from?: string | null;
  stt_mel_ms?: number | null;
  stt_encode_ms?: number | null;
  stt_decode_ms?: number | null;
  llm_ttft_ms?: number | null;
  llm_completion_ms?: number | null;
  tts_first_audio_ms?: number | null;
  tts_completion_ms?: number | null;
  total_ms: number;
}

export interface SessionSummary {
  session_id: string;
  topic_label: string;
  turns: number;
  headline: string;
  encouragement: string;
}

export interface VoiceTurnInput {
  sessionId: string;
  samples: number[];
  sampleRate: number;
  browserTranscript?: string;
}

export interface VoiceStreamFinishInput {
  streamId: string;
  tailSamples: number[];
  sampleRate: number;
  browserTranscript?: string;
}

export interface EllaBridge {
  bootstrap(): Promise<AppSnapshot>;
  saveLearner(name: string, age?: number | null): Promise<Learner>;
  startSession(topicId: string): Promise<Session>;
  getSession(sessionId: string): Promise<Session>;
  sendTextTurn(sessionId: string, text: string): Promise<TurnResult>;
  sendVoiceTurn(input: VoiceTurnInput): Promise<TurnResult>;
  /** Live chunked STT while recording; only implemented by the Tauri bridge. */
  beginVoiceStream?(sessionId: string): Promise<string>;
  pushVoiceStream?(streamId: string, samples: number[], sampleRate: number): Promise<void>;
  cancelVoiceStream?(streamId: string): Promise<void>;
  finishVoiceStreamTurn?(input: VoiceStreamFinishInput): Promise<TurnResult>;
  completeSession(sessionId: string): Promise<SessionSummary>;
  resetDemoData(): Promise<AppSnapshot>;
}

/* ------------------------------------------------------------------ *
 * Presentation layer
 *
 * The Ella v5 design shows curriculum framing the Rust backend does not
 * model yet: a CEFR level, a talking streak, named garden units on a path,
 * per-topic category + duration, and a weekly digest. Everything below is
 * derived from `AppSnapshot` where the data exists and filled from the
 * placeholders in `lib/presentation.ts` where it does not.
 * ------------------------------------------------------------------ */

export type TopicCategory = "role-play" | "vocabulary" | "grammar" | "fluency";

/** How a topic renders in the home bento grid. */
export type TopicSlot = "wide" | "wave" | "framed" | "inset" | "chat" | "quote";

export type Tone = "violet" | "pink" | "green" | "orange" | "lilac" | "ink";

export interface TopicPresentation {
  category: TopicCategory;
  minutes: number;
  tone: Tone;
  /** Longer line used by the "Ella recommends" hero. */
  blurb: string;
  /** Sample exchange printed on the framed/chat/quote cards. */
  sample: string;
  reply: string;
}

export type StreakDayState = "done" | "today" | "future";

export interface StreakDay {
  label: string;
  state: StreakDayState;
}

export interface Streak {
  days: number;
  week: StreakDay[];
}


export interface WeeklyDigest {
  talks: number;
}

/** What the onboarding placement talk hands back once it has really run. */
export interface PlacementResult {
  level: string;
}

export interface LevelInfo {
  /** CEFR band shown next to the learner's name. */
  code: string;
}
