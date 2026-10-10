/**
 * Ella Desktop's language-model proxy, as one Supabase Edge Function.
 *
 * It holds the desktop's DeepSeek key, so no laptop ever does: a laptop gets
 * an install token once and sends its chat requests here. The routes are in
 * `app.ts`, what may be forwarded in `chat.ts`, the books in `store.ts` and
 * `supabase/migrations/`.
 */

import postgres from "postgres";
import { createHandler } from "./app.ts";
import { configFromEnv } from "./config.ts";
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
  max: 3,
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

if (config.chat.apiKey === "") {
  console.warn("DEEPSEEK_API_KEY is not set — every chat request is refused; laptops stay local.");
} else {
  console.info("DeepSeek configured", { base_url: config.chat.baseUrl, model: config.chat.model });
}
console.info("limits", config.limits);
if (config.databaseUrlFrom === "SUPABASE_DB_URL") {
  console.warn("ELLA_DB_URL is not set — using SUPABASE_DB_URL, a direct connection.");
}

const port = Deno.env.get("ELLA_PORT");
Deno.serve(
  port === undefined ? {} : { port: Number(port) },
  createHandler({
    store: new PgStore(sql),
    chat: config.chat,
    limits: config.limits,
    background,
  }),
);
