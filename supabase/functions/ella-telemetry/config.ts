/**
 * Everything the relay reads from the environment at startup.
 *
 *   ELLA_DB_URL                     Postgres for the relay's books, as for
 *                                   `ella-desktop`; falls back to
 *                                   SUPABASE_DB_URL.
 *   SUPABASE_URL                    The project's API, which Supabase sets for
 *                                   every function. Storage is reached there.
 *   SUPABASE_SECRET_KEYS            The project's secret keys, as JSON, which
 *                                   Supabase sets for every function: its
 *                                   `default` writes to Storage. Falls back to
 *                                   SUPABASE_SERVICE_ROLE_KEY, the older form.
 *                                   Neither ever leaves the function.
 *   ELLA_TELEMETRY_PULL_TOKEN       64 hex, made and kept by the maintainers:
 *                                   what `telemetry:repo pull` sends. Unset,
 *                                   the pull routes answer `not_configured`.
 *   ELLA_TELEMETRY_LIMITS           Overrides such as
 *                                   `day=300,day_mb=64,all=30000,all_mb=3000,installs=60`:
 *                                   batches and megabytes one install may send
 *                                   in a UTC day, the same for every install
 *                                   together, and new installs per caller's
 *                                   address per hour.
 *   ELLA_TELEMETRY_POLICY_VERSIONS  The `policy_version`s taken, such as `2,3`.
 *                                   Default `2`. A batch of another is kept on
 *                                   its laptop until it is added here.
 *   ELLA_TELEMETRY_REFUSE           `off` answers `refusals_off` (503) where a
 *                                   batch would be refused, so laptops keep
 *                                   it: for when the relay is suspected of
 *                                   refusing good batches.
 *   ELLA_PORT                       Only for `deno task serve`: the local port.
 *
 * A value is trimmed, and an empty one counts as unset.
 */

export interface Config {
  databaseUrl: string;
  /** Which variable `databaseUrl` came from, for the log; never the value. */
  databaseUrlFrom: "ELLA_DB_URL" | "SUPABASE_DB_URL" | null;
  storage: StorageConfig;
  /** The pull token, or "" when none is set. */
  pullToken: string;
  limits: Limits;
  policyVersions: number[];
  refuse: boolean;
}

export interface StorageConfig {
  url: string;
  key: string;
  /** Which variable `key` came from, for the log; never the value. */
  keyFrom: "SUPABASE_SECRET_KEYS" | "SUPABASE_SERVICE_ROLE_KEY" | null;
}

export interface Limits {
  /** Batches one install may send in a UTC day. */
  perDay: number;
  /** Bytes one install may send in a UTC day. */
  perDayBytes: number;
  /** Batches every install together may send in a UTC day. */
  allPerDay: number;
  /** Bytes every install together may send in a UTC day. */
  allPerDayBytes: number;
  /** New installs one caller's address may make in an hour. */
  installsPerHour: number;
}

const MB = 1_000_000;

/**
 * A laptop sends at most 20 batches a run, a run every 30 minutes when it
 * has a backlog: a whole 32 MB backlog of 256 KB batches fits in a day.
 * The totals are far above a hundred laptops' and stop only a flood.
 */
export const DEFAULT_LIMITS: Limits = {
  perDay: 300,
  perDayBytes: 64 * MB,
  allPerDay: 30_000,
  allPerDayBytes: 3_000 * MB,
  installsPerHour: 60,
};

export const DEFAULT_POLICY_VERSIONS = [2];

export type Env = (key: string) => string | undefined;

export function configFromEnv(env: Env = (key) => Deno.env.get(key)): Config {
  const read = (key: string) => (env(key) ?? "").trim();
  const own = read("ELLA_DB_URL");
  const platform = read("SUPABASE_DB_URL");
  const secret = defaultSecretKey(read("SUPABASE_SECRET_KEYS"));
  const legacy = read("SUPABASE_SERVICE_ROLE_KEY");
  const pull = read("ELLA_TELEMETRY_PULL_TOKEN");
  const versions = policyVersionsFrom(read("ELLA_TELEMETRY_POLICY_VERSIONS"));
  return {
    databaseUrl: own || platform,
    databaseUrlFrom: own ? "ELLA_DB_URL" : platform ? "SUPABASE_DB_URL" : null,
    storage: {
      url: read("SUPABASE_URL").replace(/\/+$/, ""),
      key: secret || legacy,
      keyFrom: secret ? "SUPABASE_SECRET_KEYS" : legacy ? "SUPABASE_SERVICE_ROLE_KEY" : null,
    },
    // A token not as `openssl rand -hex 32` makes one is taken as none.
    pullToken: /^[0-9a-f]{64}$/.test(pull) ? pull : "",
    limits: limitsFrom(read("ELLA_TELEMETRY_LIMITS")),
    policyVersions: versions.length > 0 ? versions : DEFAULT_POLICY_VERSIONS,
    refuse: read("ELLA_TELEMETRY_REFUSE").toLowerCase() !== "off",
  };
}

/** `{"default": "<key>", ...}`'s default; "" for anything else. */
function defaultSecretKey(setting: string): string {
  if (setting === "") return "";
  try {
    const keys = JSON.parse(setting);
    const key = keys?.default;
    return typeof key === "string" ? key.trim() : "";
  } catch {
    return "";
  }
}

/** `name=value` pairs, comma separated; anything unreadable is skipped. */
function pairs(setting: string): Array<[string, number]> {
  return setting.split(",").flatMap((part) => {
    const [name, value] = part.split("=").map((s) => s.trim());
    const n = Number(value);
    return name && value && Number.isInteger(n) && n > 0 ? [[name, n] as [string, number]] : [];
  });
}

export function limitsFrom(setting: string): Limits {
  const limits = { ...DEFAULT_LIMITS };
  for (const [name, n] of pairs(setting)) {
    if (name === "day") limits.perDay = n;
    if (name === "day_mb") limits.perDayBytes = n * MB;
    if (name === "all") limits.allPerDay = n;
    if (name === "all_mb") limits.allPerDayBytes = n * MB;
    if (name === "installs") limits.installsPerHour = n;
  }
  return limits;
}

export function policyVersionsFrom(setting: string): number[] {
  return setting
    .split(",")
    .map((part) => Number(part.trim()))
    .filter((n) => Number.isSafeInteger(n) && n > 0);
}
