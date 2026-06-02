-- Demo seed for public.activity
--
-- Populates the activity feed with a curated set of realistic
-- `opportunity_detected` + `dry_run` events so the dashboard has something
-- to render during the blockchain-club presentation when the live bot
-- hasn't naturally surfaced opportunities yet (binary arbs on Polymarket
-- are rare; a typical short collection window finds zero).
--
-- Honesty notes
-- - Kinds are limited to `opportunity_detected` and `dry_run`. These are
--   the only two `arb-recorder` ever emits in dry-run mode, so the seed
--   matches what real dry-run output looks like.
-- - Sizes / spreads / expected profits are within the same orders of
--   magnitude the live bot would produce given the configured caps
--   (max_order_size_usdc, min_net_spread = 0.001).
--
-- Idempotency
-- - Events use `gen_random_uuid()` for `id` and `now() - interval ...` for
--   `ts`, so re-running will append a fresh batch. Run `delete from
--   public.activity;` first if you want a clean reset.
--
-- Run via the Supabase SQL Editor, or `psql $DATABASE_URL -f supabase/seed_demo_activity.sql`.

begin;

insert into public.activity
  (ts, kind, is_live, strategy_mode, condition_id, market_question,
   yes_price, no_price, size, net_spread, expected_profit, total_cost)
values
  -- ── Macro / politics
  (now() - interval '7 hours 41 minutes', 'opportunity_detected', false, 'simultaneous',
   '0xa1f9b7c4e8d2f1a3c5b6d9e7f8a1b2c3d4e5f6a7b8c9d0e1f2a3b4c5d6e7f8a9',
   'Will the US Federal Reserve cut rates at the June 2026 meeting?',
   0.523, 0.475, 75, 0.0021, 0.158, null),
  (now() - interval '7 hours 40 minutes', 'dry_run', false, 'simultaneous',
   '0xa1f9b7c4e8d2f1a3c5b6d9e7f8a1b2c3d4e5f6a7b8c9d0e1f2a3b4c5d6e7f8a9',
   'Will the US Federal Reserve cut rates at the June 2026 meeting?',
   0.523, 0.475, 75, 0.0021, 0.158, null),
  (now() - interval '5 hours 12 minutes', 'opportunity_detected', false, 'simultaneous',
   '0xa1f9b7c4e8d2f1a3c5b6d9e7f8a1b2c3d4e5f6a7b8c9d0e1f2a3b4c5d6e7f8a9',
   'Will the US Federal Reserve cut rates at the June 2026 meeting?',
   0.518, 0.479, 120, 0.0028, 0.336, null),
  (now() - interval '5 hours 11 minutes', 'dry_run', false, 'simultaneous',
   '0xa1f9b7c4e8d2f1a3c5b6d9e7f8a1b2c3d4e5f6a7b8c9d0e1f2a3b4c5d6e7f8a9',
   'Will the US Federal Reserve cut rates at the June 2026 meeting?',
   0.518, 0.479, 120, 0.0028, 0.336, null),

  (now() - interval '6 hours 03 minutes', 'opportunity_detected', false, 'hybrid',
   '0xc7d3b9e1f4a8c2b5d7e1f9a4c8b3d6e2f7a1c4b9d5e8f3a6b2c7d4e9f1a5b8c3',
   'Will US Q2 2026 GDP growth exceed 2.5%?',
   0.412, 0.585, 200, 0.0026, 0.520, null),
  (now() - interval '6 hours 02 minutes', 'dry_run', false, 'hybrid',
   '0xc7d3b9e1f4a8c2b5d7e1f9a4c8b3d6e2f7a1c4b9d5e8f3a6b2c7d4e9f1a5b8c3',
   'Will US Q2 2026 GDP growth exceed 2.5%?',
   0.412, 0.585, 200, 0.0026, 0.520, null),

  (now() - interval '4 hours 28 minutes', 'opportunity_detected', false, 'simultaneous',
   '0xd9e4f1a7b3c8d2e6f9a4b1c7d3e8f2a5b9c4d1e6f3a8b7c2d5e9f4a1b6c8d3e7',
   'Will US unemployment stay below 4.0% in May 2026?',
   0.732, 0.265, 50, 0.0014, 0.070, null),

  -- ── Crypto / digital assets
  (now() - interval '7 hours 19 minutes', 'opportunity_detected', false, 'simultaneous',
   '0xb2c8d3e9f4a5b1c7d2e8f3a9b4c6d1e7f2a8b3c5d9e4f1a6b2c8d3e9f4a5b1c7',
   'Will BTC close above $110,000 on June 30, 2026?',
   0.421, 0.575, 150, 0.0024, 0.360, null),
  (now() - interval '7 hours 17 minutes', 'dry_run', false, 'simultaneous',
   '0xb2c8d3e9f4a5b1c7d2e8f3a9b4c6d1e7f2a8b3c5d9e4f1a6b2c8d3e9f4a5b1c7',
   'Will BTC close above $110,000 on June 30, 2026?',
   0.421, 0.575, 150, 0.0024, 0.360, null),
  (now() - interval '3 hours 51 minutes', 'opportunity_detected', false, 'simultaneous',
   '0xb2c8d3e9f4a5b1c7d2e8f3a9b4c6d1e7f2a8b3c5d9e4f1a6b2c8d3e9f4a5b1c7',
   'Will BTC close above $110,000 on June 30, 2026?',
   0.418, 0.580, 100, 0.0019, 0.190, null),
  (now() - interval '1 hour 17 minutes', 'opportunity_detected', false, 'simultaneous',
   '0xb2c8d3e9f4a5b1c7d2e8f3a9b4c6d1e7f2a8b3c5d9e4f1a6b2c8d3e9f4a5b1c7',
   'Will BTC close above $110,000 on June 30, 2026?',
   0.435, 0.563, 50, 0.0015, 0.075, null),

  (now() - interval '5 hours 38 minutes', 'opportunity_detected', false, 'hybrid',
   '0xe1f6a3b8c4d9e2f7a1b5c8d3e6f9a4b2c7d1e5f8a3b6c9d4e7f2a8b1c5d9e3f6',
   'Will Ethereum''s next protocol upgrade activate before August 2026?',
   0.602, 0.395, 60, 0.0026, 0.155, null),
  (now() - interval '5 hours 37 minutes', 'dry_run', false, 'hybrid',
   '0xe1f6a3b8c4d9e2f7a1b5c8d3e6f9a4b2c7d1e5f8a3b6c9d4e7f2a8b1c5d9e3f6',
   'Will Ethereum''s next protocol upgrade activate before August 2026?',
   0.602, 0.395, 60, 0.0026, 0.155, null),
  (now() - interval '2 hours 14 minutes', 'opportunity_detected', false, 'hybrid',
   '0xe1f6a3b8c4d9e2f7a1b5c8d3e6f9a4b2c7d1e5f8a3b6c9d4e7f2a8b1c5d9e3f6',
   'Will Ethereum''s next protocol upgrade activate before August 2026?',
   0.598, 0.401, 85, 0.0011, 0.094, null),

  (now() - interval '6 hours 49 minutes', 'opportunity_detected', false, 'simultaneous',
   '0xf3a8b5c2d7e4f9a1b6c3d8e5f2a7b4c9d6e1f8a3b5c2d7e4f9a1b6c3d8e5f2a7',
   'Will Bitcoin spot ETF net inflows exceed $5B in May 2026?',
   0.658, 0.339, 90, 0.0033, 0.297, null),
  (now() - interval '6 hours 47 minutes', 'dry_run', false, 'simultaneous',
   '0xf3a8b5c2d7e4f9a1b6c3d8e5f2a7b4c9d6e1f8a3b5c2d7e4f9a1b6c3d8e5f2a7',
   'Will Bitcoin spot ETF net inflows exceed $5B in May 2026?',
   0.658, 0.339, 90, 0.0033, 0.297, null),

  -- ── Sports
  (now() - interval '4 hours 57 minutes', 'opportunity_detected', false, 'asymmetric',
   '0xb7c2d9e4f1a6b8c3d5e7f2a9b4c1d6e3f8a5b2c7d4e9f1a6b3c8d5e2f7a4b1c6',
   'Will the LA Lakers make the 2026 NBA playoffs?',
   0.541, 0.456, 80, 0.0028, 0.224, null),
  (now() - interval '4 hours 56 minutes', 'dry_run', false, 'asymmetric',
   '0xb7c2d9e4f1a6b8c3d5e7f2a9b4c1d6e3f8a5b2c7d4e9f1a6b3c8d5e2f7a4b1c6',
   'Will the LA Lakers make the 2026 NBA playoffs?',
   0.541, 0.456, 80, 0.0028, 0.224, null),

  (now() - interval '6 hours 22 minutes', 'opportunity_detected', false, 'simultaneous',
   '0xa4b9c2d7e1f6a3b8c5d2e9f4a7b1c6d3e8f5a2b9c4d1e6f3a8b5c2d7e9f4a1b6',
   'Will the LA Dodgers win the 2026 World Series?',
   0.318, 0.679, 110, 0.0019, 0.209, null),

  (now() - interval '2 hours 41 minutes', 'opportunity_detected', false, 'simultaneous',
   '0xc5d1e8f3a6b9c4d7e2f5a8b1c6d3e9f4a7b2c5d8e1f6a3b9c4d7e2f5a8b1c6d3',
   'Will the SF Giants win 90+ games in the 2026 regular season?',
   0.293, 0.704, 65, 0.0017, 0.111, null),
  (now() - interval '2 hours 39 minutes', 'dry_run', false, 'simultaneous',
   '0xc5d1e8f3a6b9c4d7e2f5a8b1c6d3e9f4a7b2c5d8e1f6a3b9c4d7e2f5a8b1c6d3',
   'Will the SF Giants win 90+ games in the 2026 regular season?',
   0.293, 0.704, 65, 0.0017, 0.111, null),

  -- ── Tech / corporate
  (now() - interval '7 hours 04 minutes', 'opportunity_detected', false, 'hybrid',
   '0xd6e3f9a4b1c8d5e2f7a4b9c6d3e1f8a5b2c9d6e3f4a7b1c8d5e2f9a6b3c4d7e1',
   'Will Apple announce a new iPhone before September 2026?',
   0.871, 0.126, 35, 0.0025, 0.087, null),
  (now() - interval '7 hours 03 minutes', 'dry_run', false, 'hybrid',
   '0xd6e3f9a4b1c8d5e2f7a4b9c6d3e1f8a5b2c9d6e3f4a7b1c8d5e2f9a6b3c4d7e1',
   'Will Apple announce a new iPhone before September 2026?',
   0.871, 0.126, 35, 0.0025, 0.087, null),

  (now() - interval '6 hours 11 minutes', 'opportunity_detected', false, 'simultaneous',
   '0xe2f7a3b8c4d1e6f9a2b5c8d3e7f1a4b9c6d2e5f8a1b3c7d4e9f6a2b5c1d8e3f4',
   'Will SpaceX successfully launch Starship before July 2026?',
   0.762, 0.235, 45, 0.0025, 0.112, null),
  (now() - interval '6 hours 09 minutes', 'dry_run', false, 'simultaneous',
   '0xe2f7a3b8c4d1e6f9a2b5c8d3e7f1a4b9c6d2e5f8a1b3c7d4e9f6a2b5c1d8e3f4',
   'Will SpaceX successfully launch Starship before July 2026?',
   0.762, 0.235, 45, 0.0025, 0.112, null),
  (now() - interval '3 hours 22 minutes', 'opportunity_detected', false, 'simultaneous',
   '0xe2f7a3b8c4d1e6f9a2b5c8d3e7f1a4b9c6d2e5f8a1b3c7d4e9f6a2b5c1d8e3f4',
   'Will SpaceX successfully launch Starship before July 2026?',
   0.758, 0.240, 60, 0.0017, 0.102, null),

  (now() - interval '5 hours 59 minutes', 'opportunity_detected', false, 'hybrid',
   '0xa9b4c1d6e3f8a5b2c7d4e9f1a6b3c8d5e2f7a4b9c1d6e3f8a5b2c7d4e9f1a6b3',
   'Will Tesla deliver 500k+ vehicles in Q2 2026?',
   0.471, 0.527, 70, 0.0018, 0.126, null),

  (now() - interval '4 hours 12 minutes', 'opportunity_detected', false, 'simultaneous',
   '0xb5c2d8e4f1a7b3c9d6e2f8a4b1c7d3e9f6a2b8c5d1e7f4a9b3c6d2e8f5a1b7c4',
   'Will an OpenAI model release happen before August 2026?',
   0.838, 0.159, 40, 0.0029, 0.116, null),
  (now() - interval '4 hours 10 minutes', 'dry_run', false, 'simultaneous',
   '0xb5c2d8e4f1a7b3c9d6e2f8a4b1c7d3e9f6a2b8c5d1e7f4a9b3c6d2e8f5a1b7c4',
   'Will an OpenAI model release happen before August 2026?',
   0.838, 0.159, 40, 0.0029, 0.116, null),

  -- ── Pop culture / events
  (now() - interval '5 hours 04 minutes', 'opportunity_detected', false, 'simultaneous',
   '0xf9a1b6c3d8e5f2a7b4c9d6e3f1a8b5c2d7e4f9a1b6c3d8e5f2a7b4c9d6e3f1a8',
   'Will Taylor Swift announce a new tour before July 2026?',
   0.358, 0.640, 25, 0.0019, 0.048, null),

  (now() - interval '3 hours 06 minutes', 'opportunity_detected', false, 'hybrid',
   '0xc4d9e6f3a8b5c2d7e4f1a6b9c3d8e5f2a7b4c1d6e9f3a8b5c2d7e4f1a6b9c3d8',
   'Will the EU publish new crypto regulations before July 2026?',
   0.616, 0.380, 95, 0.0034, 0.323, null),
  (now() - interval '3 hours 04 minutes', 'dry_run', false, 'hybrid',
   '0xc4d9e6f3a8b5c2d7e4f1a6b9c3d8e5f2a7b4c1d6e9f3a8b5c2d7e4f1a6b9c3d8',
   'Will the EU publish new crypto regulations before July 2026?',
   0.616, 0.380, 95, 0.0034, 0.323, null),

  (now() - interval '1 hour 53 minutes', 'opportunity_detected', false, 'simultaneous',
   '0xd2e7f4a9b1c6d3e8f5a2b7c4d1e6f9a3b8c5d2e7f4a9b1c6d3e8f5a2b7c4d1e6',
   'Will the Atlanta Falcons make the 2026 NFL playoffs?',
   0.391, 0.605, 55, 0.0016, 0.088, null),
  (now() - interval '1 hour 51 minutes', 'dry_run', false, 'simultaneous',
   '0xd2e7f4a9b1c6d3e8f5a2b7c4d1e6f9a3b8c5d2e7f4a9b1c6d3e8f5a2b7c4d1e6',
   'Will the Atlanta Falcons make the 2026 NFL playoffs?',
   0.391, 0.605, 55, 0.0016, 0.088, null),

  (now() - interval '0 hours 47 minutes', 'opportunity_detected', false, 'simultaneous',
   '0xe6f3a9b4c1d8e5f2a7b3c6d9e4f1a8b5c2d7e3f6a9b4c1d8e5f2a7b3c6d9e4f1',
   'Will Bitcoin volatility (BVIV) exceed 75 by July 1, 2026?',
   0.286, 0.711, 30, 0.0011, 0.033, null),

  (now() - interval '0 hours 22 minutes', 'opportunity_detected', false, 'simultaneous',
   '0xf1a8b5c2d7e4f9a3b6c1d8e5f2a9b4c7d3e6f1a8b5c2d7e4f9a3b6c1d8e5f2a9',
   'Will a major US tech company announce a Bitcoin treasury allocation in 2026?',
   0.443, 0.554, 50, 0.0013, 0.065, null);

commit;

-- Quick summary of what just landed.
select kind, count(*) as n,
       round(coalesce(sum(expected_profit), 0)::numeric, 2) as total_expected_profit
  from public.activity
 group by kind
 order by n desc;
