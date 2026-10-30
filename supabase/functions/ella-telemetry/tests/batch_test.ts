/**
 * The relay's batch checks against the policy itself: the shared vectors
 * and `scripts/telemetry-fields.json`, read here so the relay cannot drift
 * from the app and the scripts unnoticed.
 */

import { assert, assertEquals } from "@std/assert";
import { batchIdOf, BODY_FIELDS, canonical, check, MAX_BATCH_BYTES } from "../batch.ts";
import { DEFAULT_POLICY_VERSIONS } from "../config.ts";
import { BODY, makeBatch } from "./batches.ts";

const scripts = (name: string) => new URL(`../../../../scripts/${name}`, import.meta.url);
const policy = JSON.parse(await Deno.readTextFile(scripts("telemetry-fields.json")));
const vectors = JSON.parse(await Deno.readTextFile(scripts("telemetry-vectors.json")));

Deno.test("the body hashed is the policy's batch.body, and its version is taken", () => {
  assertEquals([...BODY_FIELDS], Object.keys(policy.batch.body));
  assert(DEFAULT_POLICY_VERSIONS.includes(policy.policy_version));
  assertEquals(Object.keys(BODY), Object.keys(policy.batch.body));
});

Deno.test("batch_id is the shared vector's, as the app and the scripts make it", async () => {
  const vector = vectors.batch_id;
  const lines = new TextEncoder().encode(vector.lines);
  assertEquals(await batchIdOf(vector.body, lines), vector.batch_id);
});

Deno.test("canonical JSON sorts every object's keys and has no spaces", () => {
  assertEquals(
    canonical({ b: 1, a: { d: [{ z: 1, y: null }], c: "é\n" } }),
    '{"a":{"c":"é\\n","d":[{"y":null,"z":1}]},"b":1}',
  );
});

Deno.test("a whole batch is ok, with its install and seq", async () => {
  const made = await makeBatch({ body: { batch_seq: 7 } });
  assertEquals(await check(made.data, made.id, [2]), {
    ok: true,
    installId: "0123456789abcdef",
    batchSeq: 7,
  });
});

Deno.test("too large is judged before anything is read", async () => {
  const data = new Uint8Array(MAX_BATCH_BYTES + 1);
  assertEquals(await check(data, "a".repeat(32), [2]), { ok: false, refused: "too_large" });
});

Deno.test("a number JavaScript cannot hold exactly is left to ingest, not refused", async () => {
  // Hashed by the app as written; here it would be rounded, so the id
  // cannot be checked: ingest checks it.
  const made = await makeBatch({ body: { dropped_total: 0 } });
  const text = new TextDecoder().decode(made.data).replace(
    '"dropped_total":0',
    '"dropped_total":9007199254740993',
  );
  const verdict = await check(new TextEncoder().encode(text), made.id, [2]);
  assertEquals(verdict.ok, true);
});
