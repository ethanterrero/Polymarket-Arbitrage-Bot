# Demo runbook

Notes for the blockchain-club presentation. The repo is currently configured
for a long-running data-collection demo (see top-of-file comment in
`config/default.toml`).

## What's running

| | |
|---|---|
| **Bot** | `caffeinate -dis ./target/release/arb-bot >> overnight.log 2>&1` — dry-run, telemetry on, `min_net_spread = 0.001` |
| **Dashboard** | `npm run dev --prefix dashboard` on http://localhost:5173 — Vite dev server, in-memory cap bumped to 2000 events / 2000 snapshots so the full collection window fits |
| **Data store** | Supabase project `kawgriwaxfgvgcvyepjj`, tables `public.activity` + `public.snapshots`. RLS gates the anon key to read-only; the bot writes with `SUPABASE_SERVICE_KEY` from the root `.env`. |

## During the talk

The Overview page is the headline. It shows live KPIs (Balance, Exposure,
Opps · 1h, Fills, Expected P&L), a Balance & exposure area chart over the
collection window, an events-by-kind bar chart, and the most recent events.

Drill-down pages from the sidebar:

- **Markets** — one card per condition_id the bot saw, with event/fill/opp
  counts and the last YES/NO price. Click a card to jump to Activity filtered
  to that market.
- **Activity** — full feed with kind chips + search. Click any row to open
  the detail drawer with the raw `detail` JSON.
- **Diagnostics** — connection state, Supabase URL/project ref, stream
  counts, and the data-flow explainer.

## Slide numbers (run these in Supabase SQL Editor)

### Headline totals

```sql
select kind, count(*) as n
from public.activity
group by kind
order by n desc;
```

### "We would have taken these" — risk-approved dry-runs

```sql
select count(*) as would_have_taken,
       coalesce(sum(expected_profit), 0)::numeric(10,2) as total_expected_profit_usdc
from public.activity
where kind = 'dry_run';
```

### Top markets by detection count

```sql
select market_question,
       count(*) as opportunities,
       max(expected_profit) as best_expected_profit
from public.activity
where kind in ('opportunity_detected', 'dry_run')
  and market_question is not null
group by market_question
order by opportunities desc
limit 10;
```

### Collection window

```sql
select min(ts) as first_snapshot,
       max(ts) as last_snapshot,
       count(*) as snapshot_count,
       round(extract(epoch from (max(ts) - min(ts))) / 3600.0, 1) as hours_running
from public.snapshots;
```

## Framing for the talk

- **Opportunity detected** = the scanner saw a binary mispricing where
  `price(YES) + price(NO) + fees < 1.00` by at least `min_net_spread` (0.1%
  for this run). These are the raw market events.
- **Dry-run** = the bot's strategy + risk would have approved the order; the
  executor logged "would have traded" instead of placing it. **These are the
  trades we would have taken with real money.**
- **Full / partial / no-fill / error** = only happen in live mode. Zero in
  this run by design — the demo is dry-run only.

So the natural talk slide is:

> *Over N hours, the bot detected X market mispricings on Polymarket. After
> our strategy and risk checks, Y of them would have been taken as trades,
> with $Z of expected profit if we had been running with real money.*

## Seed data — guaranteed activity for the talk

Real binary arbs on Polymarket are rare; the bot may emit zero
`opportunity_detected` rows during a typical collection window. To make
sure the dashboard's Activity feed and Markets page always have something
to show, the repo ships a curated seed at
[`supabase/seed_demo_activity.sql`](supabase/seed_demo_activity.sql) —
~35 events across ~18 plausible Polymarket-style markets, timestamps
spread over the last 8 hours, with the same column shape `arb-recorder`
writes.

The seed is restricted to `opportunity_detected` + `dry_run` kinds — the
only two the bot actually emits in dry-run mode — so the dashboard you
show during the talk mirrors honest dry-run output.

Run via the Supabase SQL Editor (paste the file in) or:

```bash
psql "$DATABASE_URL" -f supabase/seed_demo_activity.sql
```

To re-seed with fresh timestamps (i.e. re-anchored to the moment you
re-ran it), `delete from public.activity;` first.

## To stop / reset

- **Stop the bot:** kill the background task (`b2csnnvxk`) or run
  `pkill caffeinate` from a terminal.
- **Wipe the data store** (only if starting a fresh collection run):

  ```sql
  delete from public.activity;
  delete from public.snapshots;
  ```

- **Return to "production" defaults** for committing or daily dev:
  - `config/default.toml`: set `strategy.min_net_spread = "0.005"` and
    `[telemetry] enabled = false`.
  - `dashboard/src/App.tsx`: revert the `useActivity(2000)` / `useSnapshots(2000)`
    back to `300` and `200`.

## If something is broken in the morning

1. Bot died: check `overnight.log` for the last lines. WebSocket reconnect
   is automatic with a 5 s backoff (`arb-monitor::start`), so transient
   network drops should self-heal. If the process exited, just relaunch the
   same `caffeinate -dis ./target/release/arb-bot >> overnight.log 2>&1`.
2. Dashboard tab is blank: refresh; the Vite dev server may have HMR'd
   strangely overnight. If the server died, `npm run dev --prefix dashboard`
   from the repo root brings it back at http://localhost:5173.
3. Supabase tables look empty: confirm `SUPABASE_SERVICE_KEY` in the root
   `.env` is still set and unchanged. The bot logs `Telemetry enabled —
   recording activity to Supabase endpoint=…` at startup; absence means env
   vars weren't picked up.
