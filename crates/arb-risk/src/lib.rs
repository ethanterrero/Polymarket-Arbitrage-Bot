use arb_config::RiskConfig;
use arb_types::{
    ExecutionOrder, ExecutionResult, LegExecutionResult, LegOrder, SweepExecutionOrder,
    SweepOpportunity,
};
use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use std::collections::HashMap;
use tokio::sync::RwLock;
use tracing::{debug, info, warn};

#[derive(Debug, thiserror::Error)]
pub enum RiskError {
    #[error("Order size {requested} exceeds max {max}")]
    OrderSizeTooLarge { requested: Decimal, max: Decimal },
    #[error("Total exposure would be {projected} USDC, exceeds max {max}")]
    ExposureLimitExceeded { projected: Decimal, max: Decimal },
    #[error("Insufficient balance: available={available}, needed={needed}, reserve={reserve}")]
    InsufficientBalance {
        available: Decimal,
        needed: Decimal,
        reserve: Decimal,
    },
    #[error("Market {condition_id} is in cooldown (remaining: {remaining_secs}s)")]
    MarketCooldown {
        condition_id: String,
        remaining_secs: i64,
    },
    #[error("Max concurrent positions reached: {current}/{max}")]
    TooManyPositions { current: usize, max: usize },
    #[error("Unpaired exposure would be {projected} USDC, exceeds max {max}")]
    UnpairedExposureLimitExceeded { projected: Decimal, max: Decimal },
    #[error("Per-market unpaired exposure would be {projected} USDC, exceeds max {max} for {condition_id}")]
    PerMarketUnpairedLimitExceeded {
        condition_id: String,
        projected: Decimal,
        max: Decimal,
    },
}

/// Tracks risk state for the bot.
#[derive(Debug)]
struct RiskState {
    /// Current USDC balance (updated periodically from chain).
    balance: Decimal,
    /// Total USDC deployed across all open positions.
    total_exposure: Decimal,
    /// Number of currently open positions.
    open_positions: usize,
    /// Per-market cooldown tracking: condition_id → last trade time.
    last_trade_time: HashMap<String, DateTime<Utc>>,
    /// Total USDC in unpaired (single-leg) positions.
    unpaired_exposure: Decimal,
    /// Per-market unpaired exposure: condition_id → USDC.
    per_market_unpaired: HashMap<String, Decimal>,
}

/// Evaluates and enforces risk limits before order execution.
pub struct RiskManager {
    config: RiskConfig,
    state: RwLock<RiskState>,
}

impl RiskManager {
    pub fn new(config: &RiskConfig) -> Self {
        Self {
            config: config.clone(),
            state: RwLock::new(RiskState {
                balance: Decimal::ZERO,
                total_exposure: Decimal::ZERO,
                open_positions: 0,
                last_trade_time: HashMap::new(),
                unpaired_exposure: Decimal::ZERO,
                per_market_unpaired: HashMap::new(),
            }),
        }
    }

    /// Update the current USDC balance (called periodically).
    pub async fn update_balance(&self, balance: Decimal) {
        let mut state = self.state.write().await;
        state.balance = balance;
        info!(balance = %balance, "Balance updated");
    }

    /// Get the current balance.
    pub async fn balance(&self) -> Decimal {
        self.state.read().await.balance
    }

    /// Get current exposure.
    pub async fn total_exposure(&self) -> Decimal {
        self.state.read().await.total_exposure
    }

    /// Get the current number of open positions.
    pub async fn open_positions(&self) -> usize {
        self.state.read().await.open_positions
    }

    /// Evaluate an opportunity against risk limits.
    /// Returns an ExecutionOrder with the approved size, or an error.
    pub async fn evaluate(
        &self,
        mut order: ExecutionOrder,
    ) -> Result<ExecutionOrder, RiskError> {
        let state = self.state.read().await;
        let now = Utc::now();

        // Check concurrent positions limit.
        if state.open_positions >= self.config.max_concurrent_positions {
            return Err(RiskError::TooManyPositions {
                current: state.open_positions,
                max: self.config.max_concurrent_positions,
            });
        }

        // Check per-market cooldown.
        let condition_id = &order.opportunity.market.condition_id;
        if let Some(last_trade) = state.last_trade_time.get(condition_id) {
            let elapsed = (now - *last_trade).num_seconds();
            let cooldown = self.config.per_market_cooldown_secs as i64;
            if elapsed < cooldown {
                return Err(RiskError::MarketCooldown {
                    condition_id: condition_id.clone(),
                    remaining_secs: cooldown - elapsed,
                });
            }
        }

        // Cap order size to max_order_size.
        let mut approved_size = order.approved_size;
        if approved_size > self.config.max_order_size_usdc {
            debug!(
                requested = %approved_size,
                max = %self.config.max_order_size_usdc,
                "Capping order size to max"
            );
            approved_size = self.config.max_order_size_usdc;
        }

        // Check total exposure.
        let order_cost = (order.yes_price + order.no_price) * approved_size;
        let projected_exposure = state.total_exposure + order_cost;
        if projected_exposure > self.config.max_total_exposure_usdc {
            // Try to reduce size to fit.
            let available_exposure = self.config.max_total_exposure_usdc - state.total_exposure;
            if available_exposure <= Decimal::ZERO {
                return Err(RiskError::ExposureLimitExceeded {
                    projected: projected_exposure,
                    max: self.config.max_total_exposure_usdc,
                });
            }
            let cost_per_unit = order.yes_price + order.no_price;
            approved_size = approved_size.min(available_exposure / cost_per_unit);
        }

        // Check balance (with reserve).
        let order_cost = (order.yes_price + order.no_price) * approved_size;
        let available = state.balance - self.config.min_reserve_balance_usdc;
        if order_cost > available {
            if available <= Decimal::ZERO {
                return Err(RiskError::InsufficientBalance {
                    available: state.balance,
                    needed: order_cost,
                    reserve: self.config.min_reserve_balance_usdc,
                });
            }
            let cost_per_unit = order.yes_price + order.no_price;
            approved_size = approved_size.min(available / cost_per_unit);
        }

        // Final zero-size check.
        if approved_size <= Decimal::ZERO {
            return Err(RiskError::InsufficientBalance {
                available: state.balance,
                needed: order_cost,
                reserve: self.config.min_reserve_balance_usdc,
            });
        }

        order.approved_size = approved_size;

        debug!(
            condition_id = %order.opportunity.market.condition_id,
            approved_size = %approved_size,
            "Risk check passed"
        );

        Ok(order)
    }

    /// Record the result of an execution, updating risk state.
    pub async fn record_execution(&self, result: &ExecutionResult) {
        let mut state = self.state.write().await;

        match result {
            ExecutionResult::FullFill {
                order, total_cost, ..
            } => {
                state.total_exposure += total_cost;
                state.open_positions += 1;
                state
                    .last_trade_time
                    .insert(order.opportunity.market.condition_id.clone(), Utc::now());
                info!(
                    condition_id = %order.opportunity.market.condition_id,
                    total_cost = %total_cost,
                    total_exposure = %state.total_exposure,
                    open_positions = state.open_positions,
                    "Recorded full fill"
                );
            }
            ExecutionResult::PartialFill {
                order, total_cost, ..
            } => {
                state.total_exposure += total_cost;
                state.open_positions += 1;
                state
                    .last_trade_time
                    .insert(order.opportunity.market.condition_id.clone(), Utc::now());
                warn!(
                    condition_id = %order.opportunity.market.condition_id,
                    total_cost = %total_cost,
                    "Recorded partial fill — naked position risk"
                );
            }
            ExecutionResult::NoFill { order, reason } => {
                debug!(
                    condition_id = %order.opportunity.market.condition_id,
                    reason = %reason,
                    "No fill recorded"
                );
            }
            ExecutionResult::Error { order, error } => {
                warn!(
                    condition_id = %order.opportunity.market.condition_id,
                    error = %error,
                    "Execution error recorded"
                );
            }
            ExecutionResult::DryRun { order } => {
                debug!(
                    condition_id = %order.opportunity.market.condition_id,
                    "Dry run recorded (no state change)"
                );
            }
        }
    }

    /// Evaluate a multi-level sweep order against risk limits.
    pub async fn evaluate_sweep(
        &self,
        mut order: SweepExecutionOrder,
    ) -> Result<SweepExecutionOrder, RiskError> {
        let state = self.state.read().await;
        let now = Utc::now();

        if state.open_positions >= self.config.max_concurrent_positions {
            return Err(RiskError::TooManyPositions {
                current: state.open_positions,
                max: self.config.max_concurrent_positions,
            });
        }

        let condition_id = &order.opportunity.market.condition_id;
        if let Some(last_trade) = state.last_trade_time.get(condition_id) {
            let elapsed = (now - *last_trade).num_seconds();
            let cooldown = self.config.per_market_cooldown_secs as i64;
            if elapsed < cooldown {
                return Err(RiskError::MarketCooldown {
                    condition_id: condition_id.clone(),
                    remaining_secs: cooldown - elapsed,
                });
            }
        }

        let mut approved_size = order.approved_size;
        if approved_size > self.config.max_order_size_usdc {
            approved_size = self.config.max_order_size_usdc;
        }

        let cost_per_unit = order.yes_limit_price + order.no_limit_price;
        let order_cost = cost_per_unit * approved_size;
        let projected_exposure = state.total_exposure + order_cost;
        if projected_exposure > self.config.max_total_exposure_usdc {
            let available_exposure = self.config.max_total_exposure_usdc - state.total_exposure;
            if available_exposure <= Decimal::ZERO {
                return Err(RiskError::ExposureLimitExceeded {
                    projected: projected_exposure,
                    max: self.config.max_total_exposure_usdc,
                });
            }
            approved_size = approved_size.min(available_exposure / cost_per_unit);
        }

        let order_cost = cost_per_unit * approved_size;
        let available = state.balance - self.config.min_reserve_balance_usdc;
        if order_cost > available {
            if available <= Decimal::ZERO {
                return Err(RiskError::InsufficientBalance {
                    available: state.balance,
                    needed: order_cost,
                    reserve: self.config.min_reserve_balance_usdc,
                });
            }
            approved_size = approved_size.min(available / cost_per_unit);
        }

        if approved_size <= Decimal::ZERO {
            return Err(RiskError::InsufficientBalance {
                available: state.balance,
                needed: order_cost,
                reserve: self.config.min_reserve_balance_usdc,
            });
        }

        order.approved_size = approved_size;
        Ok(order)
    }

    /// Evaluate a single-leg buy against risk limits including unpaired exposure.
    pub async fn evaluate_leg(&self, order: &LegOrder) -> Result<Decimal, RiskError> {
        let state = self.state.read().await;
        let now = Utc::now();

        if state.open_positions >= self.config.max_concurrent_positions {
            return Err(RiskError::TooManyPositions {
                current: state.open_positions,
                max: self.config.max_concurrent_positions,
            });
        }

        // Check per-market cooldown.
        if let Some(last_trade) = state.last_trade_time.get(&order.condition_id) {
            let elapsed = (now - *last_trade).num_seconds();
            let cooldown = self.config.per_market_cooldown_secs as i64;
            if elapsed < cooldown {
                return Err(RiskError::MarketCooldown {
                    condition_id: order.condition_id.clone(),
                    remaining_secs: cooldown - elapsed,
                });
            }
        }

        let mut approved_size = order.size;
        if approved_size > self.config.max_order_size_usdc {
            approved_size = self.config.max_order_size_usdc;
        }

        let leg_cost = order.target_price * approved_size;

        // Check total unpaired exposure.
        let projected_unpaired = state.unpaired_exposure + leg_cost;
        if projected_unpaired > self.config.max_unpaired_exposure_usdc {
            let available = self.config.max_unpaired_exposure_usdc - state.unpaired_exposure;
            if available <= Decimal::ZERO {
                return Err(RiskError::UnpairedExposureLimitExceeded {
                    projected: projected_unpaired,
                    max: self.config.max_unpaired_exposure_usdc,
                });
            }
            approved_size = approved_size.min(available / order.target_price);
        }

        // Check per-market unpaired exposure.
        let current_market_unpaired = state
            .per_market_unpaired
            .get(&order.condition_id)
            .copied()
            .unwrap_or(Decimal::ZERO);
        let projected_market = current_market_unpaired + order.target_price * approved_size;
        if projected_market > self.config.max_unpaired_per_market_usdc {
            let available = self.config.max_unpaired_per_market_usdc - current_market_unpaired;
            if available <= Decimal::ZERO {
                return Err(RiskError::PerMarketUnpairedLimitExceeded {
                    condition_id: order.condition_id.clone(),
                    projected: projected_market,
                    max: self.config.max_unpaired_per_market_usdc,
                });
            }
            approved_size = approved_size.min(available / order.target_price);
        }

        // Check total exposure.
        let projected_total = state.total_exposure + order.target_price * approved_size;
        if projected_total > self.config.max_total_exposure_usdc {
            let available = self.config.max_total_exposure_usdc - state.total_exposure;
            if available <= Decimal::ZERO {
                return Err(RiskError::ExposureLimitExceeded {
                    projected: projected_total,
                    max: self.config.max_total_exposure_usdc,
                });
            }
            approved_size = approved_size.min(available / order.target_price);
        }

        // Check balance.
        let available_balance = state.balance - self.config.min_reserve_balance_usdc;
        let leg_cost = order.target_price * approved_size;
        if leg_cost > available_balance {
            if available_balance <= Decimal::ZERO {
                return Err(RiskError::InsufficientBalance {
                    available: state.balance,
                    needed: leg_cost,
                    reserve: self.config.min_reserve_balance_usdc,
                });
            }
            approved_size = approved_size.min(available_balance / order.target_price);
        }

        if approved_size <= Decimal::ZERO {
            return Err(RiskError::InsufficientBalance {
                available: state.balance,
                needed: order.target_price * order.size,
                reserve: self.config.min_reserve_balance_usdc,
            });
        }

        debug!(
            condition_id = %order.condition_id,
            side = %order.side,
            approved_size = %approved_size,
            "Leg risk check passed"
        );

        Ok(approved_size)
    }

    /// Record the result of a single-leg execution.
    pub async fn record_leg_execution(&self, result: &LegExecutionResult) {
        let mut state = self.state.write().await;

        match result {
            LegExecutionResult::Filled {
                order, fill_cost, ..
            } => {
                state.total_exposure += fill_cost;
                state.unpaired_exposure += fill_cost;
                let entry = state
                    .per_market_unpaired
                    .entry(order.condition_id.clone())
                    .or_insert(Decimal::ZERO);
                *entry += fill_cost;
                state
                    .last_trade_time
                    .insert(order.condition_id.clone(), Utc::now());
                info!(
                    condition_id = %order.condition_id,
                    side = %order.side,
                    fill_cost = %fill_cost,
                    unpaired_exposure = %state.unpaired_exposure,
                    "Recorded leg fill"
                );
            }
            LegExecutionResult::Resting { order, order_id, .. } => {
                debug!(
                    condition_id = %order.condition_id,
                    side = %order.side,
                    order_id = %order_id,
                    "Leg resting on the book"
                );
            }
            LegExecutionResult::NoFill { order, reason } => {
                debug!(
                    condition_id = %order.condition_id,
                    reason = %reason,
                    "Leg no fill recorded"
                );
            }
            LegExecutionResult::Error { order, error } => {
                warn!(
                    condition_id = %order.condition_id,
                    error = %error,
                    "Leg execution error recorded"
                );
            }
            LegExecutionResult::DryRun { order } => {
                debug!(
                    condition_id = %order.condition_id,
                    "Leg dry run recorded"
                );
            }
        }
    }

    /// Reduce unpaired exposure when legs are paired by inventory manager.
    pub async fn record_pairing(&self, condition_id: &str, paired_cost: Decimal) {
        let mut state = self.state.write().await;
        state.unpaired_exposure = (state.unpaired_exposure - paired_cost).max(Decimal::ZERO);
        if let Some(entry) = state.per_market_unpaired.get_mut(condition_id) {
            *entry = (*entry - paired_cost).max(Decimal::ZERO);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use arb_types::{ArbitrageOpportunity, BinaryMarket};
    use rust_decimal_macros::dec;
    use uuid::Uuid;

    fn make_config() -> RiskConfig {
        RiskConfig {
            max_order_size_usdc: dec!(50),
            max_total_exposure_usdc: dec!(500),
            min_reserve_balance_usdc: dec!(50),
            per_market_cooldown_secs: 10,
            max_concurrent_positions: 10,
            max_unpaired_exposure_usdc: dec!(200),
            max_unpaired_per_market_usdc: dec!(50),
        }
    }

    fn make_order(size: Decimal, yes_price: Decimal, no_price: Decimal) -> ExecutionOrder {
        ExecutionOrder {
            opportunity: ArbitrageOpportunity {
                id: Uuid::new_v4(),
                market: BinaryMarket {
                    condition_id: "test".to_string(),
                    yes_token_id: "yes".to_string(),
                    no_token_id: "no".to_string(),
                    question: "Test?".to_string(),
                    slug: "test".to_string(),
                    active: true,
                    liquidity: Some(dec!(1000)),
                    neg_risk: false,
                    fee_rate_bps: 0,
                    min_tick_size: dec!(0.01),
                    volume_24h: None,
                    end_date: None,
                },
                yes_ask_price: yes_price,
                no_ask_price: no_price,
                yes_ask_size: dec!(100),
                no_ask_size: dec!(100),
                gross_spread: dec!(0.05),
                net_spread: dec!(0.05),
                max_size: size,
                expected_profit: dec!(0.05) * size,
                detected_at: Utc::now(),
            },
            approved_size: size,
            yes_price,
            no_price,
            use_fok: true,
        }
    }

    #[tokio::test]
    async fn test_passes_within_limits() {
        let rm = RiskManager::new(&make_config());
        rm.update_balance(dec!(200)).await;

        let order = make_order(dec!(30), dec!(0.45), dec!(0.50));
        let result = rm.evaluate(order).await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap().approved_size, dec!(30));
    }

    #[tokio::test]
    async fn test_caps_to_max_order_size() {
        let rm = RiskManager::new(&make_config());
        rm.update_balance(dec!(1000)).await;

        let order = make_order(dec!(100), dec!(0.45), dec!(0.50));
        let result = rm.evaluate(order).await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap().approved_size, dec!(50));
    }

    #[tokio::test]
    async fn test_rejects_insufficient_balance() {
        let rm = RiskManager::new(&make_config());
        rm.update_balance(dec!(50)).await; // exactly at reserve, no available

        let order = make_order(dec!(10), dec!(0.45), dec!(0.50));
        let result = rm.evaluate(order).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_cooldown_enforced() {
        let rm = RiskManager::new(&make_config());
        rm.update_balance(dec!(500)).await;

        // Simulate a previous trade.
        {
            let mut state = rm.state.write().await;
            state
                .last_trade_time
                .insert("test".to_string(), Utc::now());
        }

        let order = make_order(dec!(10), dec!(0.45), dec!(0.50));
        let result = rm.evaluate(order).await;
        assert!(matches!(result, Err(RiskError::MarketCooldown { .. })));
    }

    #[tokio::test]
    async fn test_max_positions_enforced() {
        let rm = RiskManager::new(&make_config());
        rm.update_balance(dec!(1000)).await;

        {
            let mut state = rm.state.write().await;
            state.open_positions = 10;
        }

        let order = make_order(dec!(10), dec!(0.45), dec!(0.50));
        let result = rm.evaluate(order).await;
        assert!(matches!(result, Err(RiskError::TooManyPositions { .. })));
    }
}
