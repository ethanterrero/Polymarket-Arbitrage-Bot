-- Polymarket arb bot dashboard schema.
--
-- This is the exact migration applied to the Supabase project that backs the
-- live dashboard (project ref: kawgriwaxfgvgcvyepjj). It is checked in so the
-- schema is reproducible and reviewable, not just live in the cloud.
--
--   activity  — append-only event feed: every opportunity detected, dry-run
--               "would-have-traded" event, fill, partial fill, no-fill, error.
--   snapshots — periodic bot state (balance, exposure, open positions) for the
--               time-series charts.
--
-- Writes come from the bot using the service_role key (which bypasses RLS).
-- The frontend uses the anon / publishable key and only needs read access.

create table if not exists public.activity (
    id              uuid primary key default gen_random_uuid(),
    ts              timestamptz not null default now(),
    kind            text not null check (kind in (
                        'opportunity_detected',
                        'dry_run',
                        'full_fill',
                        'partial_fill',
                        'no_fill',
                        'error'
                    )),
    is_live         boolean not null default false,
    strategy_mode   text,
    condition_id    text,
    market_question text,
    yes_price       numeric,
    no_price        numeric,
    size            numeric,
    net_spread      numeric,
    expected_profit numeric,
    total_cost      numeric,
    detail          jsonb
);

create index if not exists activity_ts_idx on public.activity (ts desc);
create index if not exists activity_kind_idx on public.activity (kind);
create index if not exists activity_condition_id_idx on public.activity (condition_id);

create table if not exists public.snapshots (
    id              uuid primary key default gen_random_uuid(),
    ts              timestamptz not null default now(),
    is_live         boolean not null default false,
    balance         numeric,
    total_exposure  numeric,
    open_positions  integer
);

create index if not exists snapshots_ts_idx on public.snapshots (ts desc);

-- Row Level Security: read-only for the anon/publishable key the frontend uses.
-- The bot inserts with the service_role key, which bypasses RLS entirely.
alter table public.activity enable row level security;
alter table public.snapshots enable row level security;

create policy "activity_anon_read"
    on public.activity for select
    to anon, authenticated
    using (true);

create policy "snapshots_anon_read"
    on public.snapshots for select
    to anon, authenticated
    using (true);

-- Realtime: let the frontend subscribe to inserts.
alter publication supabase_realtime add table public.activity;
alter publication supabase_realtime add table public.snapshots;
