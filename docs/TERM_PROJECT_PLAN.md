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

### Largely resolved since this plan was first written

| Was a blocker | Status (as of 2026-06-01) |
|----------------|---------------------------|
| ~~Always dry-run~~ | **Wired.** `execution.mode = "live"` + `POLYMARKET_PRIVATE_KEY` opts into `OrderExecutor::new_live`; default still dry-run. |
| ~~CLOB L2 auth~~ | **Wired.** EIP-712 API-key derivation + HMAC request signing on every `/order` POST, pinned against Polymarket's reference vectors. |
| ~~Balance = not from chain~~ | **Wired.** Monitor fetches balance from the CLOB on a 30 s loop; `RiskManager::update_balance` is called from that loop in `arb-bot/main.rs`. |
| ~~No visibility into the bot~~ | **Wired.** `arb-recorder` ships activity + snapshots to Supabase; a React dashboard renders the live feed (see `dashboard/`). |

### Still not production-ready (blockers for real money)

| Gap | Why it matters |
|-----|----------------|
| **CLOB V2 migration** | Polymarket's new CLOB drops `taker`/`expiration`/`nonce`/`feeRateBps` and adds `timestamp`/`metadata`/`builder`; domain version `"1"` → `"2"`. Work is on `feat/clob-v2-migration` but not on `main` — live mode today targets V1, which Polymarket will retire. |
| **Fill truth not validated under load** | Partial-fill, race-on-cancel, and post-resting-edge cases have unit tests but no live-fire shakedown. The startup allowance check verifies approvals exist; it does not verify the wire format end-to-end against the live CLOB. |
| **Kalshi** | **No code** yet; separate API, auth, and product model. |

**Stage label:** **Pre-production on Polymarket V1, late-prototype on V2.** Detection and strategy are solid; the V1 execution path is wire-format-correct on paper and waiting on a small-size live shakedown. V2 needs to land before V1 is retired, or live trading rots.

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

The original items 1–3 (CLOB L2 auth, balance from chain, dry-run/live config switch) are all done — see the "Largely resolved" table above. The current shortlist:

1. **First small-size live fill on Polymarket V1.** Cap `risk.max_order_size_usdc` to a few dollars and `risk.max_total_exposure_usdc` to ~$10–20. The startup allowance check verifies approvals exist; only an actual live POST verifies the full wire format against the production CLOB.
2. **Bot-to-dashboard smoke test.** Drop `SUPABASE_SERVICE_KEY` into the root `.env`, set `[telemetry] enabled = true`, run dry-run, confirm rows land in `activity` / `snapshots` and on the dashboard. Snapshot loop fires every 30 s so this doesn't depend on a real arb showing up.
3. **CLOB V2 migration.** Land `feat/clob-v2-migration` so live mode keeps working when Polymarket retires V1: drop `taker`/`expiration`/`nonce`/`feeRateBps`, add `timestamp`/`metadata`/`builder`, bump EIP-712 domain version `"1"` → `"2"`.
4. **Phase 4 dashboard polish.** Seed / replay script so the screen is never empty during a live demo; sample-data toggle in the dashboard for offline use; Supabase WebSocket reconnect handling for mid-presentation drops.
5. **Kalshi spike.** Read Kalshi API docs; sketch a `Venue` trait and one impl for Polymarket so strategies can consume a normalized book without duplicate code paths.

---

## 6. Success criteria for end of term

- You can **demonstrate** either live tiny-size Polymarket execution **or** a rigorous paper-trading pipeline with real market data and reconciled hypothetical P&amp;L.  
- You can **explain** failure modes: stale book, partial leg, fee model, and why intramarket arb is scarce.  
- Optional: **Kalshi** normalized feed or a clear write-up of why cross-venue mapping stopped the scope.

This document is meant to evolve; trim or expand sections to match your course requirements and presentation length.
