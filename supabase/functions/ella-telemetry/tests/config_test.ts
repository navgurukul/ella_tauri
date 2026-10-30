import { assertEquals } from "@std/assert";
import { configFromEnv, DEFAULT_LIMITS } from "../config.ts";

const PULL = "9f".repeat(32);

Deno.test("defaults: the pooler before the direct URL, policy 2, refusing on", () => {
  const env: Record<string, string> = {
    SUPABASE_DB_URL: "postgres://direct",
    ELLA_DB_URL: " postgres://pooler ",
    SUPABASE_URL: "https://project.supabase.co/",
  };
  const config = configFromEnv((key) => env[key]);
  assertEquals(config.databaseUrl, "postgres://pooler");
  assertEquals(config.databaseUrlFrom, "ELLA_DB_URL");
  assertEquals(config.storage, { url: "https://project.supabase.co", key: "", keyFrom: null });
  assertEquals(config.pullToken, "");
  assertEquals(config.limits, DEFAULT_LIMITS);
  assertEquals(config.policyVersions, [2]);
  assertEquals(config.refuse, true);
});

Deno.test("the secret key's default comes first, the service-role key after", () => {
  const both: Record<string, string> = {
    SUPABASE_SECRET_KEYS: '{"default":"sb_secret_new","other":"x"}',
    SUPABASE_SERVICE_ROLE_KEY: "legacy",
  };
  assertEquals(configFromEnv((key) => both[key]).storage.key, "sb_secret_new");
  assertEquals(configFromEnv((key) => both[key]).storage.keyFrom, "SUPABASE_SECRET_KEYS");
  for (const keys of ["not json", '{"other":"x"}', '{"default":3}', ""]) {
    const env: Record<string, string> = {
      SUPABASE_SECRET_KEYS: keys,
      SUPABASE_SERVICE_ROLE_KEY: "legacy",
    };
    const storage = configFromEnv((key) => env[key]).storage;
    assertEquals([storage.key, storage.keyFrom], ["legacy", "SUPABASE_SERVICE_ROLE_KEY"], keys);
  }
});

Deno.test("a pull token is 64 hex, else there is none", () => {
  for (
    const [given, kept] of [[PULL, PULL], [` ${PULL} `, PULL], ["short", ""], [
      PULL.toUpperCase(),
      "",
    ]]
  ) {
    const env: Record<string, string> = { ELLA_TELEMETRY_PULL_TOKEN: given };
    assertEquals(configFromEnv((key) => env[key]).pullToken, kept);
  }
});

Deno.test("limits, versions and refusing are overridden, and nonsense is ignored", () => {
  const env: Record<string, string> = {
    ELLA_TELEMETRY_LIMITS: "day=10, day_mb=5, all=-4, all_mb=x, installs=7, bogus=3",
    ELLA_TELEMETRY_POLICY_VERSIONS: "2, 3, x, -1",
    ELLA_TELEMETRY_REFUSE: " OFF ",
  };
  const config = configFromEnv((key) => env[key]);
  assertEquals(config.limits, {
    ...DEFAULT_LIMITS,
    perDay: 10,
    perDayBytes: 5_000_000,
    installsPerHour: 7,
  });
  assertEquals(config.policyVersions, [2, 3]);
  assertEquals(config.refuse, false);
  const none: Record<string, string> = { ELLA_TELEMETRY_POLICY_VERSIONS: "x" };
  assertEquals(configFromEnv((key) => none[key]).policyVersions, [2]);
});
