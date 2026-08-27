import { FormEvent, Fragment, useEffect, useRef, useState } from "react";
import { LoaderCircle, Send } from "lucide-react";
import {
  EllaMascot,
  SpeakingWave,
  ThinkingDots,
  VoiceMeter,
  type EllaReaction,
  type EllaState,
} from "./EllaMascot";
import { MicGlyph } from "./HomeScreen";
import { bridge } from "../lib/bridge";
import {
  createSpeechQueue,
  createVoiceCapture,
  sentenceGroups,
  speakText,
  type SpeechQueue,
  type SpeechQueueCallbacks,
} from "../lib/speech";
import { llog, logServerTimings, markTurnStart, turnElapsed } from "../lib/latency";
import type {
  AudioPayload,
  Session,
  SessionSummary,
  SpokenLine,
  TurnResult,
  WordSpan,
} from "../types";

/** Anything that can be played with its word timings: a turn, or the opening. */
type Playback = { audio?: AudioPayload | null; speech_words: WordSpan[] };

const MIC_HINT: Record<EllaState, string> = {
  resting: "Tap to speak",
  listening: "Listening… tap when you finish",
  thinking: "Ella is thinking…",
  speaking: "Tap to interrupt and speak",
};

export function TalkScreen({
  session,
  onSessionChange,
  onComplete,
}: {
  session: Session;
  onSessionChange: (session: Session) => void;
  onComplete: (summary: SessionSummary) => void;
}) {
  const [state, setState] = useState<EllaState>("resting");
  const [input, setInput] = useState("");
  const [typing, setTyping] = useState(false);
  const [liveTranscript, setLiveTranscript] = useState("");
  const [voiceLevel, setVoiceLevel] = useState(0);
  const [sending, setSending] = useState(false);
  const [micStarting, setMicStarting] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [lastTurn, setLastTurn] = useState<TurnResult | null>(null);
  const [reaction, setReaction] = useState<EllaReaction>(null);
  // Ella's words in speaking order, and which one she is on. Both arrive with
  // the audio, sentence by sentence, so the reply appears as it is spoken
  // instead of all at once when the turn returns.
  // Ella's reply grouped by sentence, and which word she is on. Sentences are
  // kept apart because each one arrives whole: given its own line, an arriving
  // sentence cannot re-centre the ones already on screen.
  const [spokenSentences, setSpokenSentences] = useState<string[][]>([]);
  const [spokenIndex, setSpokenIndex] = useState(-1);
  // Ella's opening, kept so "Hear it again" can replay it with its timings
  // instead of dropping to the system voice.
  const [openingLine, setOpeningLine] = useState<SpokenLine | null>(null);

  const voice = useRef(createVoiceCapture());
  const voiceStreamId = useRef<string | null>(null);
  const stopSpeech = useRef<() => void>(() => undefined);
  const playbackWatchdog = useRef<number | null>(null);
  const playbackGeneration = useRef(0);
  /** Receives sentences while the reply is still generating. Armed for the
   * duration of one turn, and tied to the playback generation so a learner who
   * interrupts is not spoken over by the turn they cut off. */
  const speechQueue = useRef<{ queue: SpeechQueue; generation: number } | null>(null);
  const reactionTimer = useRef<number | null>(null);
  const micOperation = useRef(0);
  const captureActive = useRef(false);
  const voicePushQueue = useRef<Promise<void>>(Promise.resolve());
  const voicePushFailed = useRef(false);
  const micButton = useRef<HTMLButtonElement>(null);
  const focusMicAfterModeSwitch = useRef(false);
  const mounted = useRef(true);

  const latestElla = [...session.messages].reverse().find((message) => message.speaker === "ella");

  async function cancelVoiceStream() {
    const streamId = voiceStreamId.current;
    voiceStreamId.current = null;
    if (streamId) {
      await voicePushQueue.current.catch(() => undefined);
      await bridge.cancelVoiceStream?.(streamId).catch(() => undefined);
    }
  }

  useEffect(() => {
    mounted.current = true;
    if (session.messages.length === 1 && latestElla) speakOpening(latestElla.content);
    return () => {
      mounted.current = false;
      micOperation.current += 1;
      stopPlayback();
      if (reactionTimer.current) window.clearTimeout(reactionTimer.current);
      void cancelVoiceStream();
      void voice.current.cancel();
    };
    // The opening should play once for a newly mounted conversation.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // Sentences arrive here as Piper finishes them, well before the turn result
  // does. Subscribing on mount means nothing is missed between arming the queue
  // and the first segment landing.
  useEffect(() => {
    let unsubscribe: (() => void) | undefined;
    let dropped = false;
    void bridge
      .onSpeechSegment?.((segment) => {
        const armed = speechQueue.current;
        if (!armed || armed.generation !== playbackGeneration.current) return;
        if (segment.session_id !== session.id) return;
        // Segments arrive in order on one channel; an index already queued is a
        // repeat, and queueing it would say the sentence twice.
        if (segment.index < armed.queue.received) return;
        armed.queue.push(segment.audio, segment.words);
      })
      .then((stop) => {
        if (dropped) stop();
        else unsubscribe = stop;
      })
      .catch(() => undefined);
    return () => {
      dropped = true;
      unsubscribe?.();
    };
  }, [session.id]);

  useEffect(() => {
    if (!typing && focusMicAfterModeSwitch.current) {
      focusMicAfterModeSwitch.current = false;
      micButton.current?.focus();
    }
  }, [typing]);

  function stopPlayback() {
    playbackGeneration.current += 1;
    stopSpeech.current();
    stopSpeech.current = () => undefined;
    speechQueue.current?.queue.cancel();
    speechQueue.current = null;
    if (playbackWatchdog.current !== null) window.clearTimeout(playbackWatchdog.current);
    playbackWatchdog.current = null;
  }

  /**
   * Say Ella's opening aloud.
   *
   * The text is already on screen, so unlike a reply there is nothing to
   * reveal — but it goes through the same queue so that it is spoken by Piper
   * rather than the system voice, and highlighted word by word like everything
   * after it. Falls back to `playElla` wherever the backend cannot speak.
   */
  function speakOpening(text: string) {
    if (!bridge.speakOpening || !bridge.onSpeechSegment) {
      playElla(text);
      return;
    }
    stopPlayback();
    setSpokenIndex(-1);
    setError(null);
    setState("speaking");
    const generation = playbackGeneration.current;
    const queue = createSpeechQueue(queueCallbacks(generation, "opening:first-sentence"));
    speechQueue.current = { generation, queue };
    void bridge
      .speakOpening(session.id)
      .then((line) => {
        if (!mounted.current || playbackGeneration.current !== generation) return;
        // Keep it for the replay button, which otherwise has no audio for the
        // opening and would fall back to the system voice.
        setOpeningLine(line);
        if (line.streamed_segments > 0 && queue.received > 0) {
          queue.finish(line.streamed_segments);
          return;
        }
        // Nothing streamed: play the recording, or the system voice.
        playElla(text, line.audio ? line : undefined);
      })
      .catch(() => {
        if (!mounted.current || playbackGeneration.current !== generation) return;
        playElla(text);
      });
  }

  /** What every queue on this screen reports back, live turn or replay alike. */
  function queueCallbacks(generation: number, label: string): SpeechQueueCallbacks {
    const live = (): boolean => mounted.current && playbackGeneration.current === generation;
    const rest = () => {
      if (!live()) return;
      if (playbackWatchdog.current !== null) window.clearTimeout(playbackWatchdog.current);
      playbackWatchdog.current = null;
      setState("resting");
      setSpokenIndex(-1);
    };
    return {
      onSentences: (sentences) => live() && setSpokenSentences(sentences),
      onSpokenWord: (index) => live() && setSpokenIndex(index),
      onStart: () => {
        if (!live()) return;
        setState("speaking");
        llog(label, `END-TO-END -> Ella starts speaking: ${turnElapsed().toFixed(1)}ms`);
      },
      onEnd: rest,
      onError: () => {
        rest();
        if (!live()) return;
        flashReaction("error", 1800);
        setError("Ella could not play this aloud. You can still read her message.");
      },
    };
  }

  /**
   * Open the queue that plays this turn sentence by sentence.
   *
   * Called before the turn is sent, because the first sentence is synthesized
   * while the model is still writing the rest of the reply — by the time the
   * result comes back, Ella has been talking for seconds.
   */
  function armSpeechQueue() {
    if (!bridge.onSpeechSegment) return;
    stopPlayback();
    // Drop the highlight, but leave the words: the previous reply stays on
    // screen while Ella thinks, and clearing the grouping re-rendered that same
    // text as one paragraph instead of one line per sentence — so the subtitles
    // visibly re-centred the moment the learner stopped recording. Nothing can
    // be highlighted in the meantime: the index is cleared here, and drawing
    // the states at all requires that a clip be playing.
    setSpokenIndex(-1);
    const generation = playbackGeneration.current;
    speechQueue.current = {
      generation,
      queue: createSpeechQueue(queueCallbacks(generation, "turn:first-sentence")),
    };
  }

  function flashReaction(next: Exclude<EllaReaction, null>, duration = 1300) {
    if (reactionTimer.current !== null) window.clearTimeout(reactionTimer.current);
    setReaction(next);
    reactionTimer.current = window.setTimeout(() => {
      reactionTimer.current = null;
      if (mounted.current) setReaction(null);
    }, duration);
  }

  function playElla(text: string, result?: Playback) {
    stopPlayback();
    const generation = playbackGeneration.current;
    let playbackSettled = false;
    setError(null);
    setState("speaking");
    // Recorded audio goes through the same queue as a live turn, so hearing a
    // reply again highlights the words the same way saying it the first time
    // did. Only the browser-speech fallback below has no timings to follow.
    if (result?.audio) {
      setSpokenIndex(-1);
      const queue = createSpeechQueue(queueCallbacks(generation, "playback:replay"));
      speechQueue.current = { generation, queue };
      queue.push(result.audio, result.speech_words);
      queue.finish(1);
      return;
    }
    try {
      const cancel = speakText(text, result?.audio, {
        onStart: () => {
          if (mounted.current && playbackGeneration.current === generation) setState("speaking");
        },
        onEnd: () => {
          playbackSettled = true;
          if (!mounted.current || playbackGeneration.current !== generation) return;
          if (playbackWatchdog.current !== null) window.clearTimeout(playbackWatchdog.current);
          playbackWatchdog.current = null;
          setState("resting");
        },
        onError: () => {
          playbackSettled = true;
          if (!mounted.current || playbackGeneration.current !== generation) return;
          if (playbackWatchdog.current !== null) window.clearTimeout(playbackWatchdog.current);
          playbackWatchdog.current = null;
          setState("resting");
          flashReaction("error", 1800);
          setError("Ella could not play this aloud. You can still read her message.");
        },
      });
      stopSpeech.current = playbackSettled ? () => undefined : cancel;
    } catch {
      playbackSettled = true;
      setState("resting");
      flashReaction("error", 1800);
      setError("Ella could not play this aloud. You can still read her message.");
    }
    if (playbackSettled) return;
    // Defensive fallback for platform speech engines that never dispatch `end`.
    playbackWatchdog.current = window.setTimeout(
      () => {
        if (mounted.current && playbackGeneration.current === generation) setState("resting");
      },
      Math.max(15_000, Math.min(45_000, text.length * 180)),
    );
  }

  async function beginListening() {
    if (micStarting || sending) return;
    const operation = ++micOperation.current;
    setMicStarting(true);
    setError(null);
    setReaction(null);
    stopPlayback();
    setState("resting");
    try {
      setLiveTranscript("");
      setVoiceLevel(0);
      voicePushQueue.current = Promise.resolve();
      voicePushFailed.current = false;
      // Live chunked STT: open a Rust-side stream so audio is transcribed while
      // the learner is still speaking. Falls back to the buffered one-shot turn
      // when streaming is unavailable.
      let onChunk: ((samples: number[], sampleRate: number) => void) | undefined;
      if (bridge.beginVoiceStream && bridge.pushVoiceStream && bridge.finishVoiceStreamTurn) {
        try {
          const streamId = await bridge.beginVoiceStream(session.id);
          voiceStreamId.current = streamId;
          if (!mounted.current || operation !== micOperation.current) {
            await cancelVoiceStream();
            return;
          }
          onChunk = (samples, sampleRate) => {
            if (!mounted.current || operation !== micOperation.current) return;
            voicePushQueue.current = voicePushQueue.current.then(async () => {
              if (voicePushFailed.current || !mounted.current || operation !== micOperation.current) return;
              try {
                await bridge.pushVoiceStream?.(streamId, samples, sampleRate);
              } catch (reason) {
                voicePushFailed.current = true;
                llog("stream:push-failed", `${errorMessage(reason)}; falling back to buffered audio`);
              }
            });
          };
        } catch (reason) {
          voiceStreamId.current = null;
          llog("stream:begin-failed", `${errorMessage(reason)}; using buffered voice turn`);
        }
      }
      await voice.current.start(setLiveTranscript, setVoiceLevel, onChunk);
      if (!mounted.current || operation !== micOperation.current) {
        await cancelVoiceStream();
        await voice.current.cancel();
        return;
      }
      captureActive.current = true;
      setState("listening");
      llog("mic:listening", "microphone open, learner is speaking");
    } catch (reason) {
      await cancelVoiceStream();
      captureActive.current = false;
      await voice.current.cancel().catch(() => undefined);
      if (!mounted.current || operation !== micOperation.current) return;
      setTyping(true);
      setState("resting");
      flashReaction("error", 1800);
      setError(`I could not open the microphone (${errorMessage(reason).toLowerCase().replace(/\.$/, "")}). Type your answer instead.`);
    } finally {
      if (mounted.current && operation === micOperation.current) setMicStarting(false);
    }
  }

  async function finishListening() {
    const stoppedAt = markTurnStart("VOICE TURN");
    llog("mic:stop-pressed", "learner stopped speaking; turn clock starts now");
    setSending(true);
    setState("thinking");
    setError(null);
    armSpeechQueue();
    try {
      const capture = await voice.current.stop();
      captureActive.current = false;
      const streamId = voiceStreamId.current;
      if (streamId) await voicePushQueue.current;
      const useVoiceStream = Boolean(streamId && !voicePushFailed.current && bridge.finishVoiceStreamTurn);
      if (streamId && !useVoiceStream) await cancelVoiceStream();
      llog(
        useVoiceStream ? "ipc:finish_voice_stream_turn" : "ipc:send_voice_turn",
        `invoking Rust with ${
          useVoiceStream ? capture.streamTailSamples.length : capture.samples.length
        } ${useVoiceStream ? "tail " : ""}samples @ ${capture.sampleRate} Hz` +
          (capture.transcript ? ` (browser transcript: "${capture.transcript}")` : ""),
      );
      const ipcStarted = performance.now();
      const result =
        useVoiceStream && streamId && bridge.finishVoiceStreamTurn
          ? await bridge.finishVoiceStreamTurn({
              streamId,
              tailSamples: capture.streamTailSamples,
              sampleRate: capture.sampleRate,
              browserTranscript: capture.transcript || liveTranscript,
            })
          : await bridge.sendVoiceTurn({
              sessionId: session.id,
              samples: capture.samples,
              sampleRate: capture.sampleRate,
              browserTranscript: capture.transcript || liveTranscript,
            });
      voiceStreamId.current = null;
      llog(
        "ipc:result",
        `Rust round-trip took ${(performance.now() - ipcStarted).toFixed(1)}ms ` +
          `(Rust reported total ${result.timings?.total_ms ?? "-"}ms; ` +
          `IPC serialization overhead ~${result.timings ? Math.max(0, Math.round(performance.now() - ipcStarted - result.timings.total_ms)) : "-"}ms)`,
      );
      llog("transcript", `"${result.learner_message.content}"`);
      llog("ella-reply", `"${result.ella_message.content}"`);
      logServerTimings(result.timings);
      applyResult(result);
      llog(
        "turn:playback-ready",
        `END-TO-END mic stop -> playback started: ${turnElapsed().toFixed(1)}ms`,
      );
      console.info(
        JSON.stringify({
          event: "ella_voice_playback_ready",
          schema_version: 1,
          interaction_id: result.timings?.interaction_id,
          stop_to_playback_ready_ms: Math.round(performance.now() - stoppedAt),
          server_total_ms: result.timings?.total_ms,
          stt_engine: result.timings?.stt_engine,
          stt_backend: result.timings?.stt_backend,
        }),
      );
      setLiveTranscript("");
    } catch (reason) {
      llog("turn:error", `voice turn failed after ${turnElapsed().toFixed(1)}ms: ${errorMessage(reason)}`);
      stopPlayback();
      await cancelVoiceStream();
      captureActive.current = false;
      await voice.current.cancel().catch(() => undefined);
      setState("resting");
      flashReaction("error", 1800);
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
    setState("thinking");
    setError(null);
    armSpeechQueue();
    try {
      const ipcStarted = performance.now();
      const result = await bridge.sendTextTurn(session.id, input);
      llog(
        "ipc:result",
        `Rust round-trip took ${(performance.now() - ipcStarted).toFixed(1)}ms ` +
          `(Rust reported total ${result.timings?.total_ms ?? "-"}ms)`,
      );
      llog("ella-reply", `"${result.ella_message.content}"`);
      logServerTimings(result.timings);
      setInput("");
      applyResult(result);
      llog(
        "turn:playback-ready",
        `END-TO-END submit -> playback started: ${turnElapsed().toFixed(1)}ms`,
      );
    } catch (reason) {
      llog("turn:error", `text turn failed after ${turnElapsed().toFixed(1)}ms: ${errorMessage(reason)}`);
      stopPlayback();
      setState("resting");
      flashReaction("error", 1800);
      setError(errorMessage(reason));
    } finally {
      setSending(false);
    }
  }

  function applyResult(result: TurnResult) {
    onSessionChange({
      ...session,
      messages: [...session.messages, result.learner_message, result.ella_message],
    });
    setLastTurn(result);
    flashReaction("success");
    const streaming = speechQueue.current;
    // The reply has been playing since before this result arrived. `audio` is
    // the same recording, kept for the replay button — playing it now would say
    // the whole turn a second time. If nothing reached the queue, the stream
    // did not get through and the ordinary one-shot playback still applies.
    if (
      streaming &&
      streaming.generation === playbackGeneration.current &&
      result.streamed_segments > 0 &&
      streaming.queue.received > 0
    ) {
      llog(
        "playback:streamed",
        `${streaming.queue.received}/${result.streamed_segments} sentence(s) already playing`,
      );
      streaming.queue.finish(result.streamed_segments);
      // Defensive fallback for a queue that never reports it finished.
      playbackWatchdog.current = window.setTimeout(
        () => {
          if (mounted.current && playbackGeneration.current === streaming.generation) {
            setState("resting");
          }
        },
        Math.max(15_000, Math.min(45_000, result.ella_message.content.length * 180)),
      );
      return;
    }
    playElla(result.ella_message.content, result);
  }

  async function switchToTyping() {
    micOperation.current += 1;
    setMicStarting(false);
    stopPlayback();
    setVoiceLevel(0);
    setLiveTranscript("");
    setState("resting");
    setTyping(true);
    await cancelVoiceStream();
    if (captureActive.current || state === "listening") {
      captureActive.current = false;
      await voice.current.cancel().catch(() => undefined);
    }
  }

  function switchToMicrophone() {
    focusMicAfterModeSwitch.current = true;
    setTyping(false);
  }

  async function endConversation() {
    setSending(true);
    setError(null);
    micOperation.current += 1;
    setMicStarting(false);
    stopPlayback();
    setState("resting");
    setVoiceLevel(0);
    setLiveTranscript("");
    await cancelVoiceStream();
    captureActive.current = false;
    try {
      await voice.current.cancel().catch(() => undefined);
      const result = await bridge.completeSession(session.id);
      onComplete(result);
    } catch (reason) {
      flashReaction("error", 1800);
      setError(errorMessage(reason));
      setSending(false);
    }
  }

  const prompt = latestElla?.content ?? "";
  // While a reply is streaming, the words come from the audio, because the
  // turn's text has not arrived yet. Afterwards the two are the same sentence,
  // so which one renders is invisible.
  const promptSentences =
    spokenSentences.length > 0 ? spokenSentences : sentenceGroups(prompt);
  // Words are only dimmed while a clip with timings is actually playing. There
  // is a beat of silence before Piper's first word, and treating that as "not
  // playing" would show the reply at full contrast and then dim it the instant
  // she starts, which reads as a flicker.
  const followingWords = spokenSentences.length > 0 && state === "speaking";
  const interactionLocked = sending || micStarting || state === "thinking";
  // Ella now starts talking while the turn is still committing, so for a moment
  // she is speaking and the microphone is not yet available. Promising an
  // interrupt the button will not accept reads as a broken control.
  const micHint = micStarting
    ? "Opening the microphone…"
    : state === "speaking" && sending
      ? "Ella is answering…"
      : MIC_HINT[state];
  const stateStatus = micStarting
    ? "Opening the microphone…"
    : state === "resting"
      ? reaction === "success"
        ? "Nice work"
        : reaction === "error"
          ? "Let’s try that again"
          : "Your turn"
      : state === "listening"
        ? "Listening"
        : state === "thinking"
          ? "Ella is thinking"
          : "Ella is speaking";
  const announcedStatus = `${micHint}${reaction === "success" ? " Nice work." : ""}`;

  return (
    <div className="screen screen--talk" data-screen="talk">
      <header className="talk-head">
        <span className="pill pill--white">{session.topic_label}</span>
        <button
          type="button"
          className="btn btn--quiet"
          onClick={() => void endConversation()}
          disabled={sending || micStarting}
        >
          End talk
        </button>
      </header>

      <div className="talk-stage">
        <div className="talk-copy">
          <div className="talk-state-indicator" aria-hidden="true">
            <span className="talk-state-indicator__visual">
              {micStarting ? (
                <LoaderCircle className="spin" size={18} />
              ) : state === "speaking" ? (
                <SpeakingWave />
              ) : state === "thinking" ? (
                <ThinkingDots />
              ) : state === "listening" ? (
                <span className="talk-listen-dot" />
              ) : reaction === "success" ? (
                <span className="talk-status-sparkle" />
              ) : reaction === "error" ? (
                <span className="talk-status-concern">!</span>
              ) : (
                <span className="talk-ready-dot" />
              )}
            </span>
            <span>{stateStatus}</span>
          </div>

          <p className="talk-prompt">
            {promptSentences.map((words, sentence) => {
              // Where this sentence's words sit in the reply, because the
              // highlight counts words across the whole thing.
              const offset = promptSentences
                .slice(0, sentence)
                .reduce((total, earlier) => total + earlier.length, 0);
              return (
                <span className="talk-sentence" key={`${sentence}-${words[0] ?? ""}`}>
                  {words.map((word, index) => (
                    <Fragment key={`${index}-${word}`}>
                      <span
                        className={wordClass(offset + index, spokenIndex, followingWords)}
                      >
                        {word}
                      </span>
                      {index < words.length - 1 ? " " : ""}
                    </Fragment>
                  ))}
                </span>
              );
            })}
          </p>

          {/* Only Ella's side of the talk is on screen: the learner's own words
              are never echoed back, so a misheard transcript cannot become the
              thing they read. */}
          {lastTurn?.correction && (
            <div className="talk-feedback">
              <div className="talk-feedback__item talk-feedback__item--coach">
                <span className="talk-feedback__label">Try this</span>
                <p>{lastTurn.correction}</p>
              </div>
            </div>
          )}

          {latestElla && (
            <button
              type="button"
              className="btn btn--replay"
              disabled={interactionLocked || state === "listening"}
              onClick={() => {
                setReaction(null);
                playElla(latestElla.content, lastTurn ?? openingLine ?? undefined);
              }}
            >
              <svg viewBox="0 0 24 24" width="15" height="15" aria-hidden="true">
                <path d="M3.51 15a9 9 0 102.13-9.36L1 10" />
                <polyline points="1 4 1 10 7 10" />
              </svg>
              Hear it again
            </button>
          )}

          {error && (
            <p className="inline-error talk-error" role="alert">
              {error}
            </p>
          )}
        </div>

        <p className="sr-only" role="status" aria-live="polite" aria-atomic="true">
          {announcedStatus}
        </p>

      </div>

      <div className="talk-dock">
        <EllaMascot
          variant="conversation"
          className="ella--stage-talk"
          state={state}
          reaction={reaction}
          activity={voiceLevel}
          decorative
        />
        <div className="talk-controls">
          {typing ? (
            <form className="composer" onSubmit={submitText}>
              <label className="sr-only" htmlFor="talk-text-input">
                Your answer
              </label>
              <input
                id="talk-text-input"
                autoFocus
                value={input}
                maxLength={800}
                disabled={sending}
                placeholder="Type what you want to say…"
                onChange={(event) => setInput(event.target.value)}
              />
              <button
                type="submit"
                className="composer__send"
                disabled={!input.trim() || sending}
                aria-label="Send answer"
              >
                {sending ? <LoaderCircle className="spin" size={20} /> : <Send size={20} />}
              </button>
              <button
                type="button"
                className="link-button"
                disabled={sending}
                onClick={switchToMicrophone}
              >
                Use the microphone
              </button>
            </form>
          ) : (
            <div className="mic-stack">
              <div className="mic-wrap">
                {state === "listening" && (
                  <>
                    <span className="mic-pulse" />
                    <span className="mic-pulse mic-pulse--delayed" />
                  </>
                )}
                <button
                  ref={micButton}
                  className={`mic ${state === "listening" ? "is-live" : ""}`}
                  type="button"
                  disabled={interactionLocked}
                  aria-pressed={state === "listening"}
                  aria-describedby="talk-mic-hint"
                  aria-label={
                    micStarting
                      ? "Opening the microphone"
                      : state === "listening"
                        ? "Stop and send"
                        : state === "speaking"
                          ? "Interrupt Ella and start speaking"
                          : "Start speaking"
                  }
                  onClick={() => (state === "listening" ? void finishListening() : void beginListening())}
                >
                  {micStarting ? <LoaderCircle className="spin" size={26} /> : <MicGlyph />}
                </button>
              </div>
              <p className="mic-hint" id="talk-mic-hint">
                {micHint}
              </p>
              {state === "listening" && <VoiceMeter level={voiceLevel} />}
              <button
                type="button"
                className="link-button"
                disabled={sending || state === "thinking"}
                onClick={() => void switchToTyping()}
              >
                Type instead
              </button>
            </div>
          )}
        </div>
      </div>
    </div>
  );
}

/**
 * How one word of the reply is drawn.
 *
 * Three states while Ella is talking: said, saying, and not yet said. The words
 * ahead are dimmed rather than hidden — they hold their place, so the line
 * never re-wraps as she speaks, they can be read ahead of the voice, and a
 * sentence arriving mid-reply fades in faint instead of appearing at full
 * contrast. When she is not talking, or is talking without timings to follow,
 * the whole reply is drawn plainly.
 */
function wordClass(index: number, spoken: number, following: boolean): string {
  if (!following) return "talk-word";
  if (index === spoken) return "talk-word is-spoken";
  return index < spoken ? "talk-word is-said" : "talk-word is-ahead";
}

function errorMessage(reason: unknown): string {
  if (typeof reason === "string") return reason;
  if (reason instanceof Error) return reason.message;
  return "Something unexpected happened. Please try again.";
}
