use arb_config::StrategyConfig;
use arb_types::{
    ArbitrageOpportunity, BinaryMarket, BinaryOrderBook, ExecutionOrder, InventorySnapshot,
    LegOrder, PriceLevel, Side, SweepExecutionOrder, SweepOpportunity, SweepPlan,
};
use chrono::Utc;
use rust_decimal::Decimal;
use tracing::info;
use uuid::Uuid;

#[derive(Debug, thiserror::Error)]
pub enum StrategyError {
    #[error("Orderbook is stale ({age_ms}ms > {max_ms}ms)")]
    StaleOrderbook { age_ms: i64, max_ms: u64 },
    #[error("No asks available on {side} side")]
    NoAsks { side: String },
    #[error("Spread below threshold: net={net_spread} < min={min_spread}")]
    BelowThreshold {
        net_spread: Decimal,
        min_spread: Decimal,
    },
    #[error("Sweep profit below threshold: profit={profit} < min={min_profit}")]
    SweepBelowThreshold { profit: Decimal, min_profit: Decimal },
    #[error("No profitable leg found for asymmetric strategy")]
    NoAsymmetricOpportunity,
}

/// Detects arbitrage opportunities in binary prediction markets.
pub struct ArbitrageDetector {
    min_net_spread: Decimal,
    base_fee_rate: Decimal,
    max_staleness_ms: u64,
    use_fok: bool,
    max_sweep_levels: usize,
    min_sweep_profit: Decimal,
    asymmetric_target: Decimal,
}

impl ArbitrageDetector {
    pub fn new(config: &StrategyConfig) -> Self {
        Self {
            min_net_spread: config.min_net_spread,
            base_fee_rate: config.base_fee_rate,
            max_staleness_ms: config.max_price_staleness_ms,
            use_fok: config.use_fok_orders,
            max_sweep_levels: config.max_sweep_levels,
            min_sweep_profit: config.min_sweep_profit_usdc,
            asymmetric_target: config.asymmetric_target_total_cost,
        }
    }

    /// Analyze a binary orderbook for arbitrage opportunities.
    ///
    /// Returns Some(opportunity) if buying both YES and NO costs less than $1
    /// after fees, guaranteeing profit on resolution.
    pub fn analyze(
        &self,
        book: &BinaryOrderBook,
        market: &BinaryMarket,
    ) -> Result<ArbitrageOpportunity, StrategyError> {
        // Staleness check.
        let age_ms = book.age_ms();
        if age_ms > self.max_staleness_ms as i64 {
            return Err(StrategyError::StaleOrderbook {
                age_ms,
                max_ms: self.max_staleness_ms,
            });
        }

        // Get best asks on both sides.
        let yes_ask = book
            .best_yes_ask()
            .ok_or_else(|| StrategyError::NoAsks {
                side: "YES".to_string(),
            })?;
        let no_ask = book
            .best_no_ask()
            .ok_or_else(|| StrategyError::NoAsks {
                side: "NO".to_string(),
            })?;

        let one = Decimal::ONE;

        // Gross spread: positive means buying both sides costs less than $1.
        let gross_spread = one - (yes_ask.price + no_ask.price);

        // Quick-reject if gross spread is not positive.
        if gross_spread <= Decimal::ZERO {
            return Err(StrategyError::BelowThreshold {
                net_spread: gross_spread,
                min_spread: self.min_net_spread,
            });
        }

        // Estimate fees using the fast-path formula.
        let fee_estimate = self.estimate_fees(yes_ask.price, no_ask.price);
        let net_spread = gross_spread - fee_estimate;

        if net_spread < self.min_net_spread {
            return Err(StrategyError::BelowThreshold {
                net_spread,
                min_spread: self.min_net_spread,
            });
        }

        // Maximum size is limited by the smaller side.
        let max_size = yes_ask.size.min(no_ask.size);

        if max_size.is_zero() {
            return Err(StrategyError::NoAsks {
                side: "both (zero size)".to_string(),
            });
        }

        let expected_profit = net_spread * max_size;

        let opportunity = ArbitrageOpportunity {
            id: Uuid::new_v4(),
            market: market.clone(),
            yes_ask_price: yes_ask.price,
            no_ask_price: no_ask.price,
            yes_ask_size: yes_ask.size,
            no_ask_size: no_ask.size,
            gross_spread,
            net_spread,
            max_size,
            expected_profit,
            detected_at: Utc::now(),
        };

        info!(
            condition_id = %market.condition_id,
            question = %market.question,
            yes_ask = %yes_ask.price,
            no_ask = %no_ask.price,
            gross_spread = %gross_spread,
            net_spread = %net_spread,
            max_size = %max_size,
            expected_profit = %expected_profit,
            "Arbitrage opportunity detected"
        );

        Ok(opportunity)
    }

    /// Fast-path fee estimate for detection.
    ///
    /// fee = base_fee_rate * (min(p_yes, 1-p_yes) + min(p_no, 1-p_no))
    fn estimate_fees(&self, p_yes: Decimal, p_no: Decimal) -> Decimal {
        if self.base_fee_rate.is_zero() {
            return Decimal::ZERO;
        }

        let one = Decimal::ONE;
        let yes_fee_component = p_yes.min(one - p_yes);
        let no_fee_component = p_no.min(one - p_no);

        self.base_fee_rate * (yes_fee_component + no_fee_component)
    }

    /// Precise fee calculation for execution.
    ///
    /// For each side:
    ///   fee_factor = base_fee_rate * min(p, 1-p) / p
    ///   effective_cost = p / (1 - fee_factor)
    /// total_cost = effective_cost_yes + effective_cost_no
    /// profit_per_pair = 1.0 - total_cost
    pub fn precise_profit_per_pair(&self, p_yes: Decimal, p_no: Decimal) -> Decimal {
        let one = Decimal::ONE;

        if self.base_fee_rate.is_zero() {
            return one - p_yes - p_no;
        }

        let effective_yes = effective_cost(p_yes, self.base_fee_rate);
        let effective_no = effective_cost(p_no, self.base_fee_rate);

        one - effective_yes - effective_no
    }

    /// Convert an opportunity into an execution order (before risk approval).
    pub fn to_execution_order(&self, opportunity: ArbitrageOpportunity) -> ExecutionOrder {
        ExecutionOrder {
            yes_price: opportunity.yes_ask_price,
            no_price: opportunity.no_ask_price,
            approved_size: opportunity.max_size,
            use_fok: self.use_fok,
            opportunity,
        }
    }

    /// Convert a sweep opportunity into a sweep execution order.
    pub fn to_sweep_execution_order(
        &self,
        opportunity: SweepOpportunity,
    ) -> SweepExecutionOrder {
        SweepExecutionOrder {
            approved_size: opportunity.matched_size,
            yes_limit_price: opportunity.yes_sweep.worst_price,
            no_limit_price: opportunity.no_sweep.worst_price,
            use_fok: self.use_fok,
            opportunity,
        }
    }

    /// Analyze a binary orderbook for multi-level sweep opportunities.
    ///
    /// Walks up to `max_sweep_levels` on each side, building cumulative cost/size arrays,
    /// then finds the combination of YES levels x NO levels that maximizes total profit.
    pub fn analyze_sweep(
        &self,
        book: &BinaryOrderBook,
        market: &BinaryMarket,
    ) -> Result<SweepOpportunity, StrategyError> {
        let age_ms = book.age_ms();
        if age_ms > self.max_staleness_ms as i64 {
            return Err(StrategyError::StaleOrderbook {
                age_ms,
                max_ms: self.max_staleness_ms,
            });
        }

        if book.yes_book.asks.is_empty() {
            return Err(StrategyError::NoAsks {
                side: "YES".to_string(),
            });
        }
        if book.no_book.asks.is_empty() {
            return Err(StrategyError::NoAsks {
                side: "NO".to_string(),
            });
        }

        let max_levels = if self.max_sweep_levels == 0 {
            1
        } else {
            self.max_sweep_levels
        };

        // Build cumulative arrays: (levels_consumed, total_size, total_cost, worst_price)
        let yes_cum = build_cumulative(&book.yes_book.asks, max_levels);
        let no_cum = build_cumulative(&book.no_book.asks, max_levels);

        let one = Decimal::ONE;
        let mut best_profit = Decimal::ZERO;
        let mut best_yi = 0usize;
        let mut best_ni = 0usize;

        // Search all combinations for max profit.
        for yi in 0..yes_cum.len() {
            for ni in 0..no_cum.len() {
                let matched_size = yes_cum[yi].total_size.min(no_cum[ni].total_size);
                if matched_size.is_zero() {
                    continue;
                }

                // Interpolate VWAPs at matched_size.
                let yes_vwap = interpolate_vwap(&book.yes_book.asks, matched_size);
                let no_vwap = interpolate_vwap(&book.no_book.asks, matched_size);

                let fee_estimate = self.estimate_fees(yes_vwap, no_vwap);
                let net_spread = one - yes_vwap - no_vwap - fee_estimate;
                let profit = net_spread * matched_size;

                if profit > best_profit {
                    best_profit = profit;
                    best_yi = yi;
                    best_ni = ni;
                }
            }
        }

        if best_profit < self.min_sweep_profit {
            return Err(StrategyError::SweepBelowThreshold {
                profit: best_profit,
                min_profit: self.min_sweep_profit,
            });
        }

        let matched_size = yes_cum[best_yi]
            .total_size
            .min(no_cum[best_ni].total_size);
        let yes_vwap = interpolate_vwap(&book.yes_book.asks, matched_size);
        let no_vwap = interpolate_vwap(&book.no_book.asks, matched_size);
        let fee_estimate = self.estimate_fees(yes_vwap, no_vwap);
        let net_spread = one - yes_vwap - no_vwap - fee_estimate;

        let yes_sweep = build_sweep_plan(&book.yes_book.asks, best_yi + 1);
        let no_sweep = build_sweep_plan(&book.no_book.asks, best_ni + 1);

        let opportunity = SweepOpportunity {
            id: Uuid::new_v4(),
            market: market.clone(),
            yes_sweep,
            no_sweep,
            matched_size,
            net_spread,
            expected_profit: best_profit,
            detected_at: Utc::now(),
        };

        info!(
            condition_id = %market.condition_id,
            matched_size = %matched_size,
            net_spread = %net_spread,
            expected_profit = %best_profit,
            yes_levels = best_yi + 1,
            no_levels = best_ni + 1,
            "Sweep opportunity detected"
        );

        Ok(opportunity)
    }

    /// Analyze the book for asymmetric (passive maker) buy opportunities.
    ///
    /// For each side, computes the target price that would still pair to a
    /// profitable closed position given the opposite-side best ask (or
    /// existing inventory cost). Then chooses a passive post price *inside*
    /// the current bid-ask spread — specifically `best_bid + min_tick_size`,
    /// capped at the target — so we get paid the spread when we eventually
    /// fill, instead of crossing at the ask like a taker.
    ///
    /// Returns `LegOrder`s with `use_fok = false` so the executor will post
    /// them as GTC and they'll rest on the book. Phase 1's resting-order
    /// tracker picks them up from there.
    pub fn analyze_asymmetric(
        &self,
        book: &BinaryOrderBook,
        market: &BinaryMarket,
        inventory: &InventorySnapshot,
    ) -> Result<Vec<LegOrder>, StrategyError> {
        let age_ms = book.age_ms();
        if age_ms > self.max_staleness_ms as i64 {
            return Err(StrategyError::StaleOrderbook {
                age_ms,
                max_ms: self.max_staleness_ms,
            });
        }

        let fee_overhead = self.base_fee_rate * Decimal::new(2, 0) * Decimal::new(5, 1);
        let open_legs = inventory.open_legs.get(&market.condition_id);
        let tick = market.min_tick_size;

        let mut orders = Vec::new();

        for side in [Side::Yes, Side::No] {
            let target = compute_leg_target(
                side,
                self.asymmetric_target,
                fee_overhead,
                book,
                open_legs,
            );
            if target <= Decimal::ZERO {
                continue;
            }
            let (best_bid, best_ask, token_id) = match side {
                Side::Yes => (
                    book.best_yes_bid(),
                    book.best_yes_ask(),
                    &market.yes_token_id,
                ),
                Side::No => (
                    book.best_no_bid(),
                    book.best_no_ask(),
                    &market.no_token_id,
                ),
            };
            let Some(best_bid) = best_bid else { continue };

            // Compute the post price. `improved_bid = best_bid + tick`
            // queue-jumps in front of resting bids; cap at `target` so we
            // never post above the profitability threshold. Snap the target
            // floor down to tick granularity so the price is always valid.
            let target_floor = snap_to_tick(target, tick);
            let improved_bid = best_bid.price + tick;
            if improved_bid > target_floor {
                // Posting at a valid tick that improves the bid would push us
                // past target — skip the leg, queue improvement isn't worth
                // the loss in spread capture.
                continue;
            }
            // Also never post at or above the best ask (we'd just become a
            // taker; the simultaneous path already covers that).
            if let Some(ask) = best_ask {
                if improved_bid >= ask.price {
                    continue;
                }
            }
            let post_price = improved_bid;

            // Choose size: bounded by what's already resting at the bid (we
            // don't want to be the only resting order on a single-bidder
            // book) plus a fixed sizing baseline so it's never zero. The
            // risk layer caps this further; this is just a sane upper bound.
            let post_size = best_bid.size;
            if post_size <= Decimal::ZERO {
                continue;
            }

            orders.push(LegOrder {
                condition_id: market.condition_id.clone(),
                token_id: token_id.clone(),
                side,
                size: post_size,
                target_price: post_price,
                use_fok: false,
                neg_risk: market.neg_risk,
                fee_rate_bps: market.fee_rate_bps,
                min_tick_size: tick,
                direction: arb_types::TradeDirection::Buy,
            });
        }

        if orders.is_empty() {
            return Err(StrategyError::NoAsymmetricOpportunity);
        }

        for order in &orders {
            info!(
                condition_id = %market.condition_id,
                side = %order.side,
                post_price = %order.target_price,
                size = %order.size,
                "Asymmetric maker quote prepared"
            );
        }

        Ok(orders)
    }
}

/// Round `price` down to the nearest multiple of `tick`. Used to align a
/// computed target price onto the CLOB's valid tick grid before posting.
fn snap_to_tick(price: Decimal, tick: Decimal) -> Decimal {
    if tick <= Decimal::ZERO {
        return price;
    }
    let n = (price / tick).floor();
    n * tick
}

/// Calculate the effective cost to receive 1 token at price p, including fees.
///
/// fee_factor = base_fee_rate * min(p, 1-p) / p
/// effective_cost = p / (1 - fee_factor)
fn effective_cost(price: Decimal, base_fee_rate: Decimal) -> Decimal {
    let one = Decimal::ONE;
    let fee_factor = base_fee_rate * price.min(one - price) / price;
    price / (one - fee_factor)
}

/// Cumulative entry at a given depth of the book.
struct CumulativeLevel {
    total_size: Decimal,
    total_cost: Decimal,
}

/// Build cumulative size/cost arrays from ask levels.
fn build_cumulative(asks: &[PriceLevel], max_levels: usize) -> Vec<CumulativeLevel> {
    let mut result = Vec::new();
    let mut cum_size = Decimal::ZERO;
    let mut cum_cost = Decimal::ZERO;

    for (i, level) in asks.iter().enumerate() {
        if i >= max_levels {
            break;
        }
        cum_size += level.size;
        cum_cost += level.price * level.size;
        result.push(CumulativeLevel {
            total_size: cum_size,
            total_cost: cum_cost,
        });
    }
    result
}

/// Compute the VWAP for buying `target_size` tokens walking the ask book.
fn interpolate_vwap(asks: &[PriceLevel], target_size: Decimal) -> Decimal {
    let mut remaining = target_size;
    let mut total_cost = Decimal::ZERO;

    for level in asks {
        if remaining.is_zero() {
            break;
        }
        let fill = remaining.min(level.size);
        total_cost += fill * level.price;
        remaining -= fill;
    }

    if target_size.is_zero() {
        return Decimal::ZERO;
    }
    total_cost / target_size
}

/// Build a `SweepPlan` from the first `n_levels` ask levels.
fn build_sweep_plan(asks: &[PriceLevel], n_levels: usize) -> SweepPlan {
    let levels: Vec<PriceLevel> = asks.iter().take(n_levels).cloned().collect();
    let total_size: Decimal = levels.iter().map(|l| l.size).sum();
    let total_cost: Decimal = levels.iter().map(|l| l.price * l.size).sum();
    let vwap = if total_size.is_zero() {
        Decimal::ZERO
    } else {
        total_cost / total_size
    };
    let worst_price = levels.iter().map(|l| l.price).max().unwrap_or(Decimal::ZERO);

    SweepPlan {
        levels,
        total_size,
        vwap,
        worst_price,
    }
}

/// Compute the target price for one leg of an asymmetric strategy.
fn compute_leg_target(
    side: Side,
    target_total_cost: Decimal,
    fee_overhead: Decimal,
    book: &BinaryOrderBook,
    open_legs: Option<&Vec<arb_types::OpenLeg>>,
) -> Decimal {
    // Check if we have unpaired legs on the opposite side.
    let opposite_side = match side {
        Side::Yes => Side::No,
        Side::No => Side::Yes,
    };

    // If we hold the opposite side, use its avg_cost (sunk cost) for a tighter target.
    if let Some(legs) = open_legs {
        let opposite_legs: Vec<&arb_types::OpenLeg> =
            legs.iter().filter(|l| l.side == opposite_side).collect();
        if !opposite_legs.is_empty() {
            // Weighted average cost of opposite-side inventory.
            let total_size: Decimal = opposite_legs.iter().map(|l| l.size).sum();
            let total_cost: Decimal = opposite_legs.iter().map(|l| l.avg_cost * l.size).sum();
            let avg_cost = total_cost / total_size;
            return target_total_cost - avg_cost - fee_overhead;
        }
    }

    // No opposite inventory — use best ask on the other side as estimate.
    let opposite_ask = match side {
        Side::Yes => book.best_no_ask(),
        Side::No => book.best_yes_ask(),
    };

    match opposite_ask {
        Some(ask) => target_total_cost - ask.price - fee_overhead,
        None => Decimal::ZERO, // Can't compute target without the other side.
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use arb_types::{PriceLevel, Side, TokenOrderBook};
    use rust_decimal_macros::dec;

    fn make_config(min_spread: Decimal, fee_rate: Decimal) -> StrategyConfig {
        StrategyConfig {
            min_net_spread: min_spread,
            base_fee_rate: fee_rate,
            max_price_staleness_ms: 60_000, // generous for tests
            use_fok_orders: true,
            mode: Default::default(),
            max_sweep_levels: 5,
            min_sweep_profit_usdc: dec!(0.50),
            asymmetric_target_total_cost: dec!(0.97),
            max_unpaired_hold_secs: 3600,
            max_unpaired_exposure_usdc: dec!(200),
            max_unpaired_legs_per_market: 3,
            unwind_max_loss_usdc: dec!(5),
        }
    }

    fn make_market() -> BinaryMarket {
        BinaryMarket {
            condition_id: "test-condition".to_string(),
            yes_token_id: "yes-token".to_string(),
            no_token_id: "no-token".to_string(),
            question: "Will it rain?".to_string(),
            slug: "will-it-rain".to_string(),
            active: true,
            liquidity: Some(dec!(1000)),
            neg_risk: false,
            fee_rate_bps: 0,
            min_tick_size: dec!(0.01),
        }
    }

    fn make_book(yes_ask: Decimal, yes_size: Decimal, no_ask: Decimal, no_size: Decimal) -> BinaryOrderBook {
        BinaryOrderBook {
            condition_id: "test-condition".to_string(),
            yes_book: TokenOrderBook {
                token_id: "yes-token".to_string(),
                side: Side::Yes,
                asks: vec![PriceLevel {
                    price: yes_ask,
                    size: yes_size,
                }],
                bids: vec![],
            },
            no_book: TokenOrderBook {
                token_id: "no-token".to_string(),
                side: Side::No,
                asks: vec![PriceLevel {
                    price: no_ask,
                    size: no_size,
                }],
                bids: vec![],
            },
            timestamp: Utc::now(),
        }
    }

    #[test]
    fn test_detects_opportunity_zero_fees() {
        let config = make_config(dec!(0.001), dec!(0.0));
        let detector = ArbitrageDetector::new(&config);
        let market = make_market();

        // YES @ 0.45, NO @ 0.50 → total 0.95, spread 0.05
        let book = make_book(dec!(0.45), dec!(100), dec!(0.50), dec!(80));
        let result = detector.analyze(&book, &market);

        assert!(result.is_ok());
        let opp = result.unwrap();
        assert_eq!(opp.gross_spread, dec!(0.05));
        assert_eq!(opp.net_spread, dec!(0.05));
        assert_eq!(opp.max_size, dec!(80)); // min(100, 80)
        assert_eq!(opp.expected_profit, dec!(4.0)); // 0.05 * 80
    }

    #[test]
    fn test_rejects_no_spread() {
        let config = make_config(dec!(0.001), dec!(0.0));
        let detector = ArbitrageDetector::new(&config);
        let market = make_market();

        // YES @ 0.55, NO @ 0.50 → total 1.05, spread -0.05
        let book = make_book(dec!(0.55), dec!(100), dec!(0.50), dec!(100));
        let result = detector.analyze(&book, &market);

        assert!(result.is_err());
    }

    #[test]
    fn test_rejects_below_threshold() {
        let config = make_config(dec!(0.01), dec!(0.0));
        let detector = ArbitrageDetector::new(&config);
        let market = make_market();

        // YES @ 0.495, NO @ 0.500 → spread 0.005, below 0.01 threshold
        let book = make_book(dec!(0.495), dec!(100), dec!(0.500), dec!(100));
        let result = detector.analyze(&book, &market);

        assert!(result.is_err());
    }

    #[test]
    fn test_fee_estimation() {
        let config = make_config(dec!(0.001), dec!(0.02));
        let detector = ArbitrageDetector::new(&config);

        // p_yes=0.4 → min(0.4, 0.6) = 0.4
        // p_no=0.5  → min(0.5, 0.5) = 0.5
        // fee = 0.02 * (0.4 + 0.5) = 0.018
        let fee = detector.estimate_fees(dec!(0.4), dec!(0.5));
        assert_eq!(fee, dec!(0.018));
    }

    #[test]
    fn test_precise_profit_zero_fees() {
        let config = make_config(dec!(0.001), dec!(0.0));
        let detector = ArbitrageDetector::new(&config);

        let profit = detector.precise_profit_per_pair(dec!(0.45), dec!(0.50));
        assert_eq!(profit, dec!(0.05));
    }

    #[test]
    fn test_precise_profit_with_fees() {
        let config = make_config(dec!(0.001), dec!(0.02));
        let detector = ArbitrageDetector::new(&config);

        let profit = detector.precise_profit_per_pair(dec!(0.45), dec!(0.50));
        // With 2% base fee rate, profit should be less than 0.05
        assert!(profit < dec!(0.05));
        assert!(profit > Decimal::ZERO);
    }

    #[test]
    fn test_size_limited_by_smaller_side() {
        let config = make_config(dec!(0.001), dec!(0.0));
        let detector = ArbitrageDetector::new(&config);
        let market = make_market();

        // YES has 200 available, NO has 30 → max_size = 30
        let book = make_book(dec!(0.40), dec!(200), dec!(0.50), dec!(30));
        let opp = detector.analyze(&book, &market).unwrap();
        assert_eq!(opp.max_size, dec!(30));
    }

    // ─── Phase 2: asymmetric (maker) pricing ──────────────────────────────

    /// Build a binary book where each side has explicit best bid + ask. The
    /// strategy's maker pricing depends on the bid being present, so a
    /// fixture that only sets asks (like `make_book`) won't exercise the
    /// maker code path.
    fn make_book_with_bids(
        yes_bid: Decimal,
        yes_bid_size: Decimal,
        yes_ask: Decimal,
        yes_ask_size: Decimal,
        no_bid: Decimal,
        no_bid_size: Decimal,
        no_ask: Decimal,
        no_ask_size: Decimal,
    ) -> BinaryOrderBook {
        BinaryOrderBook {
            condition_id: "test-condition".to_string(),
            yes_book: TokenOrderBook {
                token_id: "yes-token".to_string(),
                side: Side::Yes,
                asks: vec![PriceLevel {
                    price: yes_ask,
                    size: yes_ask_size,
                }],
                bids: vec![PriceLevel {
                    price: yes_bid,
                    size: yes_bid_size,
                }],
            },
            no_book: TokenOrderBook {
                token_id: "no-token".to_string(),
                side: Side::No,
                asks: vec![PriceLevel {
                    price: no_ask,
                    size: no_ask_size,
                }],
                bids: vec![PriceLevel {
                    price: no_bid,
                    size: no_bid_size,
                }],
            },
            timestamp: Utc::now(),
        }
    }

    fn empty_inventory() -> InventorySnapshot {
        InventorySnapshot::default()
    }

    #[test]
    fn asymmetric_posts_inside_spread_at_improved_bid() {
        // target_total_cost = 0.97, no_ask = 0.50, fee_overhead = 0.
        // yes_target = 0.97 - 0.50 = 0.47 (snapped to 0.47 at tick=0.01).
        // yes_bid = 0.40, improved_bid = 0.41 ≤ 0.47 → post at 0.41.
        let config = make_config(dec!(0.001), dec!(0.0));
        let detector = ArbitrageDetector::new(&config);
        let market = make_market();
        let book = make_book_with_bids(
            dec!(0.40), dec!(50), dec!(0.55), dec!(20),
            dec!(0.40), dec!(50), dec!(0.50), dec!(20),
        );

        let orders = detector
            .analyze_asymmetric(&book, &market, &empty_inventory())
            .unwrap();
        let yes = orders.iter().find(|o| o.side == Side::Yes).unwrap();
        assert_eq!(yes.target_price, dec!(0.41));
        assert!(!yes.use_fok, "asymmetric leg orders must be GTC (resting)");
        assert_eq!(yes.size, dec!(50)); // bounded by best_bid size
    }

    #[test]
    fn asymmetric_skips_when_improved_bid_exceeds_target() {
        // yes_target = 0.97 - no_ask = 0.97 - 0.45 = 0.52.
        // yes_bid = 0.52 → improved_bid = 0.53 > 0.52 → skip yes side.
        // no_target = 0.97 - yes_ask = 0.97 - 0.55 = 0.42.
        // no_bid = 0.30 → improved_bid = 0.31 ≤ 0.42 and < no_ask (0.45) → keep.
        let config = make_config(dec!(0.001), dec!(0.0));
        let detector = ArbitrageDetector::new(&config);
        let market = make_market();
        let book = make_book_with_bids(
            dec!(0.52), dec!(50), dec!(0.55), dec!(20),
            dec!(0.30), dec!(50), dec!(0.45), dec!(20),
        );
        let orders = detector
            .analyze_asymmetric(&book, &market, &empty_inventory())
            .unwrap();
        assert!(!orders.iter().any(|o| o.side == Side::Yes));
        let no = orders.iter().find(|o| o.side == Side::No).unwrap();
        assert_eq!(no.target_price, dec!(0.31));
    }

    #[test]
    fn asymmetric_skips_when_improved_bid_would_cross_ask() {
        // Tight book: best_yes_bid = 0.40, best_yes_ask = 0.41, tick = 0.01.
        // improved_bid = 0.41 = best_ask → would be a taker, skip.
        let config = make_config(dec!(0.001), dec!(0.0));
        let detector = ArbitrageDetector::new(&config);
        let market = make_market();
        let book = make_book_with_bids(
            dec!(0.40), dec!(50), dec!(0.41), dec!(20),
            dec!(0.40), dec!(50), dec!(0.50), dec!(20),
        );
        let orders = detector
            .analyze_asymmetric(&book, &market, &empty_inventory())
            .unwrap();
        assert!(!orders.iter().any(|o| o.side == Side::Yes));
    }

    #[test]
    fn asymmetric_target_snaps_down_to_tick() {
        // Non-tick-aligned target should never push us above the tick floor.
        // Setting no_ask = 0.523 → yes_target = 0.97 - 0.523 = 0.447,
        // floored to 0.44 at tick = 0.01.
        // yes_bid = 0.43, improved_bid = 0.44 ≤ 0.44 → post at 0.44.
        let config = make_config(dec!(0.001), dec!(0.0));
        let detector = ArbitrageDetector::new(&config);
        let market = make_market();
        let book = make_book_with_bids(
            dec!(0.43), dec!(50), dec!(0.55), dec!(20),
            dec!(0.50), dec!(50), dec!(0.523), dec!(20),
        );
        let orders = detector
            .analyze_asymmetric(&book, &market, &empty_inventory())
            .unwrap();
        let yes = orders.iter().find(|o| o.side == Side::Yes).unwrap();
        assert_eq!(yes.target_price, dec!(0.44));
    }

    #[test]
    fn asymmetric_skips_side_without_bid() {
        // No bid present on yes book → no maker quote possible for yes side.
        let config = make_config(dec!(0.001), dec!(0.0));
        let detector = ArbitrageDetector::new(&config);
        let market = make_market();
        let mut book = make_book_with_bids(
            dec!(0.40), dec!(50), dec!(0.55), dec!(20),
            dec!(0.40), dec!(50), dec!(0.50), dec!(20),
        );
        book.yes_book.bids.clear();
        let orders = detector
            .analyze_asymmetric(&book, &market, &empty_inventory())
            .unwrap();
        assert!(!orders.iter().any(|o| o.side == Side::Yes));
        assert!(orders.iter().any(|o| o.side == Side::No));
    }

    #[test]
    fn snap_to_tick_floors_down() {
        assert_eq!(snap_to_tick(dec!(0.523), dec!(0.01)), dec!(0.52));
        assert_eq!(snap_to_tick(dec!(0.50), dec!(0.01)), dec!(0.50));
        assert_eq!(snap_to_tick(dec!(0.4999), dec!(0.001)), dec!(0.499));
        // Negative tick → identity (defensive, shouldn't happen in practice).
        assert_eq!(snap_to_tick(dec!(0.5), dec!(0)), dec!(0.5));
    }
}
