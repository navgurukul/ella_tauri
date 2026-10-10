/**
 * What a laptop may ask the model, and what the proxy sends on.
 *
 * The laptop speaks the OpenAI chat-completions shape it already speaks to
 * its local llama-server. Only the fields Ella uses survive, inside bounds;
 * everything else is dropped rather than forwarded, so the key cannot be
 * spent on anything but short, Ella-sized requests. The model is the
 * function's, with thinking off: Ella's replies are spoken, and a model that
 * reasons first leaves the learner waiting.
 */

import type { ChatConfig, Prices } from "./config.ts";
import { ApiError } from "./errors.ts";
import type { Usage } from "./store.ts";

/** A request body larger than this is refused before it is read as JSON. */
export const MAX_BODY_BYTES = 96 * 1024;
const MAX_MESSAGES = 120;
const MAX_CONTENT_CHARS = 24_000;
const MAX_TOTAL_CHARS = 80_000;
/** The longest answer a laptop asks for: a correction of a long talk. */
export const MAX_TOKENS = 1024;
const MAX_STOPS = 4;
const MAX_STOP_CHARS = 32;

const ROLES = new Set(["system", "user", "assistant"]);

/** The parts of a laptop's request the proxy forwards, checked. */
export interface ChatRequest {
  messages: Array<{ role: string; content: string }>;
  maxTokens: number;
  temperature: number | null;
  stream: boolean;
  json: boolean;
  stop: string[];
  /** How many characters the prompt holds, for estimating an unfinished request's cost. */
  promptChars: number;
}

const bad = (message: string) => new ApiError("bad_request", message);

/** Reads and checks a laptop's request body; throws `bad_request` with the reason. */
export function readChatRequest(body: unknown): ChatRequest {
  if (typeof body !== "object" || body === null || Array.isArray(body)) {
    throw bad("the body must be a JSON object");
  }
  const input = body as Record<string, unknown>;

  const raw = input.messages;
  if (!Array.isArray(raw) || raw.length === 0 || raw.length > MAX_MESSAGES) {
    throw bad(`messages must be an array of 1 to ${MAX_MESSAGES} messages`);
  }
  let promptChars = 0;
  const messages = raw.map((message, index) => {
    if (typeof message !== "object" || message === null) {
      throw bad(`messages[${index}] must be an object`);
    }
    const { role, content } = message as Record<string, unknown>;
    if (typeof role !== "string" || !ROLES.has(role)) {
      throw bad(`messages[${index}].role must be system, user or assistant`);
    }
    if (typeof content !== "string" || content.length > MAX_CONTENT_CHARS) {
      throw bad(
        `messages[${index}].content must be a string of at most ${MAX_CONTENT_CHARS} characters`,
      );
    }
    promptChars += content.length;
    return { role, content };
  });
  if (promptChars > MAX_TOTAL_CHARS) {
    throw bad(`the messages hold more than ${MAX_TOTAL_CHARS} characters`);
  }

  const maxTokens = input.max_tokens ?? 256;
  if (
    !Number.isInteger(maxTokens) || (maxTokens as number) < 1 || (maxTokens as number) > MAX_TOKENS
  ) {
    throw bad(`max_tokens must be a whole number from 1 to ${MAX_TOKENS}`);
  }

  const temperature = input.temperature ?? null;
  if (
    temperature !== null &&
    (typeof temperature !== "number" || temperature < 0 || temperature > 1.5)
  ) {
    throw bad("temperature must be a number from 0 to 1.5");
  }

  const stream = input.stream ?? false;
  if (typeof stream !== "boolean") throw bad("stream must be true or false");

  let json = false;
  const format = input.response_format;
  if (format !== undefined && format !== null) {
    const type = (format as Record<string, unknown>).type;
    if (type === "json_object") json = true;
    else if (type !== "text") throw bad("response_format.type must be json_object or text");
  }

  const stopInput = input.stop ?? [];
  const stop = typeof stopInput === "string" ? [stopInput] : stopInput;
  if (
    !Array.isArray(stop) || stop.length > MAX_STOPS ||
    stop.some((s) => typeof s !== "string" || s.length === 0 || s.length > MAX_STOP_CHARS)
  ) {
    throw bad(`stop must be up to ${MAX_STOPS} strings of at most ${MAX_STOP_CHARS} characters`);
  }

  return {
    messages,
    maxTokens: maxTokens as number,
    temperature: temperature as number | null,
    stream,
    json,
    stop: stop as string[],
    promptChars,
  };
}

/** The body DeepSeek receives for `request`, on behalf of install `userId`. */
export function upstreamBody(
  request: ChatRequest,
  chat: ChatConfig,
  userId: string,
): Record<string, unknown> {
  return {
    model: chat.model,
    messages: request.messages,
    max_tokens: request.maxTokens,
    thinking: { type: "disabled" },
    ...(request.temperature === null ? {} : { temperature: request.temperature }),
    ...(request.json ? { response_format: { type: "json_object" } } : {}),
    ...(request.stop.length > 0 ? { stop: request.stop } : {}),
    stream: request.stream,
    // Usage on the last chunk, for the books; refused unless streaming.
    ...(request.stream ? { stream_options: { include_usage: true } } : {}),
    // DeepSeek keeps each user's concurrency apart. Not personal: an install id.
    user_id: userId,
  };
}

/** Tokens out of DeepSeek's `usage`, priced; `null` when it holds none. */
export function usageFrom(raw: unknown, prices: Prices): Usage | null {
  if (typeof raw !== "object" || raw === null) return null;
  const usage = raw as Record<string, unknown>;
  const count = (value: unknown) => (typeof value === "number" && value >= 0 ? value : 0);
  const completion = count(usage.completion_tokens);
  const hit = count(usage.prompt_cache_hit_tokens);
  let miss = count(usage.prompt_cache_miss_tokens);
  if (hit + miss === 0) miss = count(usage.prompt_tokens);
  if (hit + miss + completion === 0) return null;
  return { hit, miss, completion, costMicroUsd: cost(hit, miss, completion, prices) };
}

/**
 * What a request that ended before DeepSeek said its usage probably cost: the
 * whole prompt as unseen, at about three characters a token, and what was
 * written so far. Only the daily cap reads it.
 */
export function estimatedUsage(promptChars: number, writtenChars: number, prices: Prices): Usage {
  const miss = Math.ceil(promptChars / 3);
  const completion = Math.ceil(writtenChars / 3);
  return { hit: 0, miss, completion, costMicroUsd: cost(0, miss, completion, prices) };
}

function cost(hit: number, miss: number, completion: number, prices: Prices): number {
  // Dollars per million tokens is millionths of a dollar per token.
  return hit * prices.hit + miss * prices.miss + completion * prices.out;
}

/**
 * Reads usage, and how much answer text went by, out of an SSE stream as its
 * bytes pass through unchanged. DeepSeek puts usage on its last content chunk.
 * Lines that are not `data:` (its `: keep-alive` comments) are skipped.
 */
export class SseWatcher {
  usage: unknown = null;
  writtenChars = 0;
  readonly #decoder = new TextDecoder();
  #pending = "";

  push(bytes: Uint8Array): void {
    this.#pending += this.#decoder.decode(bytes, { stream: true });
    let newline: number;
    while ((newline = this.#pending.indexOf("\n")) >= 0) {
      this.#line(this.#pending.slice(0, newline));
      this.#pending = this.#pending.slice(newline + 1);
    }
  }

  #line(line: string): void {
    const trimmed = line.trim();
    if (!trimmed.startsWith("data:")) return;
    const data = trimmed.slice("data:".length).trim();
    if (data === "" || data === "[DONE]") return;
    let chunk: Record<string, unknown>;
    try {
      chunk = JSON.parse(data);
    } catch {
      return;
    }
    if (chunk.usage !== undefined && chunk.usage !== null) this.usage = chunk.usage;
    const choices = chunk.choices;
    if (Array.isArray(choices)) {
      const delta = (choices[0] as { delta?: { content?: unknown } } | undefined)?.delta;
      if (typeof delta?.content === "string") this.writtenChars += delta.content.length;
    }
  }
}

/**
 * `upstream`'s bytes, passed on unchanged, with `done` told once how it ended:
 * with the watcher that read it, and whether it ran to its end. A laptop that
 * hangs up cancels the stream, which cancels the request to DeepSeek.
 */
export function relay(
  upstream: ReadableStream<Uint8Array>,
  done: (watcher: SseWatcher, finished: boolean) => void,
): ReadableStream<Uint8Array> {
  const reader = upstream.getReader();
  const watcher = new SseWatcher();
  let told = false;
  const tell = (finished: boolean) => {
    if (told) return;
    told = true;
    done(watcher, finished);
  };
  return new ReadableStream<Uint8Array>({
    async pull(controller) {
      try {
        const { value, done: ended } = await reader.read();
        if (ended) {
          tell(true);
          controller.close();
          return;
        }
        watcher.push(value);
        controller.enqueue(value);
      } catch (error) {
        tell(false);
        controller.error(error);
      }
    },
    async cancel(reason) {
      tell(false);
      await reader.cancel(reason).catch(() => {});
    },
  });
}

/** What DeepSeek's error status means for the laptop, with its own message. */
export function upstreamError(status: number, message: string): ApiError {
  if (status === 401 || status === 402 || status === 403) {
    return new ApiError("upstream_config", `DeepSeek refused the request (${status}): ${message}`);
  }
  if (status === 429) return new ApiError("upstream_busy", "DeepSeek is too busy right now");
  if (status === 400 || status === 422) {
    return new ApiError("upstream_rejected", `DeepSeek rejected the request: ${message}`);
  }
  return new ApiError("upstream", `DeepSeek failed (${status})`);
}

/** The message in DeepSeek's error body, or the start of whatever it sent. */
export function errorMessage(text: string): string {
  try {
    const body = JSON.parse(text);
    const message = body?.error?.message;
    if (typeof message === "string") return message.slice(0, 300);
  } catch {
    // Not JSON: fall through.
  }
  return text.trim().slice(0, 300);
}
