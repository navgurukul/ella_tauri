/**
 * The telemetry relay's routes:
 *
 *   GET    /healthz                       `{"ok":true,"storage":<whether a key is set>}`.
 *   POST   /v1/installs                   A new telemetry token, `{"token":"<64 hex>"}`,
 *                                         once per laptop. Body optional:
 *                                         `{"app_version":"0.1.28"}`.
 *   POST   /v1/batches/<batch_id>         A batch file, `application/x-ndjson`, with
 *                                         `Authorization: Bearer <telemetry token>`:
 *                                         201 `{"stored":true}`, or 200
 *                                         `{"duplicate":true}` when it is stored already.
 *   GET    /v1/pull/incoming              The batches waiting, `{"objects":[{"batch_id",
 *                                         "size","created_at"}],"listed":n}`, a page
 *                                         of `?limit=&offset=`: the next page is at
 *                                         offset + `listed`, and there is none once
 *                                         `listed` is under the limit.
 *   GET    /v1/pull/incoming/<batch_id>   One batch's bytes.
 *   DELETE /v1/pull/incoming/<batch_id>   Deletes it once it is in the telemetry
 *                                         repository; 204, gone already or not.
 *
 * The pull routes need `Authorization: Bearer <ELLA_TELEMETRY_PULL_TOKEN>`,
 * and reach `telemetry/incoming/` only. Deployed, Supabase hands the
 * function its whole path, `/ella-telemetry/v1/...`; served locally, it is
 * `/v1/...`. Both are routed the same.
 *
 * A laptop puts a batch aside for good only on 400 or 413 with `bad_batch`
 * (see `errors.ts`): everything else it keeps and sends again later.
 *
 * Nothing of a batch is logged, nor the caller's address: a request's log
 * line has its outcome, its size, and its telemetry install's row, never
 * the batch's `install_id`.
 */

import { check, MAX_BATCH_BYTES, type Refusal } from "./batch.ts";
import type { Limits } from "./config.ts";
import { ApiError } from "./errors.ts";
import { bearer, mintToken, sameSecret, sha256 } from "./hash.ts";
import { type Storage, StorageError } from "./storage.ts";
import type { Store } from "./store.ts";

const BATCH_ID = /^[0-9a-f]{32}$/;
/** An install's body, `{"app_version":...}`, is never more. */
const MAX_INSTALL_BODY = 4_096;
const PAGE_MAX = 1_000;

export interface Deps {
  store: Store;
  storage: Storage;
  limits: Limits;
  policyVersions: readonly number[];
  /** Whether a batch may be refused; else the laptop is told to try later. */
  refuse: boolean;
  /** The pull token, or "" when none is set. */
  pullToken: string;
  /** Keeps a task running after the answer has gone: see `index.ts`. */
  background: (task: Promise<void>) => void;
}

export function createHandler(deps: Deps): (request: Request) => Promise<Response> {
  return async (request) => {
    const requestId = crypto.randomUUID();
    try {
      const segments = routePath(new URL(request.url).pathname);
      const method = request.method;
      const route = segments.join("/");
      if (route === "healthz" && (method === "GET" || method === "HEAD")) {
        return json({ ok: true, storage: deps.storage.configured }, 200, requestId);
      }
      if (route === "v1/installs" && method === "POST") {
        return await install(request, deps, requestId);
      }
      if (segments.length === 3 && segments[0] === "v1" && segments[1] === "batches") {
        if (method === "POST" && BATCH_ID.test(segments[2])) {
          return await batch(request, segments[2], deps, requestId);
        }
      }
      if (segments[0] === "v1" && segments[1] === "pull" && segments[2] === "incoming") {
        return await pull(request, segments.slice(3), deps, requestId);
      }
      throw new ApiError("not_found", "no such route");
    } catch (error) {
      if (error instanceof ApiError) return error.response(requestId);
      console.error("internal failure", { id: requestId, detail: String(error).slice(0, 200) });
      return new ApiError("internal", "something went wrong").response(requestId);
    }
  };
}

/** The route's segments, without the function's own name in front when deployed. */
export function routePath(pathname: string): string[] {
  const segments = pathname.split("/").filter((segment) => segment !== "");
  if (segments[0] === "ella-telemetry") segments.shift();
  return segments;
}

async function install(request: Request, deps: Deps, requestId: string): Promise<Response> {
  // A school lab shares one address, so this only slows a script down.
  const ipKey = (await sha256(`ella-telemetry:${callerAddress(request)}`)).slice(0, 32);
  if (!(await deps.store.admitInstall(ipKey, deps.limits.installsPerHour))) {
    throw new ApiError("limit", "too many new installs from this address; try again later", {
      limit: "installs",
    });
  }
  const bytes = await readBody(request, MAX_INSTALL_BODY);
  if (bytes === null) throw new ApiError("bad_request", "the body is too large");
  let appVersion: string | null = null;
  const text = new TextDecoder().decode(bytes).trim();
  if (text !== "") {
    let body: unknown;
    try {
      body = JSON.parse(text);
    } catch {
      throw new ApiError("bad_request", "the body is not JSON");
    }
    const version = (body as { app_version?: unknown } | null)?.app_version;
    appVersion = typeof version === "string" ? version.slice(0, 32) : null;
  }
  const token = mintToken();
  const made = await deps.store.createInstall(await sha256(token), appVersion);
  console.info("install", { id: requestId, install: made.id.slice(0, 8), app_version: appVersion });
  return json({ token }, 201, requestId);
}

async function batch(
  request: Request,
  batchId: string,
  deps: Deps,
  requestId: string,
): Promise<Response> {
  const started = performance.now();
  const token = bearer(request);
  const found = token === null ? null : await deps.store.installByTokenHash(await sha256(token));
  if (found === null || found.revoked) {
    throw new ApiError("unauthorized", "a telemetry token from POST /v1/installs is required");
  }
  const log = (fields: Record<string, unknown>) =>
    console.info("batch", {
      id: requestId,
      install: found.id.slice(0, 8),
      ms: Math.round(performance.now() - started),
      ...fields,
    });

  const type = (request.headers.get("content-type") ?? "").split(";")[0].trim().toLowerCase();
  if (type !== "application/x-ndjson") {
    throw new ApiError("unsupported_type", "a batch is sent as application/x-ndjson");
  }
  if (!deps.storage.configured) {
    throw new ApiError("not_configured", "the relay has no storage key");
  }

  const refuse = (reason: Refusal, bytes: number): never => {
    log({ outcome: "refused", reason, bytes });
    deps.background(
      deps.store.recordRefusal(reason).catch((error) =>
        console.error("refusal not counted", { id: requestId, detail: String(error) })
      ),
    );
    if (!deps.refuse) {
      throw new ApiError("refusals_off", "the relay is not refusing batches now; try again later");
    }
    throw new ApiError(
      "bad_batch",
      reason === "too_large"
        ? `a batch is at most ${MAX_BATCH_BYTES} bytes`
        : "the batch is not whole",
      { reason },
      reason === "too_large" ? 413 : 400,
    );
  };

  const declared = Number(request.headers.get("content-length") ?? "0");
  if (declared > MAX_BATCH_BYTES) {
    await request.body?.cancel();
    return refuse("too_large", declared);
  }
  const data = await readBody(request, MAX_BATCH_BYTES);
  if (data === null) return refuse("too_large", MAX_BATCH_BYTES + 1);

  const admission = await deps.store.admit(found.id, data.length, deps.limits);
  if (admission === "all") {
    log({ outcome: "paused", bytes: data.length });
    throw new ApiError("paused", "today's telemetry has reached its limit; try again tomorrow");
  }
  if (admission === "day") {
    log({ outcome: "limit", bytes: data.length });
    throw new ApiError("limit", "over this install's batches per day", { limit: "day" });
  }

  const verdict = await check(data, batchId, deps.policyVersions);
  if (!verdict.ok) {
    if ("refused" in verdict) return refuse(verdict.refused, data.length);
    log({ outcome: "policy_not_accepted", policy_version: verdict.policyVersion });
    throw new ApiError(
      "policy_not_accepted",
      `policy_version ${verdict.policyVersion} is not taken yet; keep the batch`,
      { policy_version: verdict.policyVersion },
    );
  }

  let stored: "stored" | "exists";
  try {
    stored = await deps.storage.put(batchId, data);
  } catch (error) {
    if (!(error instanceof StorageError)) throw error;
    log({ outcome: "storage", bytes: data.length, detail: error.message });
    throw new ApiError("storage", "the batch could not be stored; try again later");
  }
  log({ outcome: stored === "stored" ? "stored" : "duplicate", bytes: data.length });
  return stored === "stored"
    ? json({ stored: true }, 201, requestId)
    : json({ duplicate: true }, 200, requestId);
}

async function pull(
  request: Request,
  rest: string[],
  deps: Deps,
  requestId: string,
): Promise<Response> {
  if (deps.pullToken === "") throw new ApiError("not_configured", "the relay has no pull token");
  const token = bearer(request);
  if (token === null || !(await sameSecret(token, deps.pullToken))) {
    throw new ApiError("unauthorized", "the pull token is required");
  }
  if (!deps.storage.configured) {
    throw new ApiError("not_configured", "the relay has no storage key");
  }
  const method = request.method;
  try {
    if (rest.length === 0 && method === "GET") {
      const params = new URL(request.url).searchParams;
      const limit = Math.min(
        Math.max(Number(params.get("limit") ?? PAGE_MAX) || PAGE_MAX, 1),
        PAGE_MAX,
      );
      const offset = Math.max(Number(params.get("offset") ?? 0) || 0, 0);
      const { objects, listed } = await deps.storage.list(limit, offset);
      console.info("pull", { id: requestId, listed });
      return json(
        {
          objects: objects.map((o) => ({
            batch_id: o.batchId,
            size: o.size,
            created_at: o.createdAt,
          })),
          listed,
        },
        200,
        requestId,
      );
    }
    if (rest.length === 1 && BATCH_ID.test(rest[0])) {
      if (method === "GET") {
        const data = await deps.storage.get(rest[0]);
        if (data === null) throw new ApiError("not_found", "no such batch waiting");
        return new Response(data, {
          status: 200,
          headers: { "content-type": "application/x-ndjson", "x-request-id": requestId },
        });
      }
      if (method === "DELETE") {
        await deps.storage.remove(rest[0]);
        console.info("pull", { id: requestId, deleted: 1 });
        return new Response(null, { status: 204, headers: { "x-request-id": requestId } });
      }
    }
  } catch (error) {
    if (!(error instanceof StorageError)) throw error;
    console.error("pull failed", { id: requestId, detail: error.message });
    throw new ApiError("storage", "Storage did not answer; try again later");
  }
  throw new ApiError("not_found", "no such route");
}

/** The body, or `null` once it is more than `max` bytes. */
async function readBody(request: Request, max: number): Promise<Uint8Array<ArrayBuffer> | null> {
  if (request.body === null) return new Uint8Array(0);
  const reader = request.body.getReader();
  const chunks: Uint8Array<ArrayBuffer>[] = [];
  let length = 0;
  while (true) {
    const { done, value } = await reader.read();
    if (done) break;
    length += value.length;
    if (length > max) {
      await reader.cancel();
      return null;
    }
    chunks.push(value);
  }
  const data = new Uint8Array(length);
  let at = 0;
  for (const chunk of chunks) {
    data.set(chunk, at);
    at += chunk.length;
  }
  return data;
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
