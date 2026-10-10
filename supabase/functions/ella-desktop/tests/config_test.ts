import { assertEquals } from "@std/assert";
import { configFromEnv, DEFAULT_LIMITS, DEFAULT_PRICES } from "../config.ts";

Deno.test("defaults: flash, DeepSeek's own API, the pooler before the direct URL", () => {
  const env: Record<string, string> = {
    SUPABASE_DB_URL: "postgres://direct",
    ELLA_DB_URL: " postgres://pooler ",
  };
  const config = configFromEnv((key) => env[key]);
  assertEquals(config.databaseUrl, "postgres://pooler");
  assertEquals(config.databaseUrlFrom, "ELLA_DB_URL");
  assertEquals(config.chat.model, "deepseek-flash");
  assertEquals(config.chat.baseUrl, "https://api.deepseek.com");
  assertEquals(config.chat.apiKey, "");
  assertEquals(config.limits, DEFAULT_LIMITS);
  assertEquals(config.chat.prices, DEFAULT_PRICES);
});

Deno.test("limits and prices are overridden one by one, and nonsense is ignored", () => {
  const env: Record<string, string> = {
    ELLA_DESKTOP_LIMITS: "minute=10, usd=2.5, day=-4, installs=x, bogus=3",
    ELLA_DESKTOP_PRICES: "miss=0.15,out=0.6",
  };
  const config = configFromEnv((key) => env[key]);
  assertEquals(config.limits, { ...DEFAULT_LIMITS, perMinute: 10, dailyUsd: 2.5 });
  assertEquals(config.chat.prices, { hit: 0.006, miss: 0.15, out: 0.6 });
});
