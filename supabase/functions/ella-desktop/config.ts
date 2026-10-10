/**
 * Everything the function reads from the environment at startup.
 *
 *   ELLA_DB_URL          Postgres for the proxy's books. On Supabase, the
 *                        transaction pooler string (port 6543). Falls back to
 *                        SUPABASE_DB_URL, which Supabase sets for every
 *                        function: a direct connection, fine for a handful of
 *                        laptops, which the function warns about.
 *   DEEPSEEK_API_KEY     The desktop's own DeepSeek key. Empty: every chat
 *                        request is refused with `not_configured`, and the
 *                        laptops carry on with their local model.
 *   DEEPSEEK_BASE_URL    DeepSeek's OpenAI-compatible API base. Default
 *                        `https://api.deepseek.com`.
 *   DEEPSEEK_MODEL       Default `deepseek-flash`. Whatever a laptop asks for,
 *                        this is the model it gets, with thinking off.
 *   ELLA_DESKTOP_LIMITS  Overrides such as `minute=40,day=800,usd=5,installs=60`:
 *                        requests per install per minute and per day, the
 *                        day's spend across every install in dollars (UTC
 *                        days), and new installs per caller's address per
 *                        hour. Empty for the defaults below.
 *   ELLA_DESKTOP_PRICES  Dollars per million tokens, as `hit=0.006,miss=0.3,out=1.2`
 *                        (DeepSeek's peak prices for deepseek-flash, so the
 *                        cap errs on the safe side). Only the cap reads them.
 *   ELLA_PORT            Only for `deno task serve`: the local port.
 *
 * A value is trimmed, and an empty one counts as unset.
 */

export interface Config {
  databaseUrl: string;
  /** Which variable `databaseUrl` came from, for the log; never the value. */
  databaseUrlFrom: "ELLA_DB_URL" | "SUPABASE_DB_URL" | null;
  chat: ChatConfig;
  limits: Limits;
}

export interface ChatConfig {
  baseUrl: string;
  apiKey: string;
  model: string;
  prices: Prices;
}

/** Dollars per million tokens. */
export interface Prices {
  hit: number;
  miss: number;
  out: number;
}

export interface Limits {
  /** Requests one install may send in any minute. */
  perMinute: number;
  /** Requests one install may send in a UTC day. */
  perDay: number;
  /** Dollars every install together may spend in a UTC day. */
  dailyUsd: number;
  /** New installs one caller's address may make in an hour. */
  installsPerHour: number;
}

/**
 * Far above any learner's pace: a turn is one reply, now and then a second
 * one, and a placement check; a talk ends in three judgements. A school lab
 * shares one address, so new installs per address are generous too.
 */
export const DEFAULT_LIMITS: Limits = {
  perMinute: 40,
  perDay: 800,
  dailyUsd: 5,
  installsPerHour: 60,
};

export const DEFAULT_PRICES: Prices = { hit: 0.006, miss: 0.3, out: 1.2 };

export type Env = (key: string) => string | undefined;

export function configFromEnv(env: Env = (key) => Deno.env.get(key)): Config {
  const read = (key: string, fallback: string) => {
    const value = (env(key) ?? "").trim();
    return value === "" ? fallback : value;
  };
  const own = read("ELLA_DB_URL", "");
  const platform = read("SUPABASE_DB_URL", "");
  return {
    databaseUrl: own || platform,
    databaseUrlFrom: own ? "ELLA_DB_URL" : platform ? "SUPABASE_DB_URL" : null,
    chat: {
      baseUrl: read("DEEPSEEK_BASE_URL", "https://api.deepseek.com"),
      apiKey: read("DEEPSEEK_API_KEY", ""),
      model: read("DEEPSEEK_MODEL", "deepseek-flash"),
      prices: pricesFrom(read("ELLA_DESKTOP_PRICES", "")),
    },
    limits: limitsFrom(read("ELLA_DESKTOP_LIMITS", "")),
  };
}

/** `name=value` pairs, comma separated; anything unreadable is skipped. */
function pairs(setting: string): Array<[string, number]> {
  return setting.split(",").flatMap((part) => {
    const [name, value] = part.split("=").map((s) => s.trim());
    const n = Number(value);
    return name && value && Number.isFinite(n) && n > 0 ? [[name, n] as [string, number]] : [];
  });
}

export function limitsFrom(setting: string): Limits {
  const limits = { ...DEFAULT_LIMITS };
  for (const [name, n] of pairs(setting)) {
    if (name === "minute" && Number.isInteger(n)) limits.perMinute = n;
    if (name === "day" && Number.isInteger(n)) limits.perDay = n;
    if (name === "usd") limits.dailyUsd = n;
    if (name === "installs" && Number.isInteger(n)) limits.installsPerHour = n;
  }
  return limits;
}

export function pricesFrom(setting: string): Prices {
  const prices = { ...DEFAULT_PRICES };
  for (const [name, n] of pairs(setting)) {
    if (name === "hit") prices.hit = n;
    if (name === "miss") prices.miss = n;
    if (name === "out") prices.out = n;
  }
  return prices;
}

/** `base` and `path` joined by exactly one slash. */
export function endpoint(baseUrl: string, path: string): string {
  return `${baseUrl.replace(/\/+$/, "")}/${path.replace(/^\/+/, "")}`;
}
