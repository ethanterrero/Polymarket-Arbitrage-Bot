# polymarket-arb · dashboard

Live web dashboard for the arb bot. Subscribes to the Supabase `activity` and
`snapshots` tables in realtime and renders them as KPI cards, time-series
charts, and a scrolling event feed.

- **Stack:** Vite + React 19 + TypeScript + Tailwind v4 + Recharts +
  `@supabase/supabase-js`
- **Data source:** the Supabase project at `kawgriwaxfgvgcvyepjj.supabase.co`
  (see `supabase/migrations/20260531_init_dashboard_schema.sql`).
- **Writes:** none — the dashboard uses the publishable / anon key, which RLS
  gates to read-only. The bot writes with the `service_role` key.

## Run locally

```bash
cd dashboard
cp .env.example .env   # already populated with the public anon key
npm install
npm run dev            # http://localhost:5173
```

## Layout

| Region          | Source table | Notes                                                                  |
|-----------------|--------------|------------------------------------------------------------------------|
| Header chips    | both         | DRY-RUN / LIVE = any row with `is_live=true`; ONLINE = realtime status |
| Stats tiles     | both         | Latest snapshot for balance/exposure/positions; activity for counts    |
| Balance chart   | snapshots    | Stacked area: balance (green) + exposure (blue) over the last N points |
| Events by kind  | activity     | Distribution across `opportunity_detected`, `dry_run`, `full_fill`, …  |
| Live activity   | activity     | Newest first; INSERT events animate in via the slide-in keyframe       |

## Files

- `src/lib/supabase.ts` — anon client, reads `VITE_SUPABASE_URL` / `VITE_SUPABASE_ANON_KEY`.
- `src/lib/types.ts` — TS row shapes that mirror the SQL schema.
- `src/hooks/useActivity.ts` — initial `select … order by ts desc limit 200`,
  then `postgres_changes` INSERT subscription, dedup by id, cap to limit.
- `src/hooks/useSnapshots.ts` — same shape, oldest-first ordering for the chart.
- `src/components/*` — KPI tiles, badges, charts, activity feed.

## Smoke test (no bot needed)

Insert a row from anywhere with the service key and it appears in the feed
within ~200ms:

```sql
insert into public.activity (kind, is_live, strategy_mode, condition_id,
                             market_question, yes_price, no_price, size,
                             net_spread, expected_profit)
values ('opportunity_detected', false, 'hybrid', '0xabc',
        'smoke test', 0.45, 0.52, 50, 0.03, 1.50);
```
