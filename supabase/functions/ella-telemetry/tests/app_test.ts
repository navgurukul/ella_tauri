import { assert, assertEquals, assertMatch } from "@std/assert";
import { createHandler, type Deps } from "../app.ts";
import { MAX_BATCH_BYTES } from "../batch.ts";
import { DEFAULT_LIMITS, type Limits } from "../config.ts";
import { MemoryStorage } from "../storage.ts";
import { MemoryStore } from "../store.ts";
import { batchOfSize, eventLines, EVENTS, type Made, makeBatch } from "./batches.ts";

const PULL = "9f".repeat(32);

/** A relay over memory: `options` as its deployment would set them. */
function relay(
  options: {
    limits?: Partial<Limits>;
    policyVersions?: number[];
    refuse?: boolean;
    pullToken?: string;
  } = {},
) {
  const store = new MemoryStore();
  const storage = new MemoryStorage();
  const pending: Promise<void>[] = [];
  const deps: Deps = {
    store,
    storage,
    limits: { ...DEFAULT_LIMITS, ...options.limits },
    policyVersions: options.policyVersions ?? [2],
    refuse: options.refuse ?? true,
    pullToken: options.pullToken ?? PULL,
    background: (task) => pending.push(task),
  };
  const handle = createHandler(deps);
  const call = (path: string, init: RequestInit = {}) =>
    handle(new Request(`http://localhost${path}`, init));
  const token = async () => {
    const response = await call("/v1/installs", { method: "POST" });
    return (await response.json()).token as string;
  };
  const send = (token: string, made: Made, init: RequestInit = {}) =>
    call(`/ella-telemetry/v1/batches/${made.id}`, {
      method: "POST",
      headers: { authorization: `Bearer ${token}`, "content-type": "application/x-ndjson" },
      body: made.data,
      ...init,
    });
  const pull = (path = "", init: RequestInit = {}, bearer = PULL) =>
    call(`/v1/pull/incoming${path}`, { headers: { authorization: `Bearer ${bearer}` }, ...init });
  const settled = () => Promise.all(pending);
  return { store, storage, call, token, send, pull, settled };
}

const decode = (data: Uint8Array<ArrayBuffer> | undefined) => new TextDecoder().decode(data);

Deno.test("healthz says whether storage is set, at either path", async () => {
  const { call, storage } = relay();
  for (const path of ["/healthz", "/ella-telemetry/healthz"]) {
    const response = await call(path);
    assertEquals(response.status, 200);
    assertEquals(await response.json(), { ok: true, storage: true });
  }
  storage.configured = false;
  assertEquals(await (await call("/healthz")).json(), { ok: true, storage: false });
});

Deno.test("an install gets a telemetry token, and an address only so many an hour", async () => {
  const { call, store } = relay({ limits: { installsPerHour: 2 } });
  const first = await call("/ella-telemetry/v1/installs", {
    method: "POST",
    body: JSON.stringify({ app_version: "0.1.28" }),
  });
  assertEquals(first.status, 201);
  assertMatch((await first.json()).token, /^[0-9a-f]{64}$/);
  assertEquals((await call("/v1/installs", { method: "POST" })).status, 201);
  const third = await call("/v1/installs", { method: "POST" });
  assertEquals(third.status, 429);
  assertEquals((await third.json()).limit, "installs");
  // Only the token's hash is kept.
  for (const install of store.installs.values()) assertMatch(install.tokenHash, /^[0-9a-f]{64}$/);
});

Deno.test("a batch needs a token this relay issued, not revoked", async () => {
  const { send, token, store, storage } = relay();
  const made = await makeBatch();
  for (const wrong of ["nonsense", "ab".repeat(32)]) {
    const response = await send(wrong, made);
    assertEquals(response.status, 401);
    assertEquals((await response.json()).error, "unauthorized");
  }
  const mine = await token();
  for (const install of store.installs.values()) install.revoked = true;
  assertEquals((await send(mine, made)).status, 401);
  assertEquals(storage.objects.size, 0);
});

Deno.test("a batch is stored once under incoming/, and again is a duplicate", async () => {
  const { send, token, storage } = relay();
  const mine = await token();
  const made = await makeBatch();
  const first = await send(mine, made);
  assertEquals(first.status, 201);
  assertEquals(await first.json(), { stored: true });
  assertEquals([...storage.objects.keys()], [`incoming/${made.id}.jsonl`]);

  // A retry carries new send fields under the same id: the first copy stays.
  const retry = await makeBatch({ send: { attempts: 2, sent_at: "2026-10-30T07:00:00.000Z" } });
  assertEquals(retry.id, made.id);
  const again = await send(mine, retry);
  assertEquals(again.status, 200);
  assertEquals(await again.json(), { duplicate: true });
  assertEquals(decode(storage.objects.get(`incoming/${made.id}.jsonl`)), decode(made.data));
});

Deno.test("a batch of no events is whole", async () => {
  const { send, token } = relay();
  const made = await makeBatch({
    body: { events: 0, talks: 0, first_event_at: null, last_event_at: null },
    lines: "",
  });
  assertEquals((await send(await token(), made)).status, 201);
});

Deno.test("only application/x-ndjson is taken, and that is never a refusal", async () => {
  const { send, token, storage, store } = relay();
  const mine = await token();
  const made = await makeBatch();
  const response = await send(mine, made, {
    headers: { authorization: `Bearer ${mine}`, "content-type": "application/json" },
  });
  assertEquals(response.status, 415);
  assertEquals((await response.json()).error, "unsupported_type");
  assertEquals(storage.objects.size, 0);
  assertEquals(store.refusals.size, 0);
  const charset = await send(mine, made, {
    headers: {
      authorization: `Bearer ${mine}`,
      "content-type": "application/x-ndjson; charset=utf-8",
    },
  });
  assertEquals(charset.status, 201);
});

Deno.test("1,000,000 bytes is the most: one byte more is refused as too large", async () => {
  const { send, token, storage, store, settled } = relay();
  const mine = await token();
  const largest = await batchOfSize(MAX_BATCH_BYTES);
  assertEquals((await send(mine, largest)).status, 201);
  assertEquals((await send(mine, await batchOfSize(MAX_BATCH_BYTES - 1))).status, 201);

  const over = await batchOfSize(MAX_BATCH_BYTES + 1);
  const response = await send(mine, over);
  assertEquals(response.status, 413);
  const body = await response.json();
  assertEquals([body.error, body.reason], ["bad_batch", "too_large"]);
  assert(!storage.objects.has(`incoming/${over.id}.jsonl`));

  // The same without a declared length: counted as it is read.
  const streamed = await send(mine, over, {
    body: new ReadableStream({
      start(controller) {
        controller.enqueue(over.data.subarray(0, 500_000));
        controller.enqueue(over.data.subarray(500_000));
        controller.close();
      },
    }),
  });
  assertEquals(streamed.status, 413);
  assertEquals((await streamed.json()).reason, "too_large");
  await settled();
  assertEquals([...store.refusals.values()], [2]);
});

Deno.test("only what is certainly wrong with the bytes is refused, each by its reason", async () => {
  const { send, token, storage, store, settled } = relay();
  const mine = await token();
  const good = await makeBatch();
  const cases: Array<[string, Made]> = [
    ["not_a_batch", { id: good.id, data: new TextEncoder().encode("no newline at all") }],
    ["not_a_batch", { id: good.id, data: new TextEncoder().encode("{not json\n") }],
    ["not_a_batch", await makeBatch({ id: good.id, envelope: { event: "ella_turn_latency" } })],
    ["id_mismatch", await makeBatch({ id: good.id, envelope: { batch_id: "0".repeat(32) } })],
    ["bad_envelope", await makeBatch({ body: { origin: "manual", batch_seq: 0 } })],
    ["bad_envelope", await makeBatch({ body: { batch_seq: 0 } })],
    ["bad_envelope", await makeBatch({ body: { install_id: "laptop-of-asha" } })],
    ["bad_envelope", await makeBatch({ body: { policy_version: "2" } })],
    ["hash_mismatch", await makeBatch({ id: good.id, body: { batch_seq: 2 } })],
    [
      "hash_mismatch",
      {
        id: good.id,
        data: new TextEncoder().encode(
          decode(good.data).replace('"total_ms":406', '"total_ms":407'),
        ),
      },
    ],
    ["bad_events", await makeBatch({ body: { events: 3 } })],
    ["bad_events", await makeBatch({ lines: eventLines(EVENTS) + "not json\n" })],
    ["bad_events", await makeBatch({ lines: eventLines(EVENTS).slice(0, -1) })],
    ["bad_events", await makeBatch({ lines: eventLines(EVENTS) + "[1,2]\n" })],
  ];
  // A missing body field.
  const missing = await makeBatch();
  const envelope = JSON.parse(decode(missing.data).split("\n")[0]);
  delete envelope.values_replaced;
  cases.push([
    "bad_envelope",
    {
      id: missing.id,
      data: new TextEncoder().encode(JSON.stringify(envelope) + "\n" + eventLines(EVENTS)),
    },
  ]);

  for (const [reason, made] of cases) {
    const response = await send(mine, made);
    assertEquals(response.status, 400, reason);
    const body = await response.json();
    assertEquals([body.error, body.reason], ["bad_batch", reason]);
  }
  assertEquals(storage.objects.size, 0);
  await settled();
  const counted = Object.fromEntries(
    [...store.refusals].map(([key, n]) => [key.split("/")[1], n]),
  );
  assertEquals(counted, {
    not_a_batch: 3,
    id_mismatch: 1,
    bad_envelope: 5,
    hash_mismatch: 2,
    bad_events: 4,
  });
});

Deno.test("a policy version not taken yet is kept on the laptop, not refused", async () => {
  const later = await makeBatch({ body: { policy_version: 3 } });
  const { send, token, storage, store, settled } = relay();
  const response = await send(await token(), later);
  assertEquals(response.status, 409);
  const body = await response.json();
  assertEquals([body.error, body.policy_version], ["policy_not_accepted", 3]);
  assertEquals(storage.objects.size, 0);
  await settled();
  assertEquals(store.refusals.size, 0);

  const taken = relay({ policyVersions: [2, 3] });
  assertEquals((await taken.send(await taken.token(), later)).status, 201);
});

Deno.test("with refusing switched off, a broken batch is kept to be sent again", async () => {
  const { send, token, storage, store, settled } = relay({ refuse: false });
  const good = await makeBatch();
  const response = await send(
    await token(),
    await makeBatch({ id: good.id, body: { batch_seq: 2 } }),
  );
  assertEquals(response.status, 503);
  assertEquals((await response.json()).error, "refusals_off");
  assertEquals(storage.objects.size, 0);
  await settled();
  assertEquals([...store.refusals.values()], [1]);
});

Deno.test("storage down or not set up: the laptop keeps the batch", async () => {
  const { send, token, storage } = relay();
  const mine = await token();
  storage.fail = true;
  const down = await send(mine, await makeBatch());
  assertEquals(down.status, 503);
  assertEquals((await down.json()).error, "storage");
  storage.fail = false;
  storage.configured = false;
  const unset = await send(mine, await makeBatch());
  assertEquals(unset.status, 503);
  assertEquals((await unset.json()).error, "not_configured");
});

Deno.test("an install over its day is told to wait, and every install over the total too", async () => {
  const own = relay({ limits: { perDay: 2 } });
  const mine = await own.token();
  for (const seq of [1, 2]) {
    assertEquals((await own.send(mine, await makeBatch({ body: { batch_seq: seq } }))).status, 201);
  }
  const over = await own.send(mine, await makeBatch({ body: { batch_seq: 3 } }));
  assertEquals(over.status, 429);
  assertEquals((await over.json()).limit, "day");
  // Another install is not held back by this one.
  const another = await makeBatch({ body: { install_id: "fedcba9876543210" } });
  assertEquals((await own.send(await own.token(), another)).status, 201);

  const all = relay({ limits: { allPerDay: 1 } });
  assertEquals((await all.send(await all.token(), await makeBatch())).status, 201);
  const paused = await all.send(await all.token(), await makeBatch({ body: { batch_seq: 2 } }));
  assertEquals(paused.status, 503);
  assertEquals((await paused.json()).error, "paused");

  const bytes = relay({ limits: { perDayBytes: 2_000 } });
  const big = await bytes.send(await bytes.token(), await batchOfSize(2_001));
  assertEquals(big.status, 429);
});

Deno.test("pulling lists, reads and deletes incoming/ with the pull token only", async () => {
  const { send, token, pull, storage } = relay();
  const mine = await token();
  const one = await makeBatch();
  const two = await makeBatch({ body: { batch_seq: 2 } });
  await send(mine, one);
  await send(mine, two);

  for (const bearer of [mine, "ab".repeat(32)]) {
    assertEquals((await pull("", {}, bearer)).status, 401);
  }
  assertEquals((await pull("", { headers: {} })).status, 401);

  const listed = await pull("?limit=1000&offset=0");
  assertEquals(listed.status, 200);
  const objects = (await listed.json()).objects;
  assertEquals(
    objects.map((o: { batch_id: string }) => o.batch_id).sort(),
    [one.id, two.id].sort(),
  );
  assertEquals(
    objects.find((o: { batch_id: string }) => o.batch_id === one.id).size,
    one.data.length,
  );
  const page = await (await pull("?limit=1&offset=1")).json();
  assertEquals(page.objects.length, 1);

  const read = await pull(`/${one.id}`);
  assertEquals(read.status, 200);
  assertEquals(read.headers.get("content-type"), "application/x-ndjson");
  assertEquals(decode(new Uint8Array(await read.arrayBuffer())), decode(one.data));

  assertEquals((await pull(`/${one.id}`, { method: "DELETE" })).status, 204);
  assertEquals((await pull(`/${one.id}`, { method: "DELETE" })).status, 204);
  assertEquals((await pull(`/${one.id}`)).status, 404);
  assertEquals([...storage.objects.keys()], [`incoming/${two.id}.jsonl`]);
});

Deno.test("pulling reaches nothing but incoming/<batch_id>.jsonl", async () => {
  const { pull, call } = relay();
  for (
    const path of [
      "/..%2Fdata.jsonl",
      "/../x",
      `/${"a".repeat(31)}`,
      `/${"a".repeat(32)}.jsonl`,
      `/${"A".repeat(32)}`,
      `/${"a".repeat(32)}/more`,
    ]
  ) {
    assertEquals((await pull(path)).status, 404, path);
  }
  assertEquals((await pull("", { method: "DELETE" })).status, 404);
  assertEquals((await pull("", { method: "POST" })).status, 404);
  const other = await call("/v1/pull/other", { headers: { authorization: `Bearer ${PULL}` } });
  assertEquals(other.status, 404);
});

Deno.test("without a pull token set, nothing can be pulled", async () => {
  const { pull } = relay({ pullToken: "" });
  const response = await pull("", {}, "");
  assertEquals(response.status, 503);
  assertEquals((await response.json()).error, "not_configured");
});

Deno.test("only the defined routes answer", async () => {
  const { call, token } = relay();
  const mine = await token();
  for (
    const [path, method] of [
      ["/", "GET"],
      ["/v1/chat", "POST"],
      ["/v1/batches", "POST"],
      [`/v1/batches/${"a".repeat(32)}`, "GET"],
      [`/v1/batches/${"a".repeat(32)}.jsonl`, "POST"],
      [`/v1/batches/${"A".repeat(32)}`, "POST"],
      ["/v1/installs", "GET"],
    ]
  ) {
    const response = await call(path, { method, headers: { authorization: `Bearer ${mine}` } });
    assertEquals(response.status, 404, `${method} ${path}`);
  }
});

Deno.test("no log line holds a batch's install_id, its events or the caller's address", async () => {
  const lines: string[] = [];
  const [info, error] = [console.info, console.error];
  console.info = (...args: unknown[]) => lines.push(JSON.stringify(args));
  console.error = (...args: unknown[]) => lines.push(JSON.stringify(args));
  try {
    const { send, call, storage } = relay();
    const minted = await call("/v1/installs", {
      method: "POST",
      headers: { "x-forwarded-for": "203.0.113.7" },
    });
    const mine = (await minted.json()).token;
    const made = await makeBatch();
    await send(mine, made);
    await send(mine, made);
    await send(mine, await makeBatch({ id: made.id, body: { batch_seq: 2 } }));
    storage.fail = true;
    await send(mine, await makeBatch({ body: { batch_seq: 3 } }));
  } finally {
    console.info = info;
    console.error = error;
  }
  const logged = lines.join("\n");
  assert(lines.length >= 4);
  for (const secret of ["0123456789abcdef", "203.0.113.7", "b599b6020bbe50d2", "total_ms"]) {
    assert(!logged.includes(secret), `${secret} was logged`);
  }
});
