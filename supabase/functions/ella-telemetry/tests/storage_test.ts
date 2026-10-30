/** Storage's API as the relay calls it, against a fake that records each call. */

import { assert, assertEquals, assertRejects } from "@std/assert";
import { SupabaseStorage } from "../storage.ts";

const ID = "0f8d6371cfd6e793b27b17d509751e84";
const KEY = "sb_secret_test_key";

interface Seen {
  method: string;
  url: string;
  headers: Headers;
  body: string;
}

function storage(answer: (seen: Seen) => Response) {
  const seen: Seen[] = [];
  const fake: typeof fetch = async (input, init) => {
    const call = {
      method: init?.method ?? "GET",
      url: String(input),
      headers: new Headers(init?.headers),
      body: init?.body instanceof Uint8Array
        ? new TextDecoder().decode(init.body)
        : String(init?.body ?? ""),
    };
    seen.push(call);
    return await Promise.resolve(answer(call));
  };
  return {
    store: new SupabaseStorage({ url: "https://p.supabase.co", key: KEY, keyFrom: null }, fake),
    seen,
  };
}

const json = (body: unknown, status = 200) =>
  new Response(JSON.stringify(body), { status, headers: { "content-type": "application/json" } });

Deno.test("a batch is written to incoming/ once, with the key and no upsert", async () => {
  const { store, seen } = storage(() => json({ Key: `telemetry/incoming/${ID}.jsonl` }));
  assertEquals(await store.put(ID, new TextEncoder().encode("{}\n")), "stored");
  const [call] = seen;
  assertEquals(call.method, "POST");
  assertEquals(call.url, `https://p.supabase.co/storage/v1/object/telemetry/incoming/${ID}.jsonl`);
  assertEquals(call.headers.get("x-upsert"), "false");
  assertEquals(call.headers.get("content-type"), "application/x-ndjson");
  assertEquals(call.headers.get("apikey"), KEY);
  assertEquals(call.headers.get("authorization"), `Bearer ${KEY}`);
  assertEquals(call.body, "{}\n");
});

Deno.test("there already, in either of Storage's answers, is a duplicate", async () => {
  const answers = [
    () =>
      json({ statusCode: "409", error: "Duplicate", message: "The resource already exists" }, 409),
    () =>
      json({ statusCode: "409", error: "Duplicate", message: "The resource already exists" }, 400),
  ];
  for (const answer of answers) {
    assertEquals(await storage(answer).store.put(ID, new Uint8Array(1)), "exists");
  }
});

Deno.test("any other refusal, or no answer, is a failure to try again, without the key", async () => {
  for (
    const answer of [
      () => json({ statusCode: "413", error: "Payload too large" }, 400),
      () => json({ error: "nope" }, 413),
      () => json({}, 500),
      () => new Response("<html>bad gateway</html>", { status: 502 }),
    ]
  ) {
    await assertRejects(() => storage(answer).store.put(ID, new Uint8Array(1)));
  }
  const unreachable = new SupabaseStorage(
    { url: "https://p.supabase.co", key: KEY, keyFrom: null },
    () => Promise.reject(new Error(`connection refused while sending ${KEY}`)),
  );
  const error = await assertRejects(() => unreachable.put(ID, new Uint8Array(1)));
  assert(!String(error).includes(KEY));
});

Deno.test("only a batch id becomes a path", async () => {
  const { store, seen } = storage(() => json({}));
  for (const id of ["../x", "a".repeat(31), "A".repeat(32), `${ID}/x`]) {
    await assertRejects(() => store.put(id, new Uint8Array(1)));
    await assertRejects(() => store.get(id));
    await assertRejects(() => store.remove(id));
  }
  assertEquals(seen.length, 0);
});

Deno.test("the listing is of incoming/ only, and only batch files from it", async () => {
  const { store, seen } = storage(() =>
    json([
      { name: `${ID}.jsonl`, created_at: "2026-10-30T06:00:00Z", metadata: { size: 1234 } },
      { name: "notes.txt", metadata: { size: 1 } },
      { name: "nested", id: null, metadata: null },
    ])
  );
  assertEquals(await store.list(50, 100), {
    objects: [{ batchId: ID, size: 1234, createdAt: "2026-10-30T06:00:00Z" }],
    listed: 3,
  });
  const [call] = seen;
  assertEquals(call.url, "https://p.supabase.co/storage/v1/object/list/telemetry");
  assertEquals(JSON.parse(call.body), {
    prefix: "incoming",
    limit: 50,
    offset: 100,
    sortBy: { column: "name", order: "asc" },
  });
});

Deno.test("a read gives the bytes, or null when the batch is gone", async () => {
  const found = storage(() => new Response("{}\n"));
  assertEquals(new TextDecoder().decode((await found.store.get(ID))!), "{}\n");
  assertEquals(
    found.seen[0].url,
    `https://p.supabase.co/storage/v1/object/authenticated/telemetry/incoming/${ID}.jsonl`,
  );
  for (const answer of [() => json({}, 404), () => json({ statusCode: "404" }, 400)]) {
    assertEquals(await storage(answer).store.get(ID), null);
  }
});

Deno.test("a delete names one object in incoming/", async () => {
  const { store, seen } = storage(() => json([]));
  await store.remove(ID);
  assertEquals(seen[0].method, "DELETE");
  assertEquals(seen[0].url, "https://p.supabase.co/storage/v1/object/telemetry");
  assertEquals(JSON.parse(seen[0].body), { prefixes: [`incoming/${ID}.jsonl`] });
  await assertRejects(() => storage(() => json({}, 500)).store.remove(ID));
});
