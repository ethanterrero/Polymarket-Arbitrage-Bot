# Asymmetric maker-mode thesis

This is the strategy document for converting the bot's `asymmetric` path from a fire-and-forget GTC taker into a real passive maker strategy. It exists so future-us (and any reviewer) can audit *why* the new code makes the choices it does, separate from *how*.

## Why bother

The simultaneous arb path requires us to beat other arbers to a crossed orderbook. We do not have a latency edge — the CLOB sits behind us-east-1 Cloudflare, our scanner refreshes Gamma every 5 minutes, and our WebSocket bus is one Tokio task on a developer laptop. Pure simultaneous arb is a losing race for us.

Asymmetric (maker) mode flips the latency dependency. We post a passive limit order, wait, and only fill when someone crosses us. We get paid the spread we posted inside of, in exchange for accepting *adverse selection* and *inventory risk*. None of that depends on being fast.

## How the strategy works

A binary market has two outcome tokens (YES and NO). If we own one of each at total cost `c`, we collect $1 at resolution regardless of outcome — risk-free profit of `$1 - c`.

The asymmetric path lets us assemble that pair one leg at a time:
1. Post a passive bid on one side at price `p_a`, well below the current best ask.
2. When it fills, we hold an unpaired leg. We're now exposed to the binary outcome until we close.
3. If the opposite side becomes attractive enough that `p_a + p_b ≤ asymmetric_target_total_cost`, send an IOC taker order on the second leg to lock in the pair.
4. If the opposite side never tightens within `max_unpaired_hold_secs`, unwind the leg at the best opposite bid (Phase 4) and eat the realized loss.

The economic question is whether the average captured spread on closed pairs exceeds the average loss on unwinds plus fees. That ratio is what this thesis exists to defend.

## Fees

Polymarket's CTF Exchange fee rate is per-market and lives at `GET /fee-rate?token_id=<id>`. As of the 2026-04-26 work that wired the live fee fetch, the vast majority of markets report `0` bps; a long tail of markets (especially newer or sponsored ones) carry small positive rates capped around 500 bps in the contract.

Fees are charged on the *output* token, so a maker buy at price `p` for size `s` pays `s * fee_rate_bps / 10000` outcome tokens at settlement. At the 0bps majority case this is free, which is the only reason this strategy is even worth considering — a 50bps fee on both legs would eat most of the realistic capture target.

**Operational rule:** in live mode, refuse to post in a market where the live-fetched `fee_rate_bps > 50` until the strategy has been calibrated on free markets. This guard belongs in scanner-side filtering once the fetcher is hot-path safe (today it's lazy-on-place, so the cheapest enforcement is to skip on first place attempt).

## Adverse selection — the load-bearing risk

When a passive order fills, the counterparty *chose* to cross our quote. Most of the time they crossed because they have information we don't: a news headline, a forecasting model update, a polling release, a resolution-clarifying event. The fill is a signal that the market just moved against us.

The implied edge required to offset adverse selection is roughly:

```
required_post_distance_from_mid > E[move_after_fill | fill] - captured_spread
```

We do not have an information edge. So we have to post far enough below mid that even after an adverse move, we're still positioned to pair the leg at a profit. Concretely: if mid is 0.50 and we post at 0.48, an informed counterparty might cross us only when their belief is at 0.46 — meaning we now hold inventory worth 0.46, while the opposite leg costs 0.54, totaling 1.00 with zero margin. The thesis is that *on average* over many fills, informed counterparties are not so numerous that the median post fills at zero-margin. The looser the market, the less informed the flow, the better this works.

Practically this rules out:
- High-volume markets where informed flow dominates.
- Markets within 24 hours of resolution (information becomes monotonically more precise).
- Markets with a recent large trade (someone just moved, others are likely to follow).

It favors:
- Mid-tier liquidity markets ($1k–$50k 24h volume) where flow is mostly retail.
- Markets in calm phases — no upcoming scheduled event in the next 48 hours.
- Markets with a stable two-sided book (genuine market makers on both sides, indicating they think the price is roughly fair).

## Inventory exit math

A filled unpaired leg is a binary option: it pays $1 if the leg wins, $0 if it loses. Holding it without a pair is the same as taking a directional bet at the fill price. We have no view on direction, so the *time-value* of holding it must come from waiting for the opposite leg to mispriced — that is, betting on volatility or noise in the *opposite* side's quotes, not the underlying probability.

Two regimes:
1. **Quote-noise regime (good for us):** opposite-side quotes drift around an unchanged fair value because of inventory shuffling among other makers. We pair on a noise tick that crosses our target.
2. **Information regime (bad for us):** opposite-side quotes drift consistently away from us because new info has moved the fair value. Our pair never tightens.

The strategy is effectively "short volatility on information arrival, long volatility on quote noise." `max_unpaired_hold_secs` is the time-budget we give regime 1 to play out before assuming we're in regime 2 and cutting.

The expected P&L of a closed pair vs. an unwound leg:

```
E[per leg] = P(pair) * (target_total_cost - $1) - P(unwind) * realized_unwind_loss - fees
```

For this to be positive at typical Polymarket spreads (1–5¢ wide on illiquid markets), we need `P(pair) / P(unwind)` to be roughly 3:1 or better assuming a 1–2¢ unwind loss against a 2¢ captured spread. This is the empirical question the Phase 5 shakedown is designed to answer.

## Recommended config for first live session

These are starting values, not optimized values. Tighten after the shakedown produces data.

| Key | Recommended | Reasoning |
|-----|-------------|-----------|
| `strategy.mode` | `"asymmetric"` | Disable hybrid for now so the data is clean. |
| `strategy.asymmetric_target_total_cost` | `"0.97"` | 3¢ gross capture target — leaves room for unwind loss. |
| `strategy.min_net_spread` | `"0.02"` | Pair-on-fill closer must clear at least 2¢ after fees. |
| `strategy.max_unpaired_hold_secs` | `1800` | 30 min. Long enough for quote noise, short enough to cap regime-2 damage. |
| `strategy.max_unpaired_legs_per_market` | `1` | Single leg per market for the shakedown — concentration risk is the easiest mistake to make. |
| `strategy.asymmetric_repost_interval_secs` | `60` | New in Phase 3. Slow enough that we're not API-spamming, fast enough that stale resting orders get re-pegged. |
| `strategy.unwind_max_loss_usdc` | `"5.0"` | New in Phase 4. Refuse to auto-unwind a single leg for more than $5; escalate to manual. |
| `risk.max_order_size_usdc` | `"5.0"` | First live size cap. Treat the first ten pairings as a tuition payment. |
| `risk.max_total_exposure_usdc` | `"25.0"` | Total session cap. Forces us to read the logs before scaling. |
| `risk.max_unpaired_per_market_usdc` | `"5.0"` | Same as order size — no doubling-down. |
| `scanner.min_24h_volume_usdc` | `"1000.0"` | Phase 5 filter. Anything quieter than this won't fill in 30 min anyway. |
| `scanner.min_secs_to_resolution` | `259200` | 72 hours. Match the adverse-selection rule above. |

## Risk failure modes

Things that will go wrong, in roughly decreasing order of severity:

1. **Resolution-imminent fill.** A market resolves while we hold an unpaired leg on the losing side. We eat the full nominal loss (e.g. $5 on a $5 order). Mitigated by `scanner.min_secs_to_resolution` and by the unwinder's hold-secs timer. Not eliminated — a market can be moved to early resolution.
2. **One-sided news.** A confirmed event spikes one side to 0.99 while ours sits at 0.45. The unwinder takes a near-total loss. Mitigated by `unwind_max_loss_usdc` aborting auto-unwind and alerting the operator. The operator then has to decide whether to manually close at a deeper loss or hold to resolution.
3. **CLOB downtime / partial outage.** Resting orders may sit longer than we want, or cancels may fail. The poller has to handle "order not found" gracefully (treat as cancelled, drop from map) and the cancel path needs retry logic. Phase 1's poller is the first defense.
4. **Stale fee-rate cache.** If a market's fee rate is updated by Polymarket between our cache and our placement, our EIP-712 digest is wrong and the order is silently rejected. The fee cache today never invalidates. Acceptable for v0; revisit if we see CLOB rejections in logs.
5. **Approval revocation mid-session.** The startup allowance check (2026-04-26) only fires at boot. If the user revokes an approval while the bot is running, places start failing on settlement. Acceptable for v0 — the bot logs the rejection loudly enough.
6. **Resting-order drift below post limit.** We post 10 resting orders, market makers improve our prices, we end up with 10 stale quotes nobody is filling. Mitigated by Phase 3's repost loop.

## What this thesis is *not* claiming

It is not claiming Polymarket has a structural maker edge that funds a sustainable strategy. It is claiming there is plausibly enough quote noise in mid-tier markets to capture small consistent profit, *if* we can avoid adverse selection by careful market selection. That hypothesis has to be tested with real money at small size, not derived from first principles.

The size caps in the recommended config exist because the first live session is data collection. The decision point — keep going, retune, or kill the strategy — comes after we have at least 20–30 closed pairs to look at.

## What's next after Phase 0

Phase 1 wires the resting-order tracking that everything else depends on. The decisions in this doc set the configuration defaults Phase 1 will need to accept, but Phase 1 itself is plumbing.
