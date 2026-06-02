# Polymarket Arbitrage Bot

A Rust bot that detects and executes **binary prediction market arbitrage** on [Polymarket](https://polymarket.com). It watches active markets' orderbooks in real time and, when it finds a mispricing (buying both YES and NO costs less than $1), can place orders to lock in risk-free profit.

A separate **React dashboard** in [`dashboard/`](dashboard/) visualizes everything the bot does in real time via Supabase Realtime.

> **Status at a glance.** Detection + execution work end-to-end on Polymarket CLOB V1. Default mode is dry-run. Live mode is wired (EIP-712 signing, HMAC L2 auth, on-chain allowance check at boot) but should only be opted into with tiny size. CLOB V2 migration is in flight on a branch. 77 tests, `cargo test --workspace` green on `main`.
>
> **Current `main` is configured for a blockchain-club demo run** — telemetry on, looser `min_net_spread`, dashboard caps bumped. See [DEMO.md](DEMO.md) for the runbook and the slide-number queries. To return to production-style defaults, see the bottom of `DEMO.md`.

For day-to-day engineering notes, per-PR rationale, and the ordered backlog, see **[devlog.md](devlog.md)**. For the term-project plan and scope decisions, see **[docs/TERM_PROJECT_PLAN.md](docs/TERM_PROJECT_PLAN.md)**.

---

## Quick start

### Run the bot (dry-run)

```bash
cargo run --release -p arb-bot
```

The binary reads `config/default.toml` from the current working directory — run from the repo root. Default config is dry-run; no key needed.

### Run the dashboard

```bash
cd dashboard
cp .env.example .env   # already pre-populated with the public Supabase anon key
npm install
npm run dev            # http://localhost:5173
```

The dashboard reads from a Supabase project gated read-only by RLS. It works on its own (empty state until events arrive). To see live data, also run the bot with telemetry enabled — see [Wiring the bot to the dashboard](#wiring-the-bot-to-the-dashboard) below.

---

## How the arbitrage works

In a binary prediction market, exactly one of two outcomes happens: **YES** or **NO**. You can buy YES tokens and NO tokens. When the market resolves:

- If YES wins: each YES token pays **$1**, each NO token pays **$0**
- If NO wins: each NO token pays **$1**, each YES token pays **$0**

So if you hold **one YES** and **one NO**, you get **$1** no matter what. If you can buy that pair for **less than $1**, you profit when the market resolves.

**Arbitrage condition:** `price(YES) + price(NO) < 1.0` (after fees)

The bot looks for exactly that on the live orderbook: best ask on YES + best ask on NO < $1, with enough size on both sides and after accounting for fees.

---

## Architecture

The bot is a Rust workspace; the dashboard is a separate Vite/React app that reads from Supabase. Everything is loosely coupled — the trading core has no dependency on the recorder, and the dashboard has no dependency on the bot beyond the shared schema.

```
                                                  ┌────────────────┐
                                                  │  Polymarket    │
                                                  │  Gamma / CLOB  │
                                                  │  / WebSocket   │
                                                  └────────┬───────┘
                                                           │
   ┌─────────────────┐   ┌─────────────────┐   ┌──────────▼──────┐
   │  MarketScanner  │──▶│  PriceMonitor   │──▶│ ArbitrageDetect │
   │  (Gamma API)    │   │  (CLOB REST/WS) │   │  (strategy)     │
   └─────────────────┘   └────────┬────────┘   └────────┬────────┘
                                  │                     │
                                  │  BinaryOrderBook    │ opportunity
                                  │                     ▼
                                  │            ┌─────────────────┐
                                  │            │  RiskManager    │
                                  │            │  (limits, etc.) │
                                  │            └────────┬────────┘
                                  │                     │ approved order
                                  │                     ▼
                                  │            ┌─────────────────┐
                                  │            │  OrderExecutor  │ ──▶ Polymarket
                                  │            │  (CLOB + auth)  │     CLOB
                                  └────────────┴────────┬────────┘
                                                        │
                                                        ▼
                                              ┌─────────────────┐    Realtime    ┌──────────────┐
                                              │  arb-recorder   │ ─────────────▶ │   Supabase   │
                                              │  (telemetry)    │   PostgREST    │ activity +   │
                                              └─────────────────┘                │ snapshots    │
                                                                                 └──────┬───────┘
                                                                                        │ websocket
                                                                                        ▼
                                                                                 ┌──────────────┐
                                                                                 │  Dashboard   │
                                                                                 │  (React)     │
                                                                                 └──────────────┘
```

**Asymmetric mode** also uses **InventoryManager** (`arb-inventory`) to track unpaired legs and pairing; the main loop can run **simultaneous**, **asymmetric**, or **hybrid** strategy (see `strategy.mode` in config).

### Crates

| Crate | Role |
|-------|------|
| `arb-bot` | Entrypoint and main loop. Wires the scanner, monitor, strategy, risk, executor, recorder. |
| `arb-scanner` | Fetches active binary markets from Polymarket's **Gamma API**. Filters by liquidity / volume / time-to-resolution; refreshes periodically. |
| `arb-monitor` | Pulls orderbook data per market (WebSocket live or REST polling). Emits `BinaryOrderBook` snapshots into a channel. Can fetch balance from the CLOB for risk checks. |
| `arb-strategy` | Detects arb on each book. Single-level, **multi-level sweep**, and asymmetric leg logic. Checks staleness, fees, and `min_net_spread`. |
| `arb-risk` | Before any trade: max order size, exposure, reserve balance, per-market cooldown, max concurrent positions, unpaired limits. Records execution results. |
| `arb-inventory` | Tracks open legs and paired positions for asymmetric / hybrid modes. |
| `arb-executor` | **L2 auth** (derive API key via EIP-712, then HMAC `POLY_*` per request), places YES + NO buys (FOK/GTC). EIP-712 **signed order body** per the Polymarket CLOB V1 ABI. |
| `arb-recorder` | **Additive** telemetry sink — fire-and-forget POSTs to Supabase. Trading core does not depend on this crate; no-op when telemetry is disabled. |
| `arb-config` | Loads `config/default.toml` and env overrides (e.g. `STRATEGY__MIN_NET_SPREAD`). Private key comes from `POLYMARKET_PRIVATE_KEY` only. |
| `arb-types` | Shared types: `BinaryMarket`, `BinaryOrderBook`, `StrategyMode`, opportunities, execution orders, etc. |

### Main loop

1. Load config; build scanner, monitor, detector, risk manager, inventory (for asymmetric/hybrid), executor, recorder.
2. **Executor mode:** if `execution.mode = "live"` and `POLYMARKET_PRIVATE_KEY` is set, the bot calls `OrderExecutor::new_live` (derive API key + HMAC, then verify on-chain allowances). On missing key or derivation failure, it falls back to **dry-run** with a warning.
3. **Initial scan:** fetch binary markets from Gamma, build `condition_id → market` lookup.
4. **Background tasks:**
   - Market refresh on `scanner.refresh_interval_secs`.
   - Balance fetch + `RiskManager::update_balance` on interval (and a `record_snapshot` if telemetry is on).
   - Stale-leg logging for asymmetric flows.
   - Price monitor pushes `BinaryOrderBook` into a channel.
5. **Main loop:** for each book, resolve market → detect → risk → spawn execution → record.

The bot is **event-driven**: it reacts to orderbook updates and only acts when an opportunity passes strategy and risk.

### Execution status

Live execution targets Polymarket's **CLOB V1**. CLOB V2 migration is in flight (drops `taker`/`expiration`/`nonce`/`feeRateBps`, adds `timestamp`/`metadata`/`builder`; domain version `"1"` → `"2"`) on `feat/clob-v2-migration` and **not yet on `main`**.

- **Dry-run** (default): no key, or `execution.mode = "dry_run"` → logs what would be traded; no signed orders.
- **Authenticated HTTP**: with a key, the executor attaches **L2 HMAC** headers and posts to `/order`.
- **Signed order body (V1)**: full EIP-712-signed `Order` struct on the wire (`order_signing.rs` reference-tested against Polymarket's vectors; `proxy_address.rs` derives the funded `maker` for proxy + Gnosis Safe accounts; BUY amount quantization pinned against `py-clob-client`'s `get_order_amounts`).
- **Per-market fee rate (V1)**: `Order.feeRateBps` is fetched lazily from `GET /fee-rate?token_id=<id>` and cached per token — the value goes into the EIP-712 digest, so a wrong default would cause silent CLOB rejections.
- **Startup allowance check**: live mode verifies USDC.e and ConditionalTokens approvals on both CTF Exchange contracts (standard + neg-risk) on Polygon before enabling order placement. Missing approvals fail at boot rather than at first attempted fill.

---

## Configuration

Edit `config/default.toml` (and optionally override with env vars using `__`, e.g. `STRATEGY__MIN_NET_SPREAD=0.01`).

| Section | Key | Meaning |
|---------|-----|---------|
| **polymarket** | `clob_url`, `gamma_url`, `ws_url`, `chain_id` | CLOB, Gamma, WebSocket endpoints + chain ID. |
| | `polygon_rpc_url` | Polygon JSON-RPC for the startup allowance check (default `https://polygon-rpc.com`; override if rate-limited). |
| **strategy** | `mode` | `simultaneous`, `asymmetric`, or `hybrid`. |
| | `min_net_spread`, `base_fee_rate`, `max_price_staleness_ms`, `use_fok_orders` | Core thresholds and ergonomics. |
| | `max_sweep_levels`, `min_sweep_profit_usdc` | Multi-level sweep tuning (0 disables sweep-first path). |
| | `asymmetric_target_total_cost`, `max_unpaired_hold_secs`, `max_unpaired_exposure_usdc`, `max_unpaired_legs_per_market` | Asymmetric tuning. |
| **risk** | `max_order_size_usdc`, `max_total_exposure_usdc`, `min_reserve_balance_usdc`, `per_market_cooldown_secs`, `max_concurrent_positions` | Trading limits. |
| | `max_unpaired_exposure_usdc`, `max_unpaired_per_market_usdc` | Extra limits for unpaired inventory. |
| **scanner** | `refresh_interval_secs`, `max_markets`, `min_liquidity_usdc` | Discovery cadence and filters. |
| **monitor** | `use_websocket`, `poll_interval_ms`, `max_concurrent_requests` | Orderbook transport and concurrency. |
| **execution** | `mode` | `"dry_run"` (default) or `"live"` (explicit opt-in). |
| **telemetry** | `enabled` | Send activity + snapshots to Supabase for the dashboard. Default `false`. Requires `SUPABASE_URL` + `SUPABASE_SERVICE_KEY` in `.env`. |

---

## Wiring the bot to the dashboard

The dashboard reads from Supabase; the bot writes to Supabase via the `arb-recorder` crate. They're decoupled — running one without the other is fine.

To turn on telemetry:

1. Copy `.env.example` to `.env` (if you haven't).
2. Set `SUPABASE_URL` to the project URL and `SUPABASE_SERVICE_KEY` to the **service_role** key from Supabase → Project Settings → API. **The service key bypasses RLS — server-side only, never commit, never ship to the frontend.**
3. In `config/default.toml`, set `[telemetry] enabled = true`.
4. Run the bot. The snapshot loop fires every ~30s so the dashboard shows balance/exposure curves within a minute, even before a real arb appears.

The dashboard subscribes via Supabase Realtime — rows appear within a couple hundred milliseconds of insert.

---

## Live trading: caveats

Live mode is opt-in (`execution.mode = "live"` **and** `POLYMARKET_PRIVATE_KEY` set). Even then:

- Cap `risk.max_order_size_usdc` to a few dollars and `risk.max_total_exposure_usdc` to ~$10–20 for the first session. The startup check verifies approvals exist; it does not verify the full wire format end-to-end against the live CLOB. A small-size shakedown is the cheapest way to learn if anything still bites.
- Set `polymarket.polygon_rpc_url` to a private provider if you have one — public RPC rate limits will surface as `RPC` errors at startup.
- **Never commit `.env`** or a real key.

---

## Project layout

```
├── config/default.toml             # bot config
├── crates/
│   ├── arb-bot/                    # entrypoint, main loop, wiring
│   ├── arb-config/
│   ├── arb-executor/               # auth, HMAC, EIP-712 signing, place_order
│   ├── arb-inventory/              # asymmetric / hybrid leg tracking
│   ├── arb-monitor/
│   ├── arb-recorder/               # Supabase telemetry sink (additive)
│   ├── arb-risk/
│   ├── arb-scanner/
│   ├── arb-strategy/
│   └── arb-types/
├── dashboard/                      # React + Vite frontend (this PR's work)
│   ├── src/
│   │   ├── components/             # sidebar, ticker, charts, feed, drawer
│   │   ├── hooks/                  # useActivity, useSnapshots (Realtime)
│   │   ├── lib/                    # supabase client, types, aggregations
│   │   └── App.tsx
│   └── README.md                   # dashboard-specific run + layout notes
├── supabase/migrations/            # SQL schema for activity + snapshots tables
├── docs/
│   ├── TERM_PROJECT_PLAN.md
│   └── ASYMMETRIC_MAKER_THESIS.md
├── devlog.md
├── DEMO.md                         # demo runbook + slide queries
├── .env.example
└── README.md
```

---

## Disclaimer

This repo is shared as a learning artifact and reference for anyone curious how a Polymarket-style arbitrage stack fits together. It is **not** investment advice and not a hardened production system. Running it with real funds means accepting that markets, on-chain transactions, exchange APIs, and your own bugs will sometimes lose money — read the relevant modules, size positions tiny, and don't trade with anything you can't afford to lose.
