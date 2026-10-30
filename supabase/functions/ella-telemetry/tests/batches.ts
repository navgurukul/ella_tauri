/** Batches as the app makes them, for the tests: envelope, events, id. */

import { batchIdOf, BODY_FIELDS } from "../batch.ts";

export const EVENTS = [
  {
    event_id: "b599b6020bbe50d2",
    event: "ella_turn_latency",
    schema_version: 2,
    timestamp: "2026-10-30T06:00:00.000Z",
    talk_id: "t1",
    total_ms: 406,
  },
  {
    event_id: "c1d2e3f405060708",
    event: "ella_assessment",
    schema_version: 2,
    timestamp: "2026-10-30T06:05:00.000Z",
    talk_id: "t1",
  },
];

export const BODY: Record<(typeof BODY_FIELDS)[number], unknown> = {
  policy_version: 2,
  install_id: "0123456789abcdef",
  batch_seq: 1,
  origin: "app",
  app_version: "0.1.28",
  events: 2,
  talks: 1,
  first_event_at: "2026-10-30T06:00:00.000Z",
  last_event_at: "2026-10-30T06:05:00.000Z",
  dropped_event_count: 0,
  dropped_by_reason: { queue_full: 0, unreadable_line: 0, write_failed: 0 },
  log_resets: 0,
  dropped_total: 0,
  log_resets_total: 0,
  policy_excluded_count: 0,
  values_replaced: 0,
  unknown_fields_dropped: 0,
};

export const SEND = {
  prepared_at: "2026-10-30T06:40:00.000Z",
  sent_at: "2026-10-30T06:41:00.000Z",
  attempts: 1,
  first_attempt_at: "2026-10-30T06:41:00.000Z",
  oldest_queued_event_age_ms: 2_460_000,
  clock_suspect: false,
  backlog_after_events: 0,
  backlog_after_bytes: 0,
};

export interface Made {
  id: string;
  data: Uint8Array<ArrayBuffer>;
}

export function eventLines(events: unknown[]): string {
  return events.map((event) => JSON.stringify(event) + "\n").join("");
}

/**
 * A batch: `lines` as given (the events' by default), its id the hash of
 * `body` and them unless `id` says otherwise, and `envelope` laid over the
 * envelope after.
 */
export async function makeBatch(options: {
  body?: Record<string, unknown>;
  send?: Record<string, unknown>;
  lines?: string;
  id?: string;
  envelope?: Record<string, unknown>;
} = {}): Promise<Made> {
  const body = { ...BODY, ...options.body };
  const lines = options.lines ?? eventLines(EVENTS);
  const bytes = new TextEncoder().encode(lines);
  const id = options.id ?? await batchIdOf(body, bytes);
  const envelope = {
    event: "ella_batch",
    ...body,
    batch_id: id,
    ...SEND,
    ...options.send,
    ...options.envelope,
  };
  const data = new TextEncoder().encode(JSON.stringify(envelope) + "\n" + lines);
  return { id, data };
}

/** A whole batch of exactly `size` bytes: one event padded to fit. */
export async function batchOfSize(size: number): Promise<Made> {
  const padded = (pad: number) => [{ ...EVENTS[0], pad: "x".repeat(pad) }, EVENTS[1]];
  const first = await makeBatch({ lines: eventLines(padded(0)) });
  const pad = size - first.data.length;
  if (pad < 0) throw new Error(`no batch is as small as ${size} bytes`);
  const made = await makeBatch({ lines: eventLines(padded(pad)) });
  if (made.data.length !== size) throw new Error(`made ${made.data.length} bytes, not ${size}`);
  return made;
}
