import { assert, assertEquals, assertMatch } from "@std/assert";
import { createHandler, type Deps } from "../app.ts";
import { type ChatConfig, DEFAULT_LIMITS, DEFAULT_PRICES, type Limits } from "../config.ts";
import { MemoryStore } from "../store.ts";

const CHAT: ChatConfig = {
  baseUrl: "https://deepseek.test",
  apiKey: "sk-test",
  model: "deepseek-flash",
  prices: DEFAULT_PRICES,
};

interface Seen {
  url: string;
  headers: Headers;
  body: Record<string, unknown>;
}

/** A proxy over a fake DeepSeek that answers with `answer`. */
function proxy(
  answer: (seen: Seen) => Response | Promise<Response>,
  options: { chat?: Partial<ChatConfig>; limits?: Partial<Limits> } = {},
) {
  const store = new MemoryStore();
  const seen: Seen[] = [];
  const pending: Promise<void>[] = [];
  const deps: Deps = {
    store,
    chat: { ...CHAT, ...options.chat },
    limits: { ...DEFAULT_LIMITS, ...options.limits },
    background: (task) => pending.push(task),
    fetch: async (input, init) => {
      const asked = {
        url: String(input),
        headers: new Headers(init?.headers),
        body: JSON.parse(String(init?.body)),
      };
      seen.push(asked);
      return await answer(asked);
    },
  };
  const handle = createHandler(deps);
  const call = (path: string, init: RequestInit = {}) =>
    handle(new Request(`http://localhost${path}`, init));
  const token = async () => {
    const response = await call("/v1/installs", { method: "POST" });
    return (await response.json()).token as string;
  };
  const chat = (token: string, body: unknown) =>
    call("/v1/chat", {
      method: "POST",
      headers: { authorization: `Bearer ${token}`, "content-type": "application/json" },
      body: JSON.stringify(body),
    });
  const settled = () => Promise.all(pending);
  return { store, seen, call, token, chat, settled };
}

const ASK = {
  messages: [
    { role: "system", content: "You are Ella." },
    { role: "user", content: "I went to market." },
  ],
  max_tokens: 50,
  temperature: 0.65,
  stream: false,
};

const ANSWER = {
  choices: [{ message: { role: "assistant", content: "Nice! What did you buy?" } }],
  usage: {
    prompt_tokens: 120,
    prompt_cache_hit_tokens: 100,
    prompt_cache_miss_tokens: 20,
    completion_tokens: 8,
  },
};

const ok = (body: unknown) =>
  new Response(JSON.stringify(body), { headers: { "content-type": "application/json" } });

function sse(parts: string[]): Response {
  const encoder = new TextEncoder();
  return new Response(
    new ReadableStream({
      start(controller) {
        for (const part of parts) controller.enqueue(encoder.encode(part));
        controller.close();
      },
    }),
    { headers: { "content-type": "text/event-stream" } },
  );
}

Deno.test("healthz says whether a key is set, at either path", async () => {
  const { call } = proxy(() => ok({}));
  for (const path of ["/healthz", "/ella-desktop/healthz"]) {
    const response = await call(path);
    assertEquals(response.status, 200);
    assertEquals(await response.json(), { ok: true, chat: true });
  }
  const bare = proxy(() => ok({}), { chat: { apiKey: "" } });
  assertEquals(await (await bare.call("/healthz")).json(), { ok: true, chat: false });
});

Deno.test("an install gets a token, and an address only so many an hour", async () => {
  const { call } = proxy(() => ok({}), { limits: { installsPerHour: 2 } });
  const first = await call("/ella-desktop/v1/installs", {
    method: "POST",
    body: JSON.stringify({ app_version: "0.1.17" }),
  });
  assertEquals(first.status, 201);
  assertMatch((await first.json()).token, /^[0-9a-f]{64}$/);
  assertEquals((await call("/v1/installs", { method: "POST" })).status, 201);
  const third = await call("/v1/installs", { method: "POST" });
  assertEquals(third.status, 429);
  assertEquals((await third.json()).limit, "installs");
});

Deno.test("a chat needs a token the proxy issued", async () => {
  const { chat, seen } = proxy(() => ok(ANSWER));
  assertEquals((await chat("nonsense", ASK)).status, 401);
  assertEquals((await chat("ab".repeat(32), ASK)).status, 401);
  assertEquals(seen.length, 0);
});

Deno.test("without a key a chat is refused so the laptop stays local", async () => {
  const { chat, token, seen } = proxy(() => ok(ANSWER), { chat: { apiKey: "" } });
  const response = await chat(await token(), ASK);
  assertEquals(response.status, 503);
  assertEquals((await response.json()).error, "not_configured");
  assertEquals(seen.length, 0);
});

Deno.test("only what Ella uses is forwarded, with the function's model and thinking off", async () => {
  const { chat, token, seen, store, settled } = proxy(() => ok(ANSWER));
  const response = await chat(await token(), {
    ...ASK,
    model: "deepseek-v4-pro",
    cache_prompt: true,
    id_slot: 0,
    timings_per_token: true,
    response_format: { type: "json_object", schema: { type: "object" } },
    reasoning_effort: "max",
    tools: [{ type: "function" }],
  });
  assertEquals(response.status, 200);
  assertEquals(await response.json(), ANSWER);

  const [asked] = seen;
  assertEquals(asked.url, "https://deepseek.test/chat/completions");
  assertEquals(asked.headers.get("authorization"), "Bearer sk-test");
  const [install] = store.installs.values();
  assertEquals(asked.body, {
    model: "deepseek-flash",
    messages: ASK.messages,
    max_tokens: 50,
    thinking: { type: "disabled" },
    temperature: 0.65,
    response_format: { type: "json_object" },
    stream: false,
    user_id: install.id,
  });

  await settled();
  const [day] = store.days.values();
  assertEquals(day.requests, 1);
  assertEquals(day.usage.hit, 100);
  assertEquals(day.usage.miss, 20);
  assertEquals(day.usage.completion, 8);
  // 100 × 0.006 + 20 × 0.3 + 8 × 1.2 millionths of a dollar.
  assertEquals(day.usage.costMicroUsd, Math.round(0.6 + 6 + 9.6));
});

Deno.test("a stream passes through byte for byte, and its usage is booked", async () => {
  const parts = [
    ": keep-alive\n\n",
    'data: {"choices":[{"delta":{"content":"Nice"}}]}\n\n',
    'data: {"choices":[{"delta":{"content":"! What did you buy?"}}],',
    '"usage":{"prompt_cache_hit_tokens":90,"prompt_cache_miss_tokens":30,"completion_tokens":9}}\n\n',
    "data: [DONE]\n\n",
  ];
  const { chat, token, seen, store, settled } = proxy(() => sse(parts));
  const response = await chat(await token(), { ...ASK, stream: true });
  assertEquals(response.status, 200);
  assertEquals(response.headers.get("content-type"), "text/event-stream");
  assertEquals(await response.text(), parts.join(""));
  assertEquals(seen[0].body.stream, true);
  assertEquals(seen[0].body.stream_options, { include_usage: true });

  await settled();
  const [day] = store.days.values();
  assertEquals(
    [day.usage.hit, day.usage.miss, day.usage.completion],
    [90, 30, 9],
  );
});

Deno.test("a stream the laptop hangs up on is booked by estimate", async () => {
  const encoder = new TextEncoder();
  let cancelled = false;
  const { chat, token, store, settled } = proxy(() =>
    new Response(
      new ReadableStream({
        start(controller) {
          controller.enqueue(
            encoder.encode('data: {"choices":[{"delta":{"content":"Nice! What"}}]}\n\n'),
          );
        },
        cancel() {
          cancelled = true;
        },
      }),
    )
  );
  const response = await chat(await token(), { ...ASK, stream: true });
  const reader = response.body!.getReader();
  await reader.read();
  await reader.cancel();
  await settled();
  assert(cancelled, "the request to DeepSeek is cancelled too");
  const [day] = store.days.values();
  const promptChars = ASK.messages.reduce((n, m) => n + m.content.length, 0);
  assertEquals(day.usage.miss, Math.ceil(promptChars / 3));
  assertEquals(day.usage.completion, Math.ceil("Nice! What".length / 3));
});

Deno.test("an answer after DeepSeek's blank keep-alive lines still reads", async () => {
  const { chat, token } = proxy(() => new Response("\n\n\n" + JSON.stringify(ANSWER)));
  const response = await chat(await token(), ASK);
  assertEquals(response.status, 200);
  assertEquals(await response.json(), ANSWER);
});

Deno.test("requests outside Ella's shape are refused before DeepSeek sees them", async () => {
  const { chat, token, seen } = proxy(() => ok(ANSWER));
  const mine = await token();
  const refusals = [
    { ...ASK, messages: [{ role: "tool", content: "x" }] },
    { ...ASK, messages: [] },
    { ...ASK, max_tokens: 5000 },
    { ...ASK, temperature: 3 },
    { ...ASK, response_format: { type: "json_schema" } },
    { ...ASK, stop: ["a", "b", "c", "d", "e"] },
    { ...ASK, messages: [{ role: "user", content: "x".repeat(24_001) }] },
  ];
  for (const body of refusals) {
    const response = await chat(mine, body);
    assertEquals(response.status, 400, JSON.stringify(body).slice(0, 80));
    assertEquals((await response.json()).error, "bad_request");
  }
  const huge = await chat(mine, { ...ASK, padding: "x".repeat(100 * 1024) });
  assertEquals(huge.status, 400);
  assertEquals(seen.length, 0);
});

Deno.test("an install over its minute is refused, and so is everyone past the day's cap", async () => {
  const { chat, token } = proxy(() => ok(ANSWER), { limits: { perMinute: 2 } });
  const mine = await token();
  assertEquals((await chat(mine, ASK)).status, 200);
  assertEquals((await chat(mine, ASK)).status, 200);
  const third = await chat(mine, ASK);
  assertEquals(third.status, 429);
  assertEquals((await third.json()).limit, "minute");

  const capped = proxy(() => ok(ANSWER), { limits: { dailyUsd: 0.00001 } });
  const other = await capped.token();
  assertEquals((await capped.chat(other, ASK)).status, 200);
  await capped.settled();
  const refused = await capped.chat(other, ASK);
  assertEquals(refused.status, 503);
  assertEquals((await refused.json()).error, "cap");
});

Deno.test("DeepSeek's failures say what the laptop should make of them", async () => {
  const cases: Array<[number, string, number]> = [
    [401, "upstream_config", 503],
    [402, "upstream_config", 503],
    [429, "upstream_busy", 429],
    [500, "upstream", 502],
    [503, "upstream", 502],
    [400, "upstream_rejected", 400],
  ];
  for (const [status, error, returned] of cases) {
    const { chat, token } = proxy(() =>
      new Response(JSON.stringify({ error: { message: `said ${status}` } }), { status })
    );
    const response = await chat(await token(), ASK);
    assertEquals(response.status, returned, `DeepSeek ${status}`);
    const body = await response.json();
    assertEquals(body.error, error, `DeepSeek ${status}`);
    if (status === 400) assertMatch(body.message, /said 400/);
  }
});

Deno.test("DeepSeek out of reach is a 502, and the key never shows in a refusal", async () => {
  const { chat, token } = proxy(() => {
    throw new TypeError("connection refused to sk-test?");
  });
  const response = await chat(await token(), ASK);
  assertEquals(response.status, 502);
  const text = await response.text();
  assertEquals(JSON.parse(text).error, "upstream");
  assert(!text.includes("sk-test"));
});
