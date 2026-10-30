-- The telemetry relay's books (the `ella-telemetry` function): its own
-- installs, how much each has sent in a day, and how many batches were
-- refused, by reason.
--
-- Nothing of a batch is kept here: batches go to the private Storage bucket
-- `telemetry`, and a refused one is counted by its reason only. A telemetry
-- install is not a chat install (`ella_desktop.installs`): a token of one
-- opens nothing of the other, and neither table names the other. As for the
-- proxy's books, the function is the only client: the schema is not exposed,
-- row level security is on with no policies, and the API roles have no
-- grants. Nothing that exists already is changed.

create table ella_desktop.telemetry_installs (
  id uuid primary key default gen_random_uuid(),
  -- SHA-256 of the install's token, hex: a copy of this table hands out nothing.
  token_hash text not null unique,
  app_version text,
  created_at timestamptz not null default now(),
  -- Set to turn one laptop's sending off without touching the rest.
  revoked_at timestamptz
);

create table ella_desktop.telemetry_daily (
  day date not null,
  install_id uuid not null references ella_desktop.telemetry_installs (id) on delete cascade,
  batches integer not null default 0,
  bytes bigint not null default 0,
  primary key (day, install_id)
);

create table ella_desktop.telemetry_refusals (
  day date not null,
  reason text not null,
  n integer not null default 0,
  primary key (day, reason)
);

alter table ella_desktop.telemetry_installs enable row level security;
alter table ella_desktop.telemetry_daily enable row level security;
alter table ella_desktop.telemetry_refusals enable row level security;

-- Whether one more batch of `p_bytes` from `p_install` may be stored, in one
-- round trip: 'all' when today's batches or bytes across every install have
-- reached their limit, 'day' when this install is over its own, else 'ok'.
-- The batch is counted against its install either way, as `admit` counts a
-- chat request, so a client that keeps sending stays refused.
create function ella_desktop.admit_telemetry(
  p_install uuid,
  p_bytes bigint,
  p_per_day integer,
  p_per_day_bytes bigint,
  p_all_per_day integer,
  p_all_per_day_bytes bigint
) returns text
language plpgsql
as $$
declare
  v_today date := (now() at time zone 'utc')::date;
  v_all integer;
  v_all_bytes bigint;
  v_batches integer;
  v_bytes bigint;
begin
  select coalesce(sum(batches), 0), coalesce(sum(bytes), 0) into v_all, v_all_bytes
    from ella_desktop.telemetry_daily
   where day = v_today;
  if v_all >= p_all_per_day or v_all_bytes + p_bytes > p_all_per_day_bytes then
    return 'all';
  end if;

  insert into ella_desktop.telemetry_daily as d (day, install_id, batches, bytes)
  values (v_today, p_install, 1, p_bytes)
  on conflict (day, install_id) do update
    set batches = d.batches + 1, bytes = d.bytes + excluded.bytes
  returning batches, bytes into v_batches, v_bytes;
  if v_batches > p_per_day or v_bytes > p_per_day_bytes then
    return 'day';
  end if;

  -- Now and then, forget days nobody reads any more.
  if random() < 0.01 then
    delete from ella_desktop.telemetry_daily where day < v_today - 90;
  end if;
  return 'ok';
end;
$$;

-- Counts one refused batch for `p_reason` today.
create function ella_desktop.record_telemetry_refusal(p_reason text)
returns void
language sql
as $$
  insert into ella_desktop.telemetry_refusals as r (day, reason, n)
  values ((now() at time zone 'utc')::date, p_reason, 1)
  on conflict (day, reason) do update set n = r.n + 1;
$$;

revoke all on ella_desktop.telemetry_installs, ella_desktop.telemetry_daily,
  ella_desktop.telemetry_refusals from public;
revoke all on function ella_desktop.admit_telemetry(uuid, bigint, integer, bigint, integer, bigint),
  ella_desktop.record_telemetry_refusal(text) from public;

-- Supabase's API roles, where they exist: none of this is theirs.
do $$
begin
  if exists (select 1 from pg_roles where rolname = 'anon') then
    revoke all on ella_desktop.telemetry_installs, ella_desktop.telemetry_daily,
      ella_desktop.telemetry_refusals from anon, authenticated;
    revoke all on function ella_desktop.admit_telemetry(uuid, bigint, integer, bigint, integer, bigint),
      ella_desktop.record_telemetry_refusal(text) from anon, authenticated;
  end if;
end;
$$;
