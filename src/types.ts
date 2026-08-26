export type Speaker = "learner" | "zoe";
export type SkillStrand = "vocabulary" | "grammar" | "fluency";

export interface Learner {
  name: string;
  level_name: string;
  created_at: string;
}

export interface Topic {
  id: string;
  label: string;
  prompt: string;
  emoji: string;
  color: "berry" | "green" | "blue";
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

export interface SkillProgress {
  id: string;
  label: string;
  strand: SkillStrand;
  evidence_count: number;
  stage: 0 | 1 | 2 | 3;
  stage_label: "Bare plot" | "Seedling" | "Young plant" | "Bloom";
  last_evidence?: string | null;
}

export interface Garden {
  level_name: string;
  total_conversations: number;
  skills: SkillProgress[];
}

export interface AppSnapshot {
  learner?: Learner | null;
  topics: Topic[];
  recent_sessions: SessionListItem[];
  engine_status: EngineStatus;
  garden: Garden;
}

export interface AudioPayload {
  mime_type: string;
  base64: string;
}

export interface SkillEvidence {
  skill_id: string;
  skill_label: string;
  strand: SkillStrand;
  new_stage: 0 | 1 | 2 | 3;
  stage_label: string;
  evidence: string;
}

export interface TurnResult {
  learner_message: Message;
  zoe_message: Message;
  correction?: string | null;
  evidence?: SkillEvidence | null;
  suggested_complete: boolean;
  audio?: AudioPayload | null;
  timings?: TurnTimings | null;
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
  best_evidence?: SkillEvidence | null;
  garden: Garden;
}

export interface VoiceTurnInput {
  sessionId: string;
  samples: number[];
  sampleRate: number;
  browserTranscript?: string;
}

export interface EllaBridge {
  bootstrap(): Promise<AppSnapshot>;
  saveLearner(name: string): Promise<Learner>;
  startSession(topicId: string): Promise<Session>;
  getSession(sessionId: string): Promise<Session>;
  sendTextTurn(sessionId: string, text: string): Promise<TurnResult>;
  sendVoiceTurn(input: VoiceTurnInput): Promise<TurnResult>;
  completeSession(sessionId: string): Promise<SessionSummary>;
  resetDemoData(): Promise<AppSnapshot>;
}
