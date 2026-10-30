/**
 * The private `telemetry` bucket, through Supabase Storage's API with the
 * project's secret key. Every path is `incoming/<batch_id>.jsonl`, built
 * here from a checked id: nothing else in this bucket, or any other, is
 * reachable through the relay.
 */

import type { StorageConfig } from "./config.ts";

export const BUCKET = "telemetry";
export const PREFIX = "incoming";

export interface Stored {
  batchId: string;
  size: number;
  createdAt: string | null;
}

export interface Listing {
  objects: Stored[];
  listed: number;
}

export interface Storage {
  /** Whether the bucket is reachable at all: a URL and a key are set. */
  readonly configured: boolean;
  /** Writes a batch once: "exists" when it is there already, never overwritten. */
  put(batchId: string, data: Uint8Array<ArrayBuffer>): Promise<"stored" | "exists">;
  /**
   * The batches waiting, a page at a time, by name; `listed` is how many
   * objects Storage gave for the page, the batches' and any others', to page on by.
   */
  list(limit: number, offset: number): Promise<Listing>;
  /** A batch's bytes, or `null` when it is not there. */
  get(batchId: string): Promise<Uint8Array<ArrayBuffer> | null>;
  /** Deletes a batch; one already gone is no fault. */
  remove(batchId: string): Promise<void>;
}

/** Storage refused or failed: the batch is kept on its laptop for later. */
export class StorageError extends Error {}

const BATCH_ID = /^[0-9a-f]{32}$/;
const OBJECT = /^([0-9a-f]{32})\.jsonl$/;

function path(batchId: string): string {
  if (!BATCH_ID.test(batchId)) throw new StorageError("not a batch id");
  return `${PREFIX}/${batchId}.jsonl`;
}

/**
 * Storage's answer to a write over an object that is there already: 409, or
 * an older server's 400 with `statusCode` "409" in its body.
 */
function alreadyThere(status: number, body: unknown): boolean {
  if (status === 409) return true;
  const fields = body as { statusCode?: unknown; error?: unknown } | null;
  return status === 400 &&
    (String(fields?.statusCode) === "409" || fields?.error === "Duplicate");
}

function notThere(status: number, body: unknown): boolean {
  if (status === 404) return true;
  const fields = body as { statusCode?: unknown } | null;
  return status === 400 && String(fields?.statusCode) === "404";
}

export class SupabaseStorage implements Storage {
  readonly #config: StorageConfig;
  readonly #fetch: typeof fetch;

  constructor(config: StorageConfig, fetcher: typeof fetch = fetch) {
    this.#config = config;
    this.#fetch = fetcher;
  }

  get configured(): boolean {
    return this.#config.url !== "" && this.#config.key !== "";
  }

  async #call(
    method: string,
    route: string,
    body?: BodyInit,
    headers: Record<string, string> = {},
  ): Promise<Response> {
    try {
      return await this.#fetch(`${this.#config.url}/storage/v1/${route}`, {
        method,
        headers: {
          // As Supabase's own client sends a key: in both headers.
          "apikey": this.#config.key,
          "authorization": `Bearer ${this.#config.key}`,
          ...headers,
        },
        body,
        redirect: "error",
        signal: AbortSignal.timeout(20_000),
      });
    } catch (error) {
      const detail = String(error).replaceAll(this.#config.key, "[key]").slice(0, 200);
      throw new StorageError(`Storage could not be reached: ${detail}`);
    }
  }

  async put(batchId: string, data: Uint8Array<ArrayBuffer>): Promise<"stored" | "exists"> {
    const response = await this.#call("POST", `object/${BUCKET}/${path(batchId)}`, data, {
      "content-type": "application/x-ndjson",
      "x-upsert": "false",
    });
    const answer = await response.json().catch(() => null);
    if (response.ok) return "stored";
    if (alreadyThere(response.status, answer)) return "exists";
    throw new StorageError(`Storage refused the write: HTTP ${response.status}`);
  }

  async list(limit: number, offset: number): Promise<Listing> {
    const response = await this.#call(
      "POST",
      `object/list/${BUCKET}`,
      JSON.stringify({ prefix: PREFIX, limit, offset, sortBy: { column: "name", order: "asc" } }),
      { "content-type": "application/json" },
    );
    if (!response.ok) {
      await response.body?.cancel();
      throw new StorageError(`Storage refused the listing: HTTP ${response.status}`);
    }
    const items = await response.json();
    if (!Array.isArray(items)) throw new StorageError("Storage's listing was not a list");
    const objects = items.flatMap((item) => {
      const name = OBJECT.exec(String(item?.name ?? ""));
      if (name === null) return [];
      const size = Number(item?.metadata?.size);
      return [{
        batchId: name[1],
        size: Number.isFinite(size) ? size : 0,
        createdAt: typeof item?.created_at === "string" ? item.created_at : null,
      }];
    });
    return { objects, listed: items.length };
  }

  async get(batchId: string): Promise<Uint8Array<ArrayBuffer> | null> {
    const response = await this.#call("GET", `object/authenticated/${BUCKET}/${path(batchId)}`);
    if (response.ok) return new Uint8Array(await response.arrayBuffer());
    const answer = await response.json().catch(() => null);
    if (notThere(response.status, answer)) return null;
    throw new StorageError(`Storage refused the read: HTTP ${response.status}`);
  }

  async remove(batchId: string): Promise<void> {
    const response = await this.#call(
      "DELETE",
      `object/${BUCKET}`,
      JSON.stringify({ prefixes: [path(batchId)] }),
      { "content-type": "application/json" },
    );
    await response.body?.cancel();
    if (!response.ok) throw new StorageError(`Storage refused the delete: HTTP ${response.status}`);
  }
}

/** The same bucket in memory, for tests: `fail` makes every call fail. */
export class MemoryStorage implements Storage {
  readonly objects = new Map<string, Uint8Array<ArrayBuffer>>();
  configured = true;
  fail = false;

  #check(): void {
    if (this.fail) throw new StorageError("Storage is down");
  }

  put(batchId: string, data: Uint8Array<ArrayBuffer>): Promise<"stored" | "exists"> {
    this.#check();
    const key = path(batchId);
    if (this.objects.has(key)) return Promise.resolve("exists");
    this.objects.set(key, data.slice());
    return Promise.resolve("stored");
  }

  list(limit: number, offset: number): Promise<Listing> {
    this.#check();
    const names = [...this.objects.keys()].sort().slice(offset, offset + limit);
    return Promise.resolve({
      objects: names.map((key) => ({
        batchId: key.slice(PREFIX.length + 1, -".jsonl".length),
        size: this.objects.get(key)!.length,
        createdAt: null,
      })),
      listed: names.length,
    });
  }

  get(batchId: string): Promise<Uint8Array<ArrayBuffer> | null> {
    this.#check();
    return Promise.resolve(this.objects.get(path(batchId)) ?? null);
  }

  remove(batchId: string): Promise<void> {
    this.#check();
    this.objects.delete(path(batchId));
    return Promise.resolve();
  }
}
