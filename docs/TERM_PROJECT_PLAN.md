# Term project plan: prediction-market trading stack

**One-line pitch:** Build a Rust system that discovers and acts on pricing inefficiencies in **binary prediction markets**, starting on **Polymarket (on-chain CLOB)** and extending toward **Kalshi (regulated exchange)** for personal, risk-aware trading and research.

**Blockchain angle for your meeting:** Polymarket lives on **Polygon** with tokenized outcome positions and CLOB trading; the project ties together **DeFi-style market structure**, **signing / API auth**, and **execution risk** (partial fills, latency). Kalshi adds a **TradFi vs crypto venue** comparison (APIs, fees, settlement, regulation) even if you do not chain them for arbitrage in v1.

---

## 1. Where the project is today (stage assessment)

### Done or largely in place (research / prototype quality)

| Area | Status |
|------|--------|
| **Market discovery** | Gamma API scan; active binary markets; liquidity filter; periodic refresh. |
| **Market data** | CLOB orderbooks via WebSocket (and REST fallback); combined YES+NO snapshots. |
| **Intramarket strategies** | **Simultaneous:** single-level arb + **multi-level sweep** (`analyze_sweep` / `execute_sweep`). **Asymmetric:** leg-by-leg buying toward a target total cost with **inventory / pairing** (`arb-inventory`). **Hybrid:** try simultaneous first, then asymmetric. |
| **Risk scaffolding** | Per-order caps, exposure, cooldowns, unpaired exposure limits, leg-level checks, sweep evaluation. |
| **Execution structure** | `execute`, `execute_sweep`, `execute_leg` with dry-run paths and typed results (full / partial / error). |

### Not production-ready (blockers for real money)

| Gap | Why it matters |
|-----|----------------|
| **Always dry-run** | `main` always uses `OrderExecutor::new_dry_run`; private key does not enable live mode yet. |
| **CLOB L2 auth** | API key derivation (EIP-712) and **HMAC request signing** are incomplete in `arb-executor` (placeholders / TODO). Without this, live orders will not authenticate. |
| **Balance = not from chain** | Risk uses an internal balance that is not fed by on-chain USDC polling; `update_balance` exists but is not wired to Polygon RPC in the bot loop. |
| **Fill truth** | Assumptions about success vs partial fill should be validated against real API responses and order lifecycle. |
| **Kalshi** | **No code** yet; separate API, auth, and product model. |

**Stage label:** **Late prototype / pre-production** — strong **detection and strategy** surface area on Polymarket; **execution and funding truth** still need hardening before you trade meaningful size.

---

## 2. Term goals (pick a scope you can defend)

### Tier A — Minimum credible outcome (recommended floor)

1. **Polymarket live (small size):** working L2 auth + signed orders + verify fills; optional **paper** mode that logs “would trade” with real book data.
2. **Observability:** structured logs or metrics for opportunities/hour, reject reasons, latency, and post-trade P&amp;L bookkeeping.
3. **Safety:** enforce max loss per day / kill switch in config; keep dry-run default in repo; secrets only via env.

### Tier B — Stretch

4. **Kalshi read path:** authenticated read-only client (orderbook / markets); same **abstract “venue” trait** in Rust so strategies can consume a normalized book.
5. **Kalshi execution (optional):** one simple strategy (e.g. post-only or small FOK experiment) if API and account setup allow.
6. **Cross-venue (only if you have clear matching):** same **economic event** on both venues is rare and **mapping is a project**; treat as research milestone, not a promise.

---

## 3. Suggested term timeline (adjust to your actual term length)

| Phase | Focus | Deliverable |
|-------|--------|-------------|
| **Weeks 1–2** | Lock **Tier A** scope; document risks; run bot **dry-run** 24h logs; list Polymarket CLOB auth steps from official docs. | Short **design note** + sample logs in repo or appendix. |
| **Weeks 3–5** | Implement **API key derive + HMAC**; smallest live test (min size); wire **balance refresh** from RPC or CLOB balance endpoint. | **First live fill** (even $1) with checklist. |
| **Weeks 6–8** | Harden execution: idempotency, partial-fill handling, backoff; backtest or **shadow** compare detection vs hypothetical fills. | **Risk postmortem** doc (what failed / why). |
| **Weeks 9+** | **Kalshi** crate or module: types, auth, market discovery, normalized orderbook; optional execution. | Demo: **two venues** side-by-side or **one slide** on regulatory/tech contrast. |

---

## 4. Outline for your blockchain meeting (slides or talking points)

1. **Problem:** Binary markets should satisfy \(p_{YES} + p_{NO} \approx 1\) (fees aside); transient books violate that.  
2. **Approach:** Intramarket strategies (simultaneous, sweep depth, asymmetric inventory).  
3. **System:** Scanner → monitor → strategy → risk → executor; Rust workspace crates.  
4. **Chain:** Polygon + Polymarket CLOB; wallets, signing, and API credentials.  
5. **Status:** Prototype complete for detection; **execution auth** in progress.  
6. **Safety:** Limits, dry-run default, no keys in git.  
7. **Extension:** Kalshi as second venue (API + economics + regulation).  
8. **Ethics / compliance:** Personal research; not financial advice; respect ToS and jurisdictional rules.

---

## 5. Immediate next steps (resume coding)

1. **Polymarket CLOB auth:** implement derive-api-key flow + request signing per current Polymarket docs; gate `new_authenticated` behind env and tests.  
2. **Balance source:** call `RiskManager::update_balance` from a real balance reader on an interval.  
3. **Config switch:** `execution.mode = "dry_run" | "live"` so a key alone does not imply live trading.  
4. **Kalshi spike:** read Kalshi API docs; sketch `Venue` trait and one `impl` for Polymarket to avoid duplicating strategy code later.

---

## 6. Success criteria for end of term

- You can **demonstrate** either live tiny-size Polymarket execution **or** a rigorous paper-trading pipeline with real market data and reconciled hypothetical P&amp;L.  
- You can **explain** failure modes: stale book, partial leg, fee model, and why intramarket arb is scarce.  
- Optional: **Kalshi** normalized feed or a clear write-up of why cross-venue mapping stopped the scope.

This document is meant to evolve; trim or expand sections to match your course requirements and presentation length.
