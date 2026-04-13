# Polymarket Arbitrage Bot

A Rust bot that detects and executes **binary prediction market arbitrage** on [Polymarket](https://polymarket.com). It scans active markets, watches orderbooks in real time, and when it finds a mispricing (buying both YES and NO costs less than $1), it can place orders to lock in risk-free profit.

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
                                 │              │ (CLOB orders)   │
                                 └──────────────┴─────────────────┘
```

### Components

| Component | Crate | Role |
|-----------|--------|------|
| **Market Scanner** | `arb-scanner` | Fetches active binary markets from Polymarket’s **Gamma API**. Filters by liquidity and keeps a list of markets to monitor. Refreshes periodically (e.g. every 5 minutes). |
| **Price Monitor** | `arb-monitor` | Gets orderbook data for each market. Can use **WebSocket** (live updates) or **REST polling**. Sends combined YES+NO orderbook snapshots (`BinaryOrderBook`) into a channel. |
| **Arbitrage Detector** | `arb-strategy` | Consumes orderbook updates. For each book: checks staleness, takes best YES ask and best NO ask, computes gross spread `1 - (yes_ask + no_ask)`, subtracts fee estimate, and if net spread ≥ `min_net_spread` and size is sufficient, emits an `ArbitrageOpportunity`. |
| **Risk Manager** | `arb-risk` | Before any trade: enforces max order size, max total exposure, minimum reserve balance, per-market cooldown, and max concurrent positions. Can cap size to fit within limits. Records fills to update exposure and cooldowns. |
| **Order Executor** | `arb-executor` | Takes a risk-approved `ExecutionOrder` and places **two buy orders** (YES and NO) on the Polymarket **CLOB**, optionally as Fill-or-Kill (FOK). Handles full fill, partial fill, and errors. |
| **Config** | `arb-config` | Loads settings from `config/default.toml` and env (e.g. `STRATEGY__MIN_NET_SPREAD`). Private key is read from `POLYMARKET_PRIVATE_KEY` only. |
| **Types** | `arb-types` | Shared types: `BinaryMarket`, `BinaryOrderBook`, `ArbitrageOpportunity`, `ExecutionOrder`, `ExecutionResult`, etc. |

### Main Loop (high level)

1. Load config and create scanner, monitor, detector, risk manager, executor.
2. **Initial scan:** fetch active binary markets from Gamma and build a `condition_id → market` lookup.
3. **Background tasks:**
   - **Market refresh:** every `refresh_interval_secs`, re-scan Gamma and update the market list and lookup.
   - **Balance check:** periodically log balance and exposure (on-chain balance query is still a TODO).
   - **Price monitor:** start WebSocket or REST polling; for each market, fetch/stream orderbooks and send `BinaryOrderBook` messages into a channel.
4. **Main loop:** receive `BinaryOrderBook` from the channel. For each book:
   - Look up the market; if not found, skip.
   - Run **arbitrage detection**. If no opportunity, skip.
   - Convert opportunity to an **execution order**.
   - Run **risk evaluation**; if rejected, skip.
   - **Execute** (or dry-run); then have the risk manager **record the result**.

So the bot is **event-driven**: it reacts to orderbook updates and only acts when an opportunity passes both strategy and risk checks.

---

## Configuration

Edit `config/default.toml` (and optionally override with environment variables using `__`, e.g. `STRATEGY__MIN_NET_SPREAD=0.01`).

| Section | Key | Meaning |
|---------|-----|---------|
| **polymarket** | `clob_url`, `gamma_url`, `ws_url` | Polymarket CLOB and Gamma API endpoints. |
| **strategy** | `min_net_spread` | Minimum net spread (after fees) to consider an opportunity (e.g. `0.005` = 0.5%). |
| | `base_fee_rate` | Fee rate used for net spread and fee estimation (e.g. `0.0` or `0.02`). |
| | `max_price_staleness_ms` | Reject orderbooks older than this (ms). |
| | `use_fok_orders` | Use Fill-or-Kill orders when executing. |
| **risk** | `max_order_size_usdc` | Max size per order in USDC. |
| | `max_total_exposure_usdc` | Max total capital in open positions. |
| | `min_reserve_balance_usdc` | Reserve balance not used for new orders. |
| | `per_market_cooldown_secs` | Minimum time between trades in the same market. |
| | `max_concurrent_positions` | Max number of open positions. |
| **scanner** | `refresh_interval_secs` | How often to re-fetch the market list. |
| | `max_markets` | Max number of markets to track. |
| | `min_liquidity_usdc` | Skip markets with less than this liquidity. |
| **monitor** | `use_websocket` | Use WebSocket for orderbooks; if false, use REST polling. |
| | `poll_interval_ms` | Polling interval when not using WebSocket. |

---

## Running the Bot

### Prerequisites

- Rust (e.g. `rustup` with a recent stable toolchain).

### Build and run

```bash
cargo build --release
cargo run --release -p arb-bot
```

The binary is the `arb-bot` crate; it loads `config/default.toml` from the current working directory, so run from the repo root.

### Environment

- Copy `.env.example` to `.env` and set `POLYMARKET_PRIVATE_KEY` if you plan to use live trading later. **Never commit `.env` or your real key.**
- Config overrides: e.g. `STRATEGY__MIN_NET_SPREAD=0.01` `RISK__MAX_ORDER_SIZE_USDC=100`.

### Modes

- **Detect-only / dry-run:** If `POLYMARKET_PRIVATE_KEY` is not set (or API key derivation is not implemented), the bot runs in **dry-run** mode: it discovers opportunities and runs risk checks, but **does not place orders**. It logs what it would have done.
- **Live trading:** Would require deriving CLOB API credentials from the private key (EIP-712 auth) and using `OrderExecutor::new_authenticated`. Currently the code path still uses dry-run until that is implemented.

---

## Project Layout

```
polymarket-arb/
├── config/
│   └── default.toml          # Main config
├── crates/
│   ├── arb-bot/              # Entrypoint, main loop, wiring
│   ├── arb-config/           # Config loading
│   ├── arb-executor/          # CLOB order placement
│   ├── arb-monitor/          # Orderbook (REST + WebSocket)
│   ├── arb-risk/             # Risk checks and state
│   ├── arb-scanner/          # Gamma API market discovery
│   ├── arb-strategy/         # Arbitrage detection + fee math
│   └── arb-types/            # Shared types
├── .env.example
└── README.md
```

---

## Summary

- **What it does:** Finds binary markets on Polymarket where YES ask + NO ask &lt; $1 (after fees) and can execute buys on both sides to lock in profit at resolution.
- **How:** Scanner discovers markets → Monitor streams/polls orderbooks → Detector finds spreads → Risk manager approves or caps size → Executor sends (or dry-runs) YES + NO orders.
- **Current state:** Full pipeline works in **dry-run**; live trading is gated on implementing CLOB API key derivation and HMAC signing in the executor.
