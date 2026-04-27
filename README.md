# Polymarket Arbitrage Bot

A Rust bot that detects and executes **binary prediction market arbitrage** on [Polymarket](https://polymarket.com). It scans active markets, watches orderbooks in real time, and when it finds a mispricing (buying both YES and NO costs less than $1), it can place orders to lock in risk-free profit.

For day-to-day engineering notes and the ordered backlog, see **[devlog.md](devlog.md)**.

---

## How It Works

### The Arbitrage Idea

In a binary market, one of two outcomes happens: **YES** or **NO**. You can buy YES tokens and NO tokens. When the market resolves:

- If YES wins: each YES token pays **$1**, each NO token pays **$0**
- If NO wins: each NO token pays **$1**, each YES token pays **$0**

So if you hold **one YES** and **one NO**, you get **$1** no matter what. If you can buy that pair for **less than $1**, you profit when the market resolves.

**Arbitrage condition:** `price(YES) + price(NO) < 1.0` (after fees)

The bot looks for exactly that: best ask on YES + best ask on NO &lt; $1, with enough size on both sides and after accounting for fees.

---

## Architecture

The bot is a Rust workspace with several crates that work together:

```
┌─────────────────┐     ┌─────────────────┐     ┌─────────────────┐
│  MarketScanner  │────▶│  PriceMonitor   │────▶│ ArbitrageDetector│
│  (Gamma API)    │     │  (CLOB REST/WS) │     │  (strategy)      │
└─────────────────┘     └────────┬────────┘     └────────┬─────────┘
                                 │                       │
                                 │    BinaryOrderBook    │ opportunity
                                 │                       ▼
                                 │              ┌─────────────────┐
                                 │              │  RiskManager    │
                                 │              │  (limits, etc.) │
                                 │              └────────┬────────┘
                                 │                       │ approved order
                                 │                       ▼
                                 │              ┌─────────────────┐
                                 │              │ OrderExecutor   │
                                 │              │ (CLOB + auth)   │
                                 └──────────────┴─────────────────┘
```

**Asymmetric mode** also uses **InventoryManager** (`arb-inventory`) to track unpaired legs, pairing, and locked profit; the main loop can run **simultaneous**, **asymmetric**, or **hybrid** strategy (see `strategy.mode` in config).

### Components

| Component | Crate | Role |
|-----------|--------|------|
| **Market Scanner** | `arb-scanner` | Fetches active binary markets from Polymarket’s **Gamma API**. Filters by liquidity and keeps a list of markets to monitor. Refreshes periodically (e.g. every 5 minutes). |
| **Price Monitor** | `arb-monitor` | Gets orderbook data for each market. Can use **WebSocket** (live updates) or **REST polling**. Sends combined YES+NO orderbook snapshots (`BinaryOrderBook`) into a channel. Can **fetch balance** from the CLOB for risk checks. |
| **Arbitrage Detector** | `arb-strategy` | Consumes orderbook updates. Single-level arb, optional **multi-level sweep** (`analyze_sweep`), and asymmetric leg logic. Checks staleness, fees, and `min_net_spread`. |
| **Risk Manager** | `arb-risk` | Before any trade: max order size, exposure, reserve balance, per-market cooldown, max concurrent positions, unpaired limits for asymmetric flow. Records execution results. |
| **Inventory Manager** | `arb-inventory` | Tracks open legs and paired positions for asymmetric / hybrid modes; stale-leg warnings in the bot loop. |
| **Order Executor** | `arb-executor` | **L2 auth:** derive API key (`POST /auth/derive-api-key` with EIP-712) and **HMAC** `POLY_*` headers per request. Places YES + NO buys (FOK/GTC). **Order payload:** see *Execution status* below. |
| **Config** | `arb-config` | Loads `config/default.toml` and env (e.g. `STRATEGY__MIN_NET_SPREAD`). Private key from `POLYMARKET_PRIVATE_KEY` only. |
| **Types** | `arb-types` | Shared types: `BinaryMarket`, `BinaryOrderBook`, `StrategyMode`, opportunities, execution orders, etc. |

### Main Loop (high level)

1. Load config; build scanner, monitor, detector, risk manager, inventory (for asymmetric/hybrid), executor.
2. **Executor mode:** if `POLYMARKET_PRIVATE_KEY` is set, the bot calls `OrderExecutor::new_live` (derive API key + HMAC). On missing key or derivation failure, it uses **dry-run** (`new_dry_run`) and logs a warning.
3. **Initial scan:** fetch binary markets from Gamma and build `condition_id → market` lookup.
4. **Background tasks:**
   - **Market refresh:** re-scan Gamma on `scanner.refresh_interval_secs` and refresh the lookup.
   - **Balance:** periodically `fetch_balance` from the monitor and `RiskManager::update_balance` (then log balance vs exposure).
   - **Stale legs:** in asymmetric-style flows, periodic logging of stale unpaired legs (inventory).
   - **Price monitor:** WebSocket or REST; push `BinaryOrderBook` into a channel.
5. **Main loop:** for each book, resolve market → detect (simultaneous sweep and/or single-level, or asymmetric/hybrid per `strategy.mode`) → risk → spawn execution → `record_execution`.

The bot is **event-driven**: it reacts to orderbook updates and only acts when an opportunity passes strategy and risk checks.

### Execution status (important)

- **Dry-run:** Safe default. No key, or failed API-key derivation → logs what would be traded; no signed CLOB orders.
- **Authenticated HTTP:** With a valid key, the executor attaches **L2 HMAC** headers and posts to `/order`.
- **Signed order body:** Polymarket’s CLOB expects an EIP-712–signed **Order** struct on the wire. The full signed payload is wired into `place_order` (`order_signing.rs` reference-tested against Polymarket's vectors; `proxy_address.rs` derives the funded `maker` for both proxy and Gnosis Safe accounts; BUY amount quantization is pinned against `py-clob-client`'s `get_order_amounts`).
- **Per-market fee rate:** `Order.feeRateBps` is fetched lazily from `GET /fee-rate?token_id=<id>` and cached per token, matching `py-clob-client`'s `get_fee_rate_bps` (the value goes into the EIP-712 digest, so a wrong default would cause silent rejections).
- **Startup allowance check:** Live mode verifies USDC.e and ConditionalTokens approvals to both CTF Exchange contracts (standard + neg-risk) on Polygon before enabling order placement. Missing approvals fail the bot at boot rather than at first attempted fill.

---

## Configuration

Edit `config/default.toml` (and optionally override with environment variables using `__`, e.g. `STRATEGY__MIN_NET_SPREAD=0.01`).

| Section | Key | Meaning |
|---------|-----|---------|
| **polymarket** | `clob_url`, `gamma_url`, `ws_url`, `chain_id` | CLOB, Gamma, WebSocket endpoints and chain ID. |
| | `polygon_rpc_url` | Polygon JSON-RPC URL for the startup allowance check (default `https://polygon-rpc.com`; override with your own provider if rate-limited). |
| **strategy** | `mode` | `simultaneous`, `asymmetric`, or `hybrid`. |
| | `min_net_spread` | Minimum net spread (after fees) to trade. |
| | `base_fee_rate` | Fee rate for spread math. |
| | `max_price_staleness_ms` | Reject stale orderbooks (ms). |
| | `use_fok_orders` | Fill-or-kill vs GTC when executing. |
| | `max_sweep_levels` | Depth levels for sweep analysis (0 disables sweep-first path). |
| | `min_sweep_profit_usdc`, `asymmetric_target_total_cost`, `max_unpaired_hold_secs`, `max_unpaired_exposure_usdc`, `max_unpaired_legs_per_market` | Sweep and asymmetric tuning. |
| **risk** | `max_order_size_usdc`, `max_total_exposure_usdc`, `min_reserve_balance_usdc`, `per_market_cooldown_secs`, `max_concurrent_positions` | Risk limits. |
| | `max_unpaired_exposure_usdc`, `max_unpaired_per_market_usdc` | Extra limits for unpaired inventory. |
| **scanner** | `refresh_interval_secs`, `max_markets`, `min_liquidity_usdc` | Discovery cadence and filters. |
| **monitor** | `use_websocket`, `poll_interval_ms`, `max_concurrent_requests` | Orderbook transport and concurrency. |

| **execution** | `mode` | `"dry_run"` (default) or `"live"` (explicit opt-in). |

---

## Running the Bot

### Prerequisites

- Rust (e.g. `rustup` with a recent stable toolchain).

### Build and run

```bash
cargo build --release
cargo run --release -p arb-bot
```

The `arb-bot` binary loads `config/default.toml` from the **current working directory** — run from the repo root.

### Environment

- Copy `.env.example` to `.env`. Set `POLYMARKET_PRIVATE_KEY` only if you intend authenticated requests toward the CLOB. **Never commit `.env` or a real key.**
- Config overrides: e.g. `STRATEGY__MIN_NET_SPREAD=0.01` `RISK__MAX_ORDER_SIZE_USDC=100`.

### Modes

- **Dry-run:** `execution.mode="dry_run"` → `OrderExecutor::new_dry_run`. Discovers opportunities, runs risk, logs trades; **does not** send real orders.
- **Live (explicit opt-in):** `execution.mode="live"` and `POLYMARKET_PRIVATE_KEY` set → `OrderExecutor::new_live`, HMAC-authenticated `POST /order` with a full EIP-712-signed Order body, per-market fee rate fetched from the CLOB, and a Polygon RPC allowance check at startup. **Recommended:** cap `risk.max_order_size_usdc` to a few dollars for the first live session as a small-size shakedown.

---

## Project Layout

```
├── config/
│   └── default.toml
├── crates/
│   ├── arb-bot/          # Entrypoint, main loop, wiring
│   ├── arb-config/
│   ├── arb-executor/     # Auth, HMAC, order signing (crypto), place_order
│   ├── arb-inventory/    # Asymmetric / hybrid leg tracking
│   ├── arb-monitor/
│   ├── arb-risk/
│   ├── arb-scanner/
│   ├── arb-strategy/
│   └── arb-types/
├── docs/                 # e.g. term project plan
├── devlog.md             # Progress + ordered next steps
├── .env.example
└── README.md
```

---

## Summary

- **What it does:** Finds binary markets where YES ask + NO ask &lt; $1 (after fees) and can execute (or dry-run) buys on both sides, including sweep and asymmetric paths.
- **How:** Scanner → monitor → strategy → risk → executor; optional inventory for asymmetric/hybrid.
- **Current state:** Full **detection and risk** pipeline; **CLOB L2 auth** (derive key + HMAC) implemented; **balance** fed from the monitor into risk on an interval; **EIP-712 order signing** wired into `place_order` with the full signed-Order payload, BUY amount quantization pinned to `py-clob-client`, per-market fee rate fetched and cached, and a startup **allowance check** that fails the bot at boot if USDC.e or ConditionalTokens approvals are missing on Polygon. See **[devlog.md](devlog.md)** for the per-PR rationale.
