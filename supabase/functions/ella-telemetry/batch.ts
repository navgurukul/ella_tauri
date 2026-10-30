/**
 * Whether a batch is whole and consistent with itself, before it is stored.
 *
 * A batch is the file `scripts/telemetry-fields.json` describes: its envelope
 * on the first line (`event: "ella_batch"`, the body, `batch_id` and the send
 * fields), then its event lines, each ending "\n". `batch_id` is the first 32
 * hex of sha256(canonical(body) + "\n" + the event lines' bytes).
 *
 * Only what is certainly wrong with the bytes themselves is refused: they
 * would be refused again however often they were sent. Whether each event
 * keeps the policy is not looked at here: ingest
 * (`scripts/latency-sanitize.py --validate`) decides that, and a second copy
 * of the policy here could only drift from it and refuse what ingest takes.
 */

import { sha256Hex } from "./hash.ts";

/** The bucket's own limit is 1 MB; nothing larger is ever written to it. */
export const MAX_BATCH_BYTES = 1_000_000;

/** The policy's `batch.body`, which `batch_id` is the hash of. */
export const BODY_FIELDS = [
  "policy_version",
  "install_id",
  "batch_seq",
  "origin",
  "app_version",
  "events",
  "talks",
  "first_event_at",
  "last_event_at",
  "dropped_event_count",
  "dropped_by_reason",
  "log_resets",
  "dropped_total",
  "log_resets_total",
  "policy_excluded_count",
  "values_replaced",
  "unknown_fields_dropped",
] as const;

export type Refusal =
  | "too_large"
  | "not_a_batch"
  | "id_mismatch"
  | "bad_envelope"
  | "hash_mismatch"
  | "bad_events";

export type Verdict =
  | { ok: true; installId: string; batchSeq: number }
  | { ok: false; refused: Refusal }
  | { ok: false; policyVersion: number };

/** JSON with its keys sorted and no spaces: what a batch's body is hashed as. */
export function canonical(value: unknown): string {
  const sorted = (item: unknown): unknown => {
    if (Array.isArray(item)) return item.map(sorted);
    if (item !== null && typeof item === "object") {
      const fields = item as Record<string, unknown>;
      return Object.fromEntries(
        Object.keys(fields).sort().map((key) => [key, sorted(fields[key])]),
      );
    }
    return item;
  };
  return JSON.stringify(sorted(value));
}

export async function batchIdOf(body: unknown, lines: Uint8Array<ArrayBuffer>): Promise<string> {
  const head = new TextEncoder().encode(canonical(body) + "\n");
  const data = new Uint8Array(head.length + lines.length);
  data.set(head);
  data.set(lines, head.length);
  return (await sha256Hex(data)).slice(0, 32);
}

/**
 * Whether canonical JSON here is canonical JSON as the app and the scripts
 * write it: only for whole numbers JavaScript holds exactly. A body with any
 * other number is not hashed here, and ingest checks it.
 */
function hashable(value: unknown): boolean {
  if (typeof value === "number") return Number.isSafeInteger(value);
  if (Array.isArray(value)) return value.every(hashable);
  if (value !== null && typeof value === "object") return Object.values(value).every(hashable);
  return true;
}

const isObject = (value: unknown): value is Record<string, unknown> =>
  value !== null && typeof value === "object" && !Array.isArray(value);

const isCount = (value: unknown): value is number =>
  typeof value === "number" && Number.isSafeInteger(value) && value >= 0;

/** The verdict on `data`, sent as the batch `batchId`. */
export async function check(
  data: Uint8Array<ArrayBuffer>,
  batchId: string,
  policyVersions: readonly number[],
): Promise<Verdict> {
  const refused = (reason: Refusal): Verdict => ({ ok: false, refused: reason });
  if (data.length > MAX_BATCH_BYTES) return refused("too_large");

  const newline = data.indexOf(0x0a);
  if (newline < 0) return refused("not_a_batch");
  let head: unknown;
  try {
    head = JSON.parse(new TextDecoder("utf-8", { fatal: true }).decode(data.subarray(0, newline)));
  } catch {
    return refused("not_a_batch");
  }
  if (!isObject(head) || head.event !== "ella_batch") return refused("not_a_batch");
  const envelope = head;
  if (envelope.batch_id !== batchId) return refused("id_mismatch");

  // A version not taken yet may lay its batch out otherwise: kept, not judged.
  const version = envelope.policy_version;
  if (!(typeof version === "number" && Number.isSafeInteger(version) && version >= 1)) {
    return refused("bad_envelope");
  }
  if (!policyVersions.includes(version)) return { ok: false, policyVersion: version };

  if (!BODY_FIELDS.every((field) => field in envelope)) return refused("bad_envelope");
  const installId = envelope.install_id;
  const batchSeq = envelope.batch_seq;
  if (
    typeof installId !== "string" || !/^[0-9a-f]{16}$/.test(installId) ||
    !isCount(batchSeq) || batchSeq < 1 ||
    envelope.origin !== "app" ||
    !isCount(envelope.events) || !isCount(envelope.talks)
  ) {
    return refused("bad_envelope");
  }

  const lines = data.subarray(newline + 1);
  const body = Object.fromEntries(BODY_FIELDS.map((field) => [field, envelope[field]]));
  if (hashable(body) && (await batchIdOf(body, lines)) !== batchId) return refused("hash_mismatch");

  if (lines.length > 0 && lines[lines.length - 1] !== 0x0a) return refused("bad_events");
  let text: string;
  try {
    text = new TextDecoder("utf-8", { fatal: true }).decode(lines);
  } catch {
    return refused("bad_events");
  }
  const events = text === "" ? [] : text.slice(0, -1).split("\n");
  for (const line of events) {
    try {
      if (!isObject(JSON.parse(line))) return refused("bad_events");
    } catch {
      return refused("bad_events");
    }
  }
  if (events.length !== envelope.events) return refused("bad_events");
  return { ok: true, installId, batchSeq };
}
