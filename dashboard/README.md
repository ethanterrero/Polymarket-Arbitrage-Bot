# polymarket-arb · dashboard

A small React + Vite web app that subscribes to the bot's Supabase tables and
renders what it sees in real time. The bot writes; this app only reads.

- **Stack:** Vite + React 19 + TypeScript + Tailwind v4 + Recharts +
  `@supabase/supabase-js`
- **Data:** Supabase project at `kawgriwaxfgvgcvyepjj.supabase.co`
  (schema in [`supabase/migrations/20260531_init_dashboard_schema.sql`](../supabase/migrations/20260531_init_dashboard_schema.sql))
- **Writes:** none. The dashboard uses the publishable / anon key, which RLS
  gates to read-only. The bot writes with the `service_role` key.

## Run

```bash
cd dashboard
cp .env.example .env   # already populated with the public anon key
npm install
npm run dev            # http://localhost:5173
```

`.env.example` ships the public Supabase anon key, which is safe to commit
(it can only read, and only what RLS exposes). The bot's `SUPABASE_SERVICE_KEY`
is a separate value that lives in the **root** `.env`, never here.

## Pages

The sidebar switches between four pages — each backed by the same two
realtime streams.

| Page | What it shows |
|------|---------------|
| **Overview** | 5 KPI tiles with inline sparklines (Balance, Exposure, Opps · 1h, Fills, Expected P&L), a Balance & exposure area chart, an Events-by-kind bar chart, and the most recent ~30 events. |
| **Markets** | One card per observed `condition_id`, with event/fill/opp/error counts, last Y/N price, expected profit, and time of the last update. Click a card to jump to Activity pre-filtered to that market. |
| **Activity** | Full event feed (300 most-recent rows). Filter by kind (chips with live counts) and free-text search over question / condition_id. Click any row to open a detail drawer with every field plus the raw `detail` JSON. |
| **Diagnostics** | Realtime status, Supabase URL / project ref, anon-key presence, stream counts, oldest/newest event, and a short data-flow explainer. |

The top bar also has a quiet "Latest" strip of the 4 most-recent events under
the page title, a live/dry-run pill, and an online/connecting/reconnecting
indicator.

## Files

- `src/lib/supabase.ts` — anon client. Throws at load time if env vars are
  missing rather than silently shipping a broken client.
- `src/lib/types.ts` — TS row shapes that mirror the SQL schema (`ActivityRow`,
  `SnapshotRow`).
- `src/lib/aggregate.ts` — pure helpers: `groupByMarket`,
  `snapshotSparkline`, `opportunitiesPerMinute`, `fillCumulative`.
- `src/hooks/useActivity.ts` — initial `select … order by ts desc limit N`,
  then `postgres_changes` INSERT subscription. Dedups by id, caps to N.
- `src/hooks/useSnapshots.ts` — same shape, oldest-first ordering for the
  time-series chart.
- `src/components/`
  - `layout/Sidebar.tsx`, `layout/TopBar.tsx`, `layout/Ticker.tsx`
  - `pages/{Overview,Markets,Activity,Diagnostics}Page.tsx`
  - `tiles/KpiTile.tsx`, `tiles/Sparkline.tsx`
  - `feed/FilterChips.tsx`, `feed/DetailDrawer.tsx`
  - `ActivityFeed.tsx`, `BalanceChart.tsx`, `KindBreakdown.tsx`,
    `KindBadge.tsx`, `Card.tsx`

## Smoke test (no bot needed)

Insert a row from anywhere with the service key and it appears in the feed
within a couple hundred milliseconds:

```sql
insert into public.activity (kind, is_live, strategy_mode, condition_id,
                             market_question, yes_price, no_price, size,
                             net_spread, expected_profit)
values ('opportunity_detected', false, 'hybrid', '0xabc',
        'smoke test', 0.45, 0.52, 50, 0.03, 1.50);
```

## Design notes

- **Palette:** Coinbase-inspired — cool slate near-black background, a single
  Coinbase blue accent on the brand mark, active nav rail, opportunity badge,
  online status, and the balance chart line. Classic green-up / red-down for
  fills / YES and errors / NO.
- **Typography:** Inter for everything; JetBrains Mono only for numerics.
- **No** marquee tickers, glow effects, or rainbow gradient borders by design
  — type hierarchy and tasteful color carry it.
