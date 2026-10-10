/**
 * The proxy's three routes:
 *
 *   GET  /healthz      `{"ok":true,"chat":<whether a key is set>}`: what a
 *                      laptop asks to tell whether it is online.
 *   POST /v1/installs  A new install token, `{"token":"<64 hex>"}`, once per
 *                      laptop. Body optional: `{"app_version":"0.1.17"}`.
 *   POST /v1/chat      An OpenAI chat-completions request, with
 *                      `Authorization: Bearer <token>`, forwarded to DeepSeek
 *                      as `chat.ts` allows, its answer or stream passed back
 *                      unchanged.
 *
 * Deployed, Supabase hands the function its whole path, `/ella-desktop/v1/chat`;
 * served locally, it is `/v1/chat`. Both are routed the same.
 *
 * Nothing a learner says is logged or kept: a request's log line has its
 * install, its timings and its token counts.
 */

import {
  errorMessage,
  estimatedUsage,
  MAX_BODY_BYTES,
  readChatRequest,
  relay,
  upstreamBody,
  upstreamError,
  usageFrom,
} from "./chat.ts";
import { type ChatConfig, endpoint, type Limits } from "./config.ts";
import { ApiError } from "./errors.ts";
import type { Store, Usage } from "./store.ts";
import { bearer, mintToken, sha256 } from "./tokens.ts";

/**
 * How long a request to DeepSeek may take, stream included. A laptop gives
 * up far sooner and hangs up, which cancels it; this only bounds the rest.
 */
const UPSTREAM_TIMEOUT_MS = 100_000;

export interface Deps {
  store: Store;
  chat: ChatConfig;
  limits: Limits;
  /** Keeps a task running after the answer has gone: see `index.ts`. */
  background: (task: Promise<void>) => void;
  /** DeepSeek's fetch; a fake in tests. */
  fetch?: typeof fetch;
}

export function createHandler(deps: Deps): (request: Request) => Promise<Response> {
  return async (request) => {
    const requestId = crypto.randomUUID();
    try {
      const path = routePath(new URL(request.url).pathname);
      const method = request.method;
      if (path === "healthz" && (method === "GET" || method === "HEAD")) {
        return json({ ok: true, chat: deps.chat.apiKey !== "" }, 200, requestId);
      }
      if (path === "v1/installs" && method === "POST") {
        return await install(request, deps, requestId);
      }
      if (path === "v1/chat" && method === "POST") return await chat(request, deps, requestId);
      throw new ApiError("not_found", "no such route");
    } catch (error) {
      if (error instanceof ApiError) return error.response(requestId);
      console.error("internal failure", { id: requestId, detail: String(error) });
      return new ApiError("internal", "something went wrong").response(requestId);
    }
  };
}

/** The route, without the function's own name in front when deployed. */
export function routePath(pathname: string): string {
  const segments = pathname.split("/").filter((segment) => segment !== "");
  if (segments[0] === "ella-desktop") segments.shift();
  return segments.join("/");
}

async function install(request: Request, deps: Deps, requestId: string): Promise<Response> {
  // A school lab shares one address, so this only slows a script down.
  const ipKey = (await sha256(`ella-desktop:${callerAddress(request)}`)).slice(0, 32);
  if (!(await deps.store.admitInstall(ipKey, deps.limits.installsPerHour))) {
    throw new ApiError("limit", "too many new installs from this address; try again later", {
      limit: "installs",
    });
  }
  const body = await readBody(request, true);
  const version = (body as { app_version?: unknown } | null)?.app_version;
  const appVersion = typeof version === "string" ? version.slice(0, 32) : null;
  const token = mintToken();
  const made = await deps.store.createInstall(await sha256(token), appVersion);
  console.info("install", { id: requestId, install: made.id.slice(0, 8), app_version: appVersion });
  return json({ token }, 201, requestId);
}

async function chat(request: Request, deps: Deps, requestId: string): Promise<Response> {
  const token = bearer(request);
  const found = token === null ? null : await deps.store.installByTokenHash(await sha256(token));
  if (found === null || found.revoked) {
    throw new ApiError("unauthorized", "an install token from POST /v1/installs is required");
  }
  if (deps.chat.apiKey === "") {
    throw new ApiError("not_configured", "the proxy has no DeepSeek key");
  }
  const asked = readChatRequest(await readBody(request, false));

  const admission = await deps.store.admit(found.id, deps.limits);
  if (admission === "cap") throw new ApiError("cap", "today's spend has reached the cap");
  if (admission !== "ok") {
    throw new ApiError("limit", `over this install's requests per ${admission}`, {
      limit: admission,
    });
  }

  const started = performance.now();
  const since = () => Math.round(performance.now() - started);
  const prices = deps.chat.prices;
  const record = (usage: Usage) =>
    deps.background(
      deps.store.recordUsage(found.id, usage).catch((error) =>
        console.error("usage not recorded", { id: requestId, detail: String(error) })
      ),
    );
  const log = (fields: Record<string, unknown>) =>
    console.info("chat", {
      id: requestId,
      install: found.id.slice(0, 8),
      stream: asked.stream,
      json: asked.json,
      ...fields,
    });

  let upstream: Response;
  try {
    upstream = await (deps.fetch ?? fetch)(endpoint(deps.chat.baseUrl, "chat/completions"), {
      method: "POST",
      headers: {
        "authorization": `Bearer ${deps.chat.apiKey}`,
        "content-type": "application/json",
        "accept": asked.stream ? "text/event-stream" : "application/json",
      },
      body: JSON.stringify(upstreamBody(asked, deps.chat, found.id)),
      signal: AbortSignal.any([request.signal, AbortSignal.timeout(UPSTREAM_TIMEOUT_MS)]),
    });
  } catch (error) {
    const detail = String(error).replaceAll(deps.chat.apiKey, "[key]").slice(0, 200);
    log({ outcome: "unreachable", ms: since(), detail });
    throw new ApiError("upstream", "DeepSeek could not be reached");
  }
  const headersMs = since();

  if (!upstream.ok) {
    const error = upstreamError(
      upstream.status,
      errorMessage(await upstream.text().catch(() => "")),
    );
    log({ outcome: error.code, upstream_status: upstream.status, ms: headersMs });
    throw error;
  }

  if (asked.stream) {
    if (upstream.body === null) throw new ApiError("upstream", "DeepSeek sent no stream");
    const body = relay(upstream.body, (watcher, finished) => {
      const usage = usageFrom(watcher.usage, prices) ??
        estimatedUsage(asked.promptChars, watcher.writtenChars, prices);
      record(usage);
      log({
        outcome: finished ? "ok" : "cut_short",
        headers_ms: headersMs,
        ms: since(),
        hit: usage.hit,
        miss: usage.miss,
        completion: usage.completion,
      });
    });
    return new Response(body, {
      status: 200,
      headers: {
        "content-type": "text/event-stream",
        "cache-control": "no-cache",
        "x-request-id": requestId,
      },
    });
  }

  // While it waits under load DeepSeek sends blank lines first: JSON allows them.
  const text = await upstream.text();
  let answer: Record<string, unknown>;
  try {
    answer = JSON.parse(text);
  } catch {
    record(estimatedUsage(asked.promptChars, 0, prices));
    log({ outcome: "unreadable", ms: since() });
    throw new ApiError("upstream", "DeepSeek's answer could not be read");
  }
  const usage = usageFrom(answer.usage, prices) ?? estimatedUsage(asked.promptChars, 0, prices);
  record(usage);
  log({
    outcome: "ok",
    headers_ms: headersMs,
    ms: since(),
    hit: usage.hit,
    miss: usage.miss,
    completion: usage.completion,
  });
  return json(answer, 200, requestId);
}

/** The JSON body, at most `MAX_BODY_BYTES`; `null` for an empty one when allowed. */
async function readBody(request: Request, emptyAllowed: boolean): Promise<unknown> {
  const declared = Number(request.headers.get("content-length") ?? "0");
  if (declared > MAX_BODY_BYTES) {
    throw new ApiError("bad_request", `the body is larger than ${MAX_BODY_BYTES} bytes`);
  }
  const bytes = await request.arrayBuffer();
  if (bytes.byteLength > MAX_BODY_BYTES) {
    throw new ApiError("bad_request", `the body is larger than ${MAX_BODY_BYTES} bytes`);
  }
  const text = new TextDecoder().decode(bytes).trim();
  if (text === "" && emptyAllowed) return null;
  try {
    return JSON.parse(text);
  } catch {
    throw new ApiError("bad_request", "the body is not JSON");
  }
}

/** The caller's address as the platform saw it; only ever hashed. */
function callerAddress(request: Request): string {
  const forwarded = request.headers.get("x-forwarded-for")?.split(",")[0]?.trim();
  return forwarded || request.headers.get("cf-connecting-ip") || "unknown";
}

function json(body: unknown, status: number, requestId: string): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { "content-type": "application/json", "x-request-id": requestId },
  });
}
