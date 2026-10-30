/**
 * Ella Desktop's telemetry relay, as one Supabase Edge Function of its own,
 * apart from the chat proxy (`ella-desktop`) so neither can take the other
 * down.
 *
 * A laptop gets a telemetry token once and sends its batches here; each is
 * checked to be whole and stored once in the private Storage bucket
 * `telemetry`, under `incoming/<batch_id>.jsonl`, with the project's secret
 * key, which no laptop ever holds. `telemetry:repo pull` takes them from
 * there into the telemetry repository. The routes are in `app.ts`, what a
 * batch must be in `batch.ts`, the books in `store.ts` and
 * `supabase/migrations/`.
 */

import postgres from "postgres";
import { createHandler } from "./app.ts";
import { configFromEnv } from "./config.ts";
import { SupabaseStorage } from "./storage.ts";
import { PgStore } from "./store.ts";

const config = configFromEnv();

if (config.databaseUrl === "") {
  throw new Error("ELLA_DB_URL is not set (nor SUPABASE_DB_URL) — see config.ts.");
}

/**
 * A remote database only ever over TLS; a local one tried encrypted first. A
 * URL that names its own `sslmode` keeps it. The URL holds the password, so
 * nothing here echoes it.
 */
function tls(url: string): "require" | "prefer" | undefined {
  if (url.includes("sslmode=")) return undefined;
  let host = "";
  try {
    host = new URL(url).hostname;
  } catch {
    throw new Error("the database URL is not a valid postgres:// URL");
  }
  return ["", "localhost", "127.0.0.1", "[::1]"].includes(host) ? "prefer" : "require";
}

const ssl = tls(config.databaseUrl);
const sql = postgres(config.databaseUrl, {
  ...(ssl === undefined ? {} : { ssl }),
  // The transaction pooler hands each query whichever connection is free.
  prepare: false,
  // Few: its books are small, and the chat proxy shares the database.
  max: 2,
  connect_timeout: 10,
  idle_timeout: 20,
  fetch_types: false,
  onnotice: () => {},
});

/**
 * Keeps `task` running after the answer has gone. Supabase's Edge Runtime
 * stops a worker once it has answered unless told to wait; served any other
 * way, a promise simply runs on.
 */
function background(task: Promise<void>): void {
  const runtime = (globalThis as { EdgeRuntime?: { waitUntil(task: Promise<unknown>): void } })
    .EdgeRuntime;
  runtime?.waitUntil(task);
}

const storage = new SupabaseStorage(config.storage);
if (!storage.configured) {
  console.warn(
    "no Storage key (SUPABASE_URL, SUPABASE_SECRET_KEYS) — every batch is kept on its laptop.",
  );
} else {
  console.info("storage", { key_from: config.storage.keyFrom });
}
if (config.pullToken === "") {
  console.warn("ELLA_TELEMETRY_PULL_TOKEN is not set (64 hex) — nothing can be pulled.");
}
if (!config.refuse) {
  console.warn("ELLA_TELEMETRY_REFUSE=off — a batch that would be refused is kept on its laptop.");
}
console.info("limits", config.limits, "policy versions", config.policyVersions);
if (config.databaseUrlFrom === "SUPABASE_DB_URL") {
  console.warn("ELLA_DB_URL is not set — using SUPABASE_DB_URL, a direct connection.");
}

const port = Deno.env.get("ELLA_PORT");
Deno.serve(
  port === undefined ? {} : { port: Number(port) },
  createHandler({
    store: new PgStore(sql),
    storage,
    limits: config.limits,
    policyVersions: config.policyVersions,
    refuse: config.refuse,
    pullToken: config.pullToken,
    background,
  }),
);
