export type Speaker = "learner" | "ella";
export type SkillStrand = "vocabulary" | "grammar" | "fluency";

/** The one learner a laptop keeps. */
export interface Learner {
  name: string;
  /** Collected during onboarding so Ella can pick age-appropriate topics. */
  age?: number | null;
  level_name: string;
  created_at: string;
  /** `#RRGGBB`, or null until the learner picks one on their profile. */
  avatar_color?: string | null;
}

/** Whoever is saved on this laptop, as the welcome-back step greets them. */
export interface LearnerProfile {
  name: string;
  avatar_color?: string | null;
}

/** One local calendar day on which the learner gave at least one answer. */
export interface DayActivity {
  /** `YYYY-MM-DD` on the learner's own clock. */
  day: string;
  /** Talks whose first answer fell on this day, so a talk that ran past
   * midnight is counted once, not on both days. */
  talks: number;
  /** Answers given this day, in any talk. */
  answers: number;
  /** How long this day's spoken answers lasted. Typed answers, and spoken
   * ones from before Ella kept their length, add nothing. */
  spoken_ms: number;
}

/** One finished talk, as the badges read them. */
export interface FinishedTalk {
  /** The topic, chore or placement (`placement`) it was. */
  topic_id: string;
  /** `YYYY-MM-DD` it ended on, on the learner's own clock. */
  day: string;
  /** A ledger chore whose character agreed at or past the target. */
  goal_met: boolean;
}

/** What the signed-in learner has done, over their whole history. */
export interface LearnerProgress {
  /** Every day they talked on, newest first. Not capped. */
  days: DayActivity[];
  /** Talks that were finished with at least one answer in them. */
  talks_finished: number;
  /** Every answer given, in finished talks or not. */
  answers: number;
  /** Topic and chore ids of the finished talks, each once. */
  finished_topics: string[];
  /** Ledger chores whose goal a finished talk met, each once. */
  chores_met: string[];
  /** Every finished talk, the first to end first. */
  talks: FinishedTalk[];
  /** How long the spoken answers lasted, over the `spoken_answers` whose
   * length was kept. None kept means not known yet, not nothing said. */
  spoken_ms: number;
  spoken_answers: number;
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
  /** The learner while signed in; null when signed out or nobody is saved. */
  learner?: Learner | null;
  /** Whoever is saved on this laptop, signed in or not; null on a laptop
   * nobody has used yet. */
  saved_learner?: LearnerProfile | null;
  topics: Topic[];
  /** The signed-in learner's five newest talks, newest first. Empty when
   * signed out. */
  recent_sessions: SessionListItem[];
  /** All zero and empty when nobody is signed in. */
  progress: LearnerProgress;
  /** Where the signed-in learner stands in the curriculum; null when signed out. */
  standing?: Standing | null;
  engine_status: EngineStatus;
}

/**
 * Where the learner stands in the curriculum: six levels of five steps, as on
 * Ella Mobile. The window shows a level's name and its number on the ladder,
 * never its CEFR code, which stays in the backend.
 */
export interface Standing {
  /** 1 to `level_count`. */
  level_number: number;
  level_count: number;
  level_name: string;
  /** 1 to `step_count`. */
  step: number;
  step_count: number;
  step_title: string;
  /** How far through the level, 0-100: the steps behind and this step's share. */
  percent: number;
  /** False until a placement chat has read a level for them. A learner can
   * move on through talks without one; the level map offers it until then. */
  placed: boolean;
}

/** How far a talk moved the learner. */
export type Advance = "step" | "level";

/** One skill a talk counted: its short label and how many talks have shown it. */
export interface SkillGrowth {
  label: string;
  count: number;
}

/** What a finished talk did for the learner. Worked out once by the backend
 * and kept on the talk, so asking again answers the same. */
export interface Assessment {
  session_id: string;
  kind: "placement" | "talk";
  /** Where the learner stands after this talk. */
  standing: Standing;
  /** A placement's closing line, for its result. */
  closing?: string | null;
  /** A step or level this talk finished. */
  advanced?: Advance | null;
  /** Skills the talk counted, its aim first, at most three. */
  skills: SkillGrowth[];
  /** False when nothing judged the talk: nothing said, or no model to judge. */
  scored: boolean;
  /** Ella's notes for the recap; null for a placement, and for a talk too
   * short to say anything about (`SessionSummary.short`). */
  notes?: TalkNotes | null;
}

/** What the recap says about a talk. */
export interface TalkNotes {
  /** At most two short lines, read off what the learner said. */
  went_well: string[];
  /** One of the learner's own phrases and how to say it. */
  fix?: Fix | null;
  /** Whether a model looked for a mistake; without one, no fix does not mean
   * nothing to fix. */
  checked: boolean;
}

export interface Fix {
  /** Word for word what the learner said. */
  said: string;
  better: string;
}

export type LevelState = "done" | "current" | "next" | "locked";

export interface LevelSkill {
  label: string;
  /** "I can …", word for word from the curriculum. */
  text: string;
  passed: boolean;
}

export interface LevelStep {
  number: number;
  title: string;
  focus: string;
  skills: LevelSkill[];
}

/** One level of the ladder as it stands for the learner. */
export interface LevelView {
  number: number;
  name: string;
  /** The level's "I can …" headline. */
  goal: string;
  state: LevelState;
  percent: number;
  steps: LevelStep[];
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

/** When one word of a reply is spoken, relative to the start of its clip. */
export interface WordSpan {
  text: string;
  start_ms: number;
  end_ms: number;
}

/** How long one token Piper spoke lasted, relative to the start of its clip:
 * a sound, a stress mark, a word space, the blank after every token, or a
 * sentence's start (`^`) or end (`$`). From the same inference as the audio,
 * so Ella's mouth can follow what she really says. */
export interface PhonemeSpan {
  phoneme: string;
  start_ms: number;
  end_ms: number;
}

/** One sentence of a reply, synthesized and pushed while the rest of the turn
 * is still being written. Playback starts on the first of these. */
export interface SpeechSegment {
  session_id: string;
  turn: number;
  /** 0-based playback order. Segments must be played in this order. */
  index: number;
  text: string;
  audio: AudioPayload;
  /** Milliseconds from the start of the generation to this segment. */
  ready_ms: number;
  /** When each word of `text` is spoken, from the start of `audio`. */
  words: WordSpan[];
  /** Every token of `audio`, in order. Empty or missing when the voice cannot
   * time them, and then her mouth only opens while she talks. */
  phonemes?: PhonemeSpan[];
  /** The whole reply, on a reply's first sentence only: its text is settled
   * before any of it is sent, so it can be shown whole as she starts. */
  reply?: string | null;
}

/** The result of speaking a line the app already had — Ella's opening. Carries
 * the same three things a turn does, so the screen highlights it and the replay
 * button can say it again, exactly as it does for a reply. */
export interface SpokenLine {
  audio?: AudioPayload | null;
  speech_words: WordSpan[];
  /** Every token of `audio`, for her mouth on a replay. */
  speech_phonemes?: PhonemeSpan[];
  streamed_segments: number;
}

export interface TurnResult {
  learner_message: Message;
  ella_message: Message;
  correction?: string | null;
  suggested_complete: boolean;
  /** Set once the conversation is over and the backend has already closed the
   * session. The shell shows the summary instead of asking for another turn. */
  session_summary?: SessionSummary | null;
  audio?: AudioPayload | null;
  timings?: TurnTimings | null;
  ledger?: LedgerView | null;
  signal?: TurnSignal | null;
  /** How many `SpeechSegment`s this turn already sent. Above zero, the reply
   * has been playing since before this result arrived, and `audio` is the same
   * recording kept for the replay button — playing it again repeats the turn. */
  streamed_segments: number;
  /** When each word of the whole reply is spoken, from the start of `audio`. */
  speech_words: WordSpan[];
  /** Every token of the whole reply, from the start of `audio`. */
  speech_phonemes?: PhonemeSpan[];
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
  /** When Ella started to speak, from the start of the turn. */
  speech_ms?: number | null;
  total_ms: number;
}

export interface SessionSummary {
  session_id: string;
  topic_label: string;
  turns: number;
  headline: string;
  encouragement: string;
  /** Too little said for notes, known before the assessment is. */
  short: boolean;
  /** How a ledger chore ended; null for a free talk and a rubric chore. */
  chore?: ChoreRecap | null;
}

/** How a ledger chore ended, for the recap's role-play band. */
export interface ChoreRecap {
  chore_id: string;
  character_id: string;
  unit: string;
  /** Which way the learner pushed the figure: a price down, a refund up. */
  direction: "down" | "up";
  target: number;
  /** Where the figure stood when the talk ended. */
  figure: number;
  agreed: boolean;
  /** Agreed, at or past the target. */
  met: boolean;
  /** Finished talks that met this goal, this one included. */
  times_met: number;
  /** The character's last line. */
  last_line?: string | null;
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
  /** Saves the laptop's learner and signs them in. After a log out this is
   * the same learner onboarding again, so their talks stay theirs. */
  saveLearner(name: string, age?: number | null): Promise<Learner>;
  /** Signs the saved learner back in. Refused when nobody is saved yet. */
  logIn(): Promise<Learner>;
  /** Signs out. Deletes nothing: logging back in finds every talk again. */
  logOut(): Promise<AppSnapshot>;
  /** Stores a `#RRGGBB` avatar colour on the signed-in learner. */
  saveAvatarColor(color: string): Promise<Learner>;
  startSession(topicId: string): Promise<Session>;
  /** The placement chat: a friendly first talk that ends once Ella has heard
   * enough to read a level, between five answers and twelve. Its last turn
   * closes the session, as a conversation's last turn does. */
  startPlacement(): Promise<Session>;
  /** What a finished talk did: a placement's level, or a talk's skills and any
   * step or level it finished. Slow the first time (the model reads the talk),
   * then kept. Rejects while the talk is still going, or when the model could
   * not be read — asking again later is safe. */
  assessSession(sessionId: string): Promise<Assessment>;
  /** Every level, lowest first, as it stands for the learner. */
  levels(): Promise<LevelView[]>;
  /** Start a talk partner's goal: a chore from the Rust catalog, played by its
   * character and scored by the backend. The session reads like any other. */
  startChore(choreId: string): Promise<Session>;
  /** Say Ella's opening aloud, streaming it like a reply. Tauri bridge only;
   * in the browser the opening falls back to system speech. */
  speakOpening?(sessionId: string): Promise<SpokenLine>;
  /** Said aloud when a voice turn came back with no words at all, so the
   * learner hears that Ella missed them instead of only reading it. Tauri
   * bridge only; in the browser this falls back to system speech. */
  speakRetryPrompt?(sessionId: string): Promise<SpokenLine>;
  /** The recap's fix said aloud, from the fix kept on the talk. Tauri bridge
   * only; in the browser it falls back to system speech. */
  speakFix?(sessionId: string): Promise<SpokenLine>;
  getSession(sessionId: string): Promise<Session>;
  sendTextTurn(sessionId: string, text: string): Promise<TurnResult>;
  sendVoiceTurn(input: VoiceTurnInput): Promise<TurnResult>;
  /** Sentence audio arriving mid-turn. Resolves to an unsubscribe function.
   * Only the Tauri bridge streams; in the browser a turn speaks when it ends. */
  onSpeechSegment?(handler: (segment: SpeechSegment) => void): Promise<() => void>;
  /** Live chunked STT while recording; only implemented by the Tauri bridge. */
  beginVoiceStream?(sessionId: string): Promise<string>;
  pushVoiceStream?(streamId: string, samples: number[], sampleRate: number): Promise<void>;
  cancelVoiceStream?(streamId: string): Promise<void>;
  finishVoiceStreamTurn?(input: VoiceStreamFinishInput): Promise<TurnResult>;
  completeSession(sessionId: string): Promise<SessionSummary>;
}

/* ------------------------------------------------------------------ *
 * Presentation layer
 *
 * The Ella Desktop design shows framing the Rust backend does not model
 * yet: per-topic category and duration, badges, and a cast of talk
 * partners. Everything below is derived from `AppSnapshot` where the data
 * exists (the streak and badges from the learner's `progress`) and filled
 * from the editorial tables in `lib/presentation.ts` where it does not.
 * ------------------------------------------------------------------ */

export type TopicCategory = "role-play" | "vocabulary" | "grammar" | "fluency";

export type Tone = "violet" | "pink" | "green" | "orange" | "ink";

export interface TopicPresentation {
  category: TopicCategory;
  minutes: number;
  /** Longer line used by the "Today's talk" card. */
  blurb: string;
  /** Something the other side of the scene might say, printed on the tall topic card. */
  sample: string;
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

/** Talks, the answers given in them and how long the spoken ones lasted,
 * over whatever window was asked for. */
export interface TalkTally {
  talks: number;
  answers: number;
  spokenMs: number;
}

/** The glyph a badge is drawn with, by the design's names for them. */
export type BadgeGlyph = "ella" | "mic" | "tag" | "house" | "flame" | "star";

/** What a badge's mic starts: its partner's scenes on Talk partners, the
 * placement chat, or a talk (today's when the topic is null). */
export type BadgeStart =
  | { kind: "partners" }
  | { kind: "placement" }
  | { kind: "topic"; topicId: string | null };

/** A count a badge waits on: a streak's days or a number of talks. */
export interface BadgeGoal {
  have: number;
  of: number;
  unit: "days" | "talks";
}

/** A badge as it stands for the learner. */
export interface LearnerBadge {
  id: string;
  name: string;
  /** The ladder level it is listed at, 1 to 6; null for the anytime badges. */
  level: number | null;
  /** `#RRGGBB`: its disc once earned, its ring and glyph until then. */
  color: string;
  glyph: BadgeGlyph;
  /** What earns it, as its sheet says. */
  how: string;
  /** Where it is earned, on its row: "Bippo · Talk a stall price down". */
  where: string;
  /** And on its sheet, as a title and a line: "Bippo", "Talk a stall price
   * down". */
  place: string;
  line: string;
  /** The talk partner whose scene earns it, drawn on its sheet. */
  partner: CastId | null;
  minutes: number | null;
  start: BadgeStart;
  earned: boolean;
  /** `YYYY-MM-DD` it was first earned; null until it is. */
  earnedOn: string | null;
  /** Finished talks of its scene, goal met or not. */
  tries: number;
  goal: BadgeGoal | null;
}

export type CastId = "stall-owner" | "landlord" | "doctor" | "debater";

/** What pressing a goal starts: a scored chore, a free topic, or nothing yet. */
export type CastGoalStart =
  | { kind: "chore"; choreId: string }
  | { kind: "topic"; topicId: string }
  | { kind: "soon" };

export interface CastGoal {
  id: string;
  title: string;
  /** Shown to the learner: what counts as walking away happy. */
  goal: string;
  /** The mono label's first half, e.g. `NEGOTIATION`. */
  track: string;
  minutes: number;
  /** Mirrors the chore's `min_age`; younger learners are not offered it. */
  minAge: number;
  start: CastGoalStart;
}

export interface CastMember {
  id: CastId;
  name: string;
  blurb: string;
  goals: CastGoal[];
}

