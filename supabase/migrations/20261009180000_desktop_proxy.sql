-- The desktop proxy's books: who may ask, and how much they have asked.
--
-- Nothing a learner says is kept here. The proxy forwards a laptop's request
-- to the language model and counts it: one row per install, one per install
-- per minute and per day, and one per caller's address per hour for new
-- installs. The function is the only client: the schema is not `public`, so
-- Supabase's REST API never serves it, and row level security is on with no
-- policies, so any role but the owner is refused every row even if the schema
-- is ever exposed by mistake.

create schema if not exists ella_desktop;
revoke all on schema ella_desktop from public;

create table ella_desktop.installs (
  id uuid primary key default gen_random_uuid(),
  -- SHA-256 of the install's token, hex: a copy of this table hands out nothing.
  token_hash text not null unique,
  app_version text,
  created_at timestamptz not null default now(),
  -- Set to turn one laptop off without touching the rest.
  revoked_at timestamptz
);

create table ella_desktop.minute_requests (
  install_id uuid not null references ella_desktop.installs (id) on delete cascade,
  minute timestamptz not null,
  n integer not null,
  primary key (install_id, minute)
);

create table ella_desktop.daily_usage (
  day date not null,
  install_id uuid not null references ella_desktop.installs (id) on delete cascade,
  requests integer not null default 0,
  prompt_hit_tokens bigint not null default 0,
  prompt_miss_tokens bigint not null default 0,
  completion_tokens bigint not null default 0,
  -- What the requests cost at the configured prices, in millionths of a dollar.
  cost_micro_usd bigint not null default 0,
  primary key (day, install_id)
);

create index daily_usage_by_day on ella_desktop.daily_usage (day);

create table ella_desktop.hourly_installs (
  -- A hash of the caller's address, never the address itself.
  ip_key text not null,
  hour timestamptz not null,
  n integer not null,
  primary key (ip_key, hour)
);

alter table ella_desktop.installs enable row level security;
alter table ella_desktop.minute_requests enable row level security;
alter table ella_desktop.daily_usage enable row level security;
alter table ella_desktop.hourly_installs enable row level security;

-- Whether one more request from `p_install` may go to the model, in one round
-- trip: 'cap' when today's spend across every install has reached the cap,
-- 'minute' or 'day' when this install is over its own limit, else 'ok'. The
-- request is counted either way, so a client that keeps asking stays refused.
create function ella_desktop.admit(
  p_install uuid,
  p_per_minute integer,
  p_per_day integer,
  p_cap_micro_usd bigint
) returns text
language plpgsql
as $$
declare
  v_today date := (now() at time zone 'utc')::date;
  v_spent bigint;
  v_minute integer;
  v_day integer;
begin
  select coalesce(sum(cost_micro_usd), 0) into v_spent
    from ella_desktop.daily_usage
   where day = v_today;
  if v_spent >= p_cap_micro_usd then
    return 'cap';
  end if;

  insert into ella_desktop.minute_requests as m (install_id, minute, n)
  values (p_install, date_trunc('minute', now()), 1)
  on conflict (install_id, minute) do update set n = m.n + 1
  returning n into v_minute;
  if v_minute > p_per_minute then
    return 'minute';
  end if;

  insert into ella_desktop.daily_usage as d (day, install_id, requests)
  values (v_today, p_install, 1)
  on conflict (day, install_id) do update set requests = d.requests + 1
  returning requests into v_day;
  if v_day > p_per_day then
    return 'day';
  end if;

  -- Now and then, forget counts nobody reads any more.
  if random() < 0.01 then
    delete from ella_desktop.minute_requests where minute < now() - interval '1 hour';
    delete from ella_desktop.hourly_installs where hour < now() - interval '2 days';
  end if;
  return 'ok';
end;
$$;

-- Adds what one answered request used to today's books for its install.
create function ella_desktop.record_usage(
  p_install uuid,
  p_hit bigint,
  p_miss bigint,
  p_completion bigint,
  p_cost_micro_usd bigint
) returns void
language sql
as $$
  insert into ella_desktop.daily_usage as d (
    day, install_id, requests,
    prompt_hit_tokens, prompt_miss_tokens, completion_tokens, cost_micro_usd
  )
  values (
    (now() at time zone 'utc')::date, p_install, 0,
    p_hit, p_miss, p_completion, p_cost_micro_usd
  )
  on conflict (day, install_id) do update set
    prompt_hit_tokens = d.prompt_hit_tokens + excluded.prompt_hit_tokens,
    prompt_miss_tokens = d.prompt_miss_tokens + excluded.prompt_miss_tokens,
    completion_tokens = d.completion_tokens + excluded.completion_tokens,
    cost_micro_usd = d.cost_micro_usd + excluded.cost_micro_usd;
$$;

-- Whether the caller at `p_ip_key` may make one more install this hour.
create function ella_desktop.admit_install(p_ip_key text, p_per_hour integer)
returns boolean
language plpgsql
as $$
declare
  v_n integer;
begin
  insert into ella_desktop.hourly_installs as h (ip_key, hour, n)
  values (p_ip_key, date_trunc('hour', now()), 1)
  on conflict (ip_key, hour) do update set n = h.n + 1
  returning n into v_n;
  return v_n <= p_per_hour;
end;
$$;

revoke all on all tables in schema ella_desktop from public;
revoke all on all functions in schema ella_desktop from public;

-- Supabase's API roles, where they exist: none of this is theirs.
do $$
begin
  if exists (select 1 from pg_roles where rolname = 'anon') then
    revoke all on schema ella_desktop from anon, authenticated;
    revoke all on all tables in schema ella_desktop from anon, authenticated;
    revoke all on all functions in schema ella_desktop from anon, authenticated;
  end if;
end;
$$;
