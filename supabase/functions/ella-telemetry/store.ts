/**
 * The relay's books: telemetry installs, how much each has sent today, and
 * how many batches were refused, by reason. The tables and the functions
 * that count are in `supabase/migrations/`; each check is one round trip.
 *
 * Telemetry installs are not `ella-desktop`'s installs: a token of one
 * opens nothing of the other.
 */

import type postgres from "postgres";
import type { Limits } from "./config.ts";

export type Sql = postgres.Sql;

export interface Install {
  id: string;
  revoked: boolean;
}

/** Whether a batch may be stored, or which limit stops it. */
export type Admission = "ok" | "day" | "all";

export interface Store {
  createInstall(tokenHash: string, appVersion: string | null): Promise<Install>;
  installByTokenHash(tokenHash: string): Promise<Install | null>;
  /** Whether the caller at `ipKey` may make one more install this hour. */
  admitInstall(ipKey: string, perHour: number): Promise<boolean>;
  /** Counts a batch of `bytes` against today's limits, and says whether it may go on. */
  admit(installId: string, bytes: number, limits: Limits): Promise<Admission>;
  /** Counts one refusal for `reason` today: the reason only, nothing of the batch. */
  recordRefusal(reason: string): Promise<void>;
}

export class PgStore implements Store {
  readonly #sql: Sql;

  constructor(sql: Sql) {
    this.#sql = sql;
  }

  async createInstall(tokenHash: string, appVersion: string | null): Promise<Install> {
    const [row] = await this.#sql<{ id: string }[]>`
      insert into ella_desktop.telemetry_installs (token_hash, app_version)
      values (${tokenHash}, ${appVersion})
      returning id`;
    return { id: row.id, revoked: false };
  }

  async installByTokenHash(tokenHash: string): Promise<Install | null> {
    const rows = await this.#sql<{ id: string; revoked: boolean }[]>`
      select id, revoked_at is not null as revoked
        from ella_desktop.telemetry_installs
       where token_hash = ${tokenHash}`;
    return rows[0] ?? null;
  }

  async admitInstall(ipKey: string, perHour: number): Promise<boolean> {
    const [row] = await this.#sql<{ ok: boolean }[]>`
      select ella_desktop.admit_install(${ipKey}, ${perHour}) as ok`;
    return row.ok;
  }

  async admit(installId: string, bytes: number, limits: Limits): Promise<Admission> {
    const [row] = await this.#sql<{ admission: Admission }[]>`
      select ella_desktop.admit_telemetry(
        ${installId}::uuid, ${bytes}, ${limits.perDay}, ${limits.perDayBytes},
        ${limits.allPerDay}, ${limits.allPerDayBytes}
      ) as admission`;
    return row.admission;
  }

  async recordRefusal(reason: string): Promise<void> {
    await this.#sql`select ella_desktop.record_telemetry_refusal(${reason})`;
  }
}

/** The same books in memory, for tests. */
export class MemoryStore implements Store {
  readonly installs = new Map<string, Install & { tokenHash: string }>();
  readonly days = new Map<string, { batches: number; bytes: number }>();
  readonly hours = new Map<string, number>();
  readonly refusals = new Map<string, number>();
  /** What `now` reads, so a test can move the clock. */
  now = () => new Date();

  createInstall(tokenHash: string): Promise<Install> {
    const install = { id: crypto.randomUUID(), revoked: false, tokenHash };
    this.installs.set(install.id, install);
    return Promise.resolve({ id: install.id, revoked: false });
  }

  installByTokenHash(tokenHash: string): Promise<Install | null> {
    for (const install of this.installs.values()) {
      if (install.tokenHash === tokenHash) {
        return Promise.resolve({ id: install.id, revoked: install.revoked });
      }
    }
    return Promise.resolve(null);
  }

  admitInstall(ipKey: string, perHour: number): Promise<boolean> {
    const key = `${ipKey}@${this.now().toISOString().slice(0, 13)}`;
    const n = (this.hours.get(key) ?? 0) + 1;
    this.hours.set(key, n);
    return Promise.resolve(n <= perHour);
  }

  admit(installId: string, bytes: number, limits: Limits): Promise<Admission> {
    const today = this.#today();
    let all = 0;
    let allBytes = 0;
    for (const [key, day] of this.days) {
      if (key.startsWith(`${today}/`)) {
        all += day.batches;
        allBytes += day.bytes;
      }
    }
    if (all >= limits.allPerDay || allBytes + bytes > limits.allPerDayBytes) {
      return Promise.resolve("all");
    }
    const key = `${today}/${installId}`;
    const day = this.days.get(key) ?? { batches: 0, bytes: 0 };
    day.batches += 1;
    day.bytes += bytes;
    this.days.set(key, day);
    if (day.batches > limits.perDay || day.bytes > limits.perDayBytes) {
      return Promise.resolve("day");
    }
    return Promise.resolve("ok");
  }

  recordRefusal(reason: string): Promise<void> {
    const key = `${this.#today()}/${reason}`;
    this.refusals.set(key, (this.refusals.get(key) ?? 0) + 1);
    return Promise.resolve();
  }

  #today(): string {
    return this.now().toISOString().slice(0, 10);
  }
}
