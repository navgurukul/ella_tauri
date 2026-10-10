/**
 * The books against a real Postgres, with the migration applied to a database
 * made for the run. Skipped unless ELLA_TEST_DB_URL names a server to make it
 * on, e.g. `postgres://localhost:54329/postgres`:
 *
 *   ELLA_TEST_DB_URL=postgres://localhost:54329/postgres deno task test
 */

import { assertEquals } from "@std/assert";
import postgres from "postgres";
import { DEFAULT_LIMITS } from "../config.ts";
import { PgStore } from "../store.ts";

const server = Deno.env.get("ELLA_TEST_DB_URL");

Deno.test({
  name: "the migration's books count, cap and refuse as the memory store does",
  ignore: server === undefined,
  async fn() {
    const admin = postgres(server!, { onnotice: () => {} });
    const name = `ella_desktop_test_${crypto.randomUUID().slice(0, 8)}`;
    await admin.unsafe(`create database ${name}`);
    const url = new URL(server!);
    url.pathname = `/${name}`;
    const sql = postgres(url.toString(), { onnotice: () => {}, max: 1 });
    try {
      const migration = await Deno.readTextFile(
        new URL("../../../migrations/20261009180000_desktop_proxy.sql", import.meta.url),
      );
      await sql.unsafe(migration);
      const store = new PgStore(sql);

      const install = await store.createInstall("a".repeat(64), "0.1.17");
      assertEquals(await store.installByTokenHash("a".repeat(64)), {
        id: install.id,
        revoked: false,
      });
      assertEquals(await store.installByTokenHash("b".repeat(64)), null);

      const limits = { ...DEFAULT_LIMITS, perMinute: 2, perDay: 3, dailyUsd: 0.0001 };
      assertEquals(await store.admit(install.id, limits), "ok");
      assertEquals(await store.admit(install.id, limits), "ok");
      assertEquals(await store.admit(install.id, limits), "minute");

      // 100 millionths of a dollar is the cap: one request's usage reaches it.
      await store.recordUsage(install.id, { hit: 10, miss: 20, completion: 30, costMicroUsd: 100 });
      assertEquals(await store.admit(install.id, limits), "cap");
      const [day] = await sql`
        select requests, prompt_hit_tokens, prompt_miss_tokens, completion_tokens, cost_micro_usd
          from ella_desktop.daily_usage`;
      assertEquals(
        [day.requests, day.prompt_hit_tokens, day.prompt_miss_tokens, day.completion_tokens],
        [2, "10", "20", "30"],
      );

      assertEquals(await store.admitInstall("lab", 2), true);
      assertEquals(await store.admitInstall("lab", 2), true);
      assertEquals(await store.admitInstall("lab", 2), false);
      assertEquals(await store.admitInstall("home", 2), true);

      await sql`update ella_desktop.installs set revoked_at = now()`;
      assertEquals((await store.installByTokenHash("a".repeat(64)))?.revoked, true);
    } finally {
      await sql.end();
      await admin.unsafe(`drop database ${name} with (force)`);
      await admin.end();
    }
  },
});
