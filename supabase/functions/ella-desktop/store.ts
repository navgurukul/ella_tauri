/**
 * The proxy's books: installs, and how much each has asked for. The tables
 * and the functions that count are in `supabase/migrations/`; each check is
 * one round trip.
 */

import type postgres from "postgres";
import type { Limits } from "./config.ts";

export type Sql = postgres.Sql;

export interface Install {
  id: string;
  revoked: boolean;
}

/** What one answered request used, and what that cost. */
export interface Usage {
  hit: number;
  miss: number;
  completion: number;
  costMicroUsd: number;
}

/** Whether a request may go ahead, or which limit stops it. */
export type Admission = "ok" | "minute" | "day" | "cap";

export interface Store {
  createInstall(tokenHash: string, appVersion: string | null): Promise<Install>;
  installByTokenHash(tokenHash: string): Promise<Install | null>;
  /** Whether the caller at `ipKey` may make one more install this hour. */
  admitInstall(ipKey: string, perHour: number): Promise<boolean>;
  admit(installId: string, limits: Limits): Promise<Admission>;
  recordUsage(installId: string, usage: Usage): Promise<void>;
}

export class PgStore implements Store {
  readonly #sql: Sql;

  constructor(sql: Sql) {
    this.#sql = sql;
  }

  async createInstall(tokenHash: string, appVersion: string | null): Promise<Install> {
    const [row] = await this.#sql<{ id: string }[]>`
      insert into ella_desktop.installs (token_hash, app_version)
      values (${tokenHash}, ${appVersion})
      returning id`;
    return { id: row.id, revoked: false };
  }

  async installByTokenHash(tokenHash: string): Promise<Install | null> {
    const rows = await this.#sql<{ id: string; revoked: boolean }[]>`
      select id, revoked_at is not null as revoked
        from ella_desktop.installs
       where token_hash = ${tokenHash}`;
    return rows[0] ?? null;
  }

  async admitInstall(ipKey: string, perHour: number): Promise<boolean> {
    const [row] = await this.#sql<{ ok: boolean }[]>`
      select ella_desktop.admit_install(${ipKey}, ${perHour}) as ok`;
    return row.ok;
  }

  async admit(installId: string, limits: Limits): Promise<Admission> {
    const cap = Math.round(limits.dailyUsd * 1_000_000);
    const [row] = await this.#sql<{ admission: Admission }[]>`
      select ella_desktop.admit(
        ${installId}::uuid, ${limits.perMinute}, ${limits.perDay}, ${cap}
      ) as admission`;
    return row.admission;
  }

  async recordUsage(installId: string, usage: Usage): Promise<void> {
    await this.#sql`
      select ella_desktop.record_usage(
        ${installId}::uuid, ${usage.hit}, ${usage.miss}, ${usage.completion},
        ${Math.round(usage.costMicroUsd)}
      )`;
  }
}

/** The same books in memory, for tests. */
export class MemoryStore implements Store {
  readonly installs = new Map<string, Install & { tokenHash: string }>();
  readonly minutes = new Map<string, number>();
  readonly days = new Map<string, { requests: number; usage: Usage }>();
  readonly hours = new Map<string, number>();
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

  admit(installId: string, limits: Limits): Promise<Admission> {
    const today = this.#today();
    let spent = 0;
    for (const [key, day] of this.days) {
      if (key.startsWith(`${today}/`)) spent += day.usage.costMicroUsd;
    }
    if (spent >= Math.round(limits.dailyUsd * 1_000_000)) return Promise.resolve("cap");

    const minute = `${installId}@${this.now().toISOString().slice(0, 16)}`;
    const n = (this.minutes.get(minute) ?? 0) + 1;
    this.minutes.set(minute, n);
    if (n > limits.perMinute) return Promise.resolve("minute");

    const day = this.#day(installId);
    day.requests += 1;
    if (day.requests > limits.perDay) return Promise.resolve("day");
    return Promise.resolve("ok");
  }

  recordUsage(installId: string, usage: Usage): Promise<void> {
    const day = this.#day(installId);
    day.usage.hit += usage.hit;
    day.usage.miss += usage.miss;
    day.usage.completion += usage.completion;
    day.usage.costMicroUsd += Math.round(usage.costMicroUsd);
    return Promise.resolve();
  }

  #today(): string {
    return this.now().toISOString().slice(0, 10);
  }

  #day(installId: string) {
    const key = `${this.#today()}/${installId}`;
    let day = this.days.get(key);
    if (day === undefined) {
      day = { requests: 0, usage: { hit: 0, miss: 0, completion: 0, costMicroUsd: 0 } };
      this.days.set(key, day);
    }
    return day;
  }
}
