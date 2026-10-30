/**
 * The books against a real Postgres, with both migrations applied to a
 * database made for the run. Skipped unless ELLA_TEST_DB_URL names a server
 * to make it on, e.g. `postgres://localhost:54329/postgres`:
 *
 *   ELLA_TEST_DB_URL=postgres://localhost:54329/postgres deno task test
 */

import { assertEquals } from "@std/assert";
import postgres from "postgres";
import { DEFAULT_LIMITS } from "../config.ts";
import { PgStore } from "../store.ts";

const server = Deno.env.get("ELLA_TEST_DB_URL");
const MIGRATIONS = ["20261009180000_desktop_proxy.sql", "20261030120000_telemetry_relay.sql"];

Deno.test({
  name: "the migration's books count and refuse as the memory store does",
  ignore: server === undefined,
  async fn() {
    const admin = postgres(server!, { onnotice: () => {} });
    const name = `ella_telemetry_test_${crypto.randomUUID().slice(0, 8)}`;
    await admin.unsafe(`create database ${name}`);
    const url = new URL(server!);
    url.pathname = `/${name}`;
    const sql = postgres(url.toString(), { onnotice: () => {}, max: 1 });
    try {
      for (const file of MIGRATIONS) {
        await sql.unsafe(
          await Deno.readTextFile(new URL(`../../../migrations/${file}`, import.meta.url)),
        );
      }
      const store = new PgStore(sql);

      const install = await store.createInstall("a".repeat(64), "0.1.28");
      assertEquals(await store.installByTokenHash("a".repeat(64)), {
        id: install.id,
        revoked: false,
      });
      assertEquals(await store.installByTokenHash("b".repeat(64)), null);
      // A chat install's token is not a telemetry install's.
      await sql`insert into ella_desktop.installs (token_hash) values (${"c".repeat(64)})`;
      assertEquals(await store.installByTokenHash("c".repeat(64)), null);

      const limits = { ...DEFAULT_LIMITS, perDay: 2, perDayBytes: 1_000 };
      assertEquals(await store.admit(install.id, 100, limits), "ok");
      assertEquals(await store.admit(install.id, 100, limits), "ok");
      assertEquals(await store.admit(install.id, 100, limits), "day");
      const other = await store.createInstall("d".repeat(64), null);
      assertEquals(await store.admit(other.id, 1_001, limits), "day");
      assertEquals(
        await store.admit(other.id, 1, { ...limits, allPerDay: 4 }),
        "all",
      );
      const [day] = await sql`
        select batches, bytes from ella_desktop.telemetry_daily where install_id = ${install.id}`;
      assertEquals([day.batches, day.bytes], [3, "300"]);

      await store.recordRefusal("hash_mismatch");
      await store.recordRefusal("hash_mismatch");
      await store.recordRefusal("too_large");
      const refusals = await sql`
        select reason, n from ella_desktop.telemetry_refusals order by reason`;
      assertEquals(refusals.map((r) => [r.reason, r.n]), [["hash_mismatch", 2], ["too_large", 1]]);

      assertEquals(await store.admitInstall("lab", 1), true);
      assertEquals(await store.admitInstall("lab", 1), false);

      await sql`update ella_desktop.telemetry_installs set revoked_at = now()`;
      assertEquals((await store.installByTokenHash("a".repeat(64)))?.revoked, true);
    } finally {
      await sql.end();
      await admin.unsafe(`drop database ${name} with (force)`);
      await admin.end();
    }
  },
});
