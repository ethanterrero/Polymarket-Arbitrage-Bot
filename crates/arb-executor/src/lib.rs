use arb_config::AppConfig;
use arb_types::{ExecutionOrder, ExecutionResult, LegExecutionResult, LegOrder, SweepExecutionOrder};
use reqwest::header::{HeaderMap, HeaderValue, CONTENT_TYPE};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use tracing::{debug, error, info, warn};

#[derive(Debug, thiserror::Error)]
pub enum ExecutorError {
    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),
    #[error("API error: {status} — {body}")]
    Api { status: u16, body: String },
    #[error("Authentication not configured: {0}")]
    AuthNotConfigured(String),
    #[error("Order rejected: {0}")]
    OrderRejected(String),
}

/// Represents the order type sent to the CLOB.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum OrderType {
    Fok,
    Gtc,
}

/// Order request body for the CLOB API.
#[derive(Debug, Serialize)]
struct ClobOrderRequest {
    token_id: String,
    price: String,
    size: String,
    side: String,
    order_type: OrderType,
}

/// Response from the CLOB order endpoint.
#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct ClobOrderResponse {
    #[serde(default)]
    success: bool,
    #[serde(default)]
    order_id: Option<String>,
    #[serde(default, rename = "errorMsg")]
    error_msg: Option<String>,
    #[serde(default)]
    status: Option<String>,
}

/// Executes arbitrage trades against the Polymarket CLOB.
///
/// In dry-run mode, logs opportunities without placing orders.
pub struct OrderExecutor {
    client: reqwest::Client,
    clob_url: String,
    dry_run: bool,
    /// API credentials (derived from private key via EIP-712 signing).
    /// None = dry-run only.
    api_key: Option<String>,
    api_secret: Option<String>,
    api_passphrase: Option<String>,
}

impl OrderExecutor {
    /// Create an executor in dry-run mode (no orders placed).
    pub fn new_dry_run(config: &AppConfig) -> Self {
        Self {
            client: reqwest::Client::new(),
            clob_url: config.polymarket.clob_url.clone(),
            dry_run: true,
            api_key: None,
            api_secret: None,
            api_passphrase: None,
        }
    }

    /// Create an executor with API credentials for live trading.
    ///
    /// In production, derive these from the private key using the CLOB's
    /// `POST /auth/derive-api-key` endpoint with EIP-712 signing.
    pub fn new_authenticated(
        config: &AppConfig,
        api_key: String,
        api_secret: String,
        api_passphrase: String,
    ) -> Self {
        Self {
            client: reqwest::Client::new(),
            clob_url: config.polymarket.clob_url.clone(),
            dry_run: false,
            api_key: Some(api_key),
            api_secret: Some(api_secret),
            api_passphrase: Some(api_passphrase),
        }
    }

    /// Execute an arbitrage order (buy YES + buy NO concurrently).
    pub async fn execute(&self, order: ExecutionOrder) -> ExecutionResult {
        if self.dry_run {
            info!(
                condition_id = %order.opportunity.market.condition_id,
                question = %order.opportunity.market.question,
                yes_price = %order.yes_price,
                no_price = %order.no_price,
                size = %order.approved_size,
                expected_profit = %order.opportunity.expected_profit,
                "[DRY RUN] Would execute arbitrage"
            );
            return ExecutionResult::DryRun { order };
        }

        // Verify authentication is available.
        if self.api_key.is_none() {
            return ExecutionResult::Error {
                error: "API credentials not configured".to_string(),
                order,
            };
        }

        info!(
            condition_id = %order.opportunity.market.condition_id,
            yes_price = %order.yes_price,
            no_price = %order.no_price,
            size = %order.approved_size,
            "Executing arbitrage order"
        );

        let order_type = if order.use_fok {
            OrderType::Fok
        } else {
            OrderType::Gtc
        };

        // Place both legs concurrently.
        let yes_future = self.place_order(
            &order.opportunity.market.yes_token_id,
            order.yes_price,
            order.approved_size,
            &order_type,
        );

        let no_future = self.place_order(
            &order.opportunity.market.no_token_id,
            order.no_price,
            order.approved_size,
            &order_type,
        );

        let (yes_result, no_result) = tokio::join!(yes_future, no_future);

        // Process results.
        match (yes_result, no_result) {
            (Ok(yes_resp), Ok(no_resp)) => {
                if yes_resp.success && no_resp.success {
                    let total_cost =
                        (order.yes_price + order.no_price) * order.approved_size;
                    ExecutionResult::FullFill {
                        yes_fill_size: order.approved_size,
                        no_fill_size: order.approved_size,
                        total_cost,
                        order,
                    }
                } else if yes_resp.success || no_resp.success {
                    // One leg filled, the other didn't — naked position risk!
                    let (yes_fill, no_fill) = if yes_resp.success {
                        (order.approved_size, Decimal::ZERO)
                    } else {
                        (Decimal::ZERO, order.approved_size)
                    };
                    let filled_price = if yes_resp.success {
                        order.yes_price
                    } else {
                        order.no_price
                    };
                    let total_cost = filled_price * order.approved_size;

                    let failed_side = if yes_resp.success { "NO" } else { "YES" };
                    let failed_msg = if yes_resp.success {
                        no_resp.error_msg.unwrap_or_default()
                    } else {
                        yes_resp.error_msg.unwrap_or_default()
                    };

                    warn!(
                        condition_id = %order.opportunity.market.condition_id,
                        failed_side,
                        failed_msg = %failed_msg,
                        "PARTIAL FILL — naked position created"
                    );

                    ExecutionResult::PartialFill {
                        yes_fill_size: yes_fill,
                        no_fill_size: no_fill,
                        total_cost,
                        detail: format!("{} leg failed: {}", failed_side, failed_msg),
                        order,
                    }
                } else {
                    let reason = format!(
                        "YES: {}, NO: {}",
                        yes_resp.error_msg.unwrap_or_else(|| "unknown".to_string()),
                        no_resp.error_msg.unwrap_or_else(|| "unknown".to_string()),
                    );
                    ExecutionResult::NoFill { order, reason }
                }
            }
            (Err(e), Ok(no_resp)) => {
                if no_resp.success {
                    warn!(
                        "YES leg HTTP error but NO leg filled — naked position!"
                    );
                    ExecutionResult::PartialFill {
                        yes_fill_size: Decimal::ZERO,
                        no_fill_size: order.approved_size,
                        total_cost: order.no_price * order.approved_size,
                        detail: format!("YES leg HTTP error: {}", e),
                        order,
                    }
                } else {
                    ExecutionResult::Error {
                        error: format!("YES leg error: {}", e),
                        order,
                    }
                }
            }
            (Ok(yes_resp), Err(e)) => {
                if yes_resp.success {
                    warn!(
                        "NO leg HTTP error but YES leg filled — naked position!"
                    );
                    ExecutionResult::PartialFill {
                        yes_fill_size: order.approved_size,
                        no_fill_size: Decimal::ZERO,
                        total_cost: order.yes_price * order.approved_size,
                        detail: format!("NO leg HTTP error: {}", e),
                        order,
                    }
                } else {
                    ExecutionResult::Error {
                        error: format!("NO leg error: {}", e),
                        order,
                    }
                }
            }
            (Err(e1), Err(e2)) => ExecutionResult::Error {
                error: format!("Both legs failed: YES={}, NO={}", e1, e2),
                order,
            },
        }
    }

    /// Place a single buy order on the CLOB.
    async fn place_order(
        &self,
        token_id: &str,
        price: Decimal,
        size: Decimal,
        order_type: &OrderType,
    ) -> Result<ClobOrderResponse, ExecutorError> {
        let url = format!("{}/order", self.clob_url);

        let body = ClobOrderRequest {
            token_id: token_id.to_string(),
            price: price.to_string(),
            size: size.to_string(),
            side: "BUY".to_string(),
            order_type: order_type.clone(),
        };

        let mut headers = HeaderMap::new();
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));

        // Add authentication headers.
        // In production, these would include HMAC-signed headers per the CLOB spec:
        //   POLY_ADDRESS, POLY_SIGNATURE, POLY_TIMESTAMP, POLY_NONCE, POLY_API_KEY, POLY_PASSPHRASE
        if let (Some(key), Some(_secret), Some(passphrase)) =
            (&self.api_key, &self.api_secret, &self.api_passphrase)
        {
            headers.insert(
                "POLY_API_KEY",
                HeaderValue::from_str(key).unwrap_or(HeaderValue::from_static("")),
            );
            headers.insert(
                "POLY_PASSPHRASE",
                HeaderValue::from_str(passphrase).unwrap_or(HeaderValue::from_static("")),
            );
            let timestamp = chrono::Utc::now().timestamp().to_string();
            headers.insert(
                "POLY_TIMESTAMP",
                HeaderValue::from_str(&timestamp).unwrap_or(HeaderValue::from_static("0")),
            );

            // NOTE: Full HMAC signing implementation would go here.
            // The actual signature computation depends on the CLOB's L2 auth spec:
            //   signature = HMAC-SHA256(secret, timestamp + method + path + body)
            // This is left as a TODO for when the wallet is funded and API key is derived.
            debug!("HMAC signing placeholder — full implementation needed for live trading");
        }

        let resp = self
            .client
            .post(&url)
            .headers(headers)
            .json(&body)
            .send()
            .await?;

        let status = resp.status().as_u16();
        if status >= 400 {
            let body_text = resp.text().await.unwrap_or_default();
            return Err(ExecutorError::Api {
                status,
                body: body_text,
            });
        }

        let order_resp: ClobOrderResponse = resp.json().await?;
        Ok(order_resp)
    }

    /// Execute a multi-level sweep (buy YES + buy NO at worst prices to sweep levels).
    ///
    /// A single FOK buy at `worst_price` sweeps all resting asks <= that price,
    /// so this is structurally the same as `execute()` but uses sweep limit prices.
    pub async fn execute_sweep(&self, order: SweepExecutionOrder) -> ExecutionResult {
        if self.dry_run {
            info!(
                condition_id = %order.opportunity.market.condition_id,
                yes_limit = %order.yes_limit_price,
                no_limit = %order.no_limit_price,
                size = %order.approved_size,
                expected_profit = %order.opportunity.expected_profit,
                yes_levels = order.opportunity.yes_sweep.levels.len(),
                no_levels = order.opportunity.no_sweep.levels.len(),
                "[DRY RUN] Would execute sweep"
            );
            // Convert to an ExecutionOrder-shaped result for compatibility.
            let exec_order = ExecutionOrder {
                opportunity: arb_types::ArbitrageOpportunity {
                    id: order.opportunity.id,
                    market: order.opportunity.market.clone(),
                    yes_ask_price: order.yes_limit_price,
                    no_ask_price: order.no_limit_price,
                    yes_ask_size: order.opportunity.yes_sweep.total_size,
                    no_ask_size: order.opportunity.no_sweep.total_size,
                    gross_spread: order.opportunity.net_spread,
                    net_spread: order.opportunity.net_spread,
                    max_size: order.opportunity.matched_size,
                    expected_profit: order.opportunity.expected_profit,
                    detected_at: order.opportunity.detected_at,
                },
                approved_size: order.approved_size,
                yes_price: order.yes_limit_price,
                no_price: order.no_limit_price,
                use_fok: order.use_fok,
            };
            return ExecutionResult::DryRun { order: exec_order };
        }

        if self.api_key.is_none() {
            let exec_order = ExecutionOrder {
                opportunity: arb_types::ArbitrageOpportunity {
                    id: order.opportunity.id,
                    market: order.opportunity.market.clone(),
                    yes_ask_price: order.yes_limit_price,
                    no_ask_price: order.no_limit_price,
                    yes_ask_size: order.opportunity.yes_sweep.total_size,
                    no_ask_size: order.opportunity.no_sweep.total_size,
                    gross_spread: order.opportunity.net_spread,
                    net_spread: order.opportunity.net_spread,
                    max_size: order.opportunity.matched_size,
                    expected_profit: order.opportunity.expected_profit,
                    detected_at: order.opportunity.detected_at,
                },
                approved_size: order.approved_size,
                yes_price: order.yes_limit_price,
                no_price: order.no_limit_price,
                use_fok: order.use_fok,
            };
            return ExecutionResult::Error {
                error: "API credentials not configured".to_string(),
                order: exec_order,
            };
        }

        let order_type = if order.use_fok {
            OrderType::Fok
        } else {
            OrderType::Gtc
        };

        let yes_future = self.place_order(
            &order.opportunity.market.yes_token_id,
            order.yes_limit_price,
            order.approved_size,
            &order_type,
        );
        let no_future = self.place_order(
            &order.opportunity.market.no_token_id,
            order.no_limit_price,
            order.approved_size,
            &order_type,
        );

        let (yes_result, no_result) = tokio::join!(yes_future, no_future);

        let exec_order = ExecutionOrder {
            opportunity: arb_types::ArbitrageOpportunity {
                id: order.opportunity.id,
                market: order.opportunity.market.clone(),
                yes_ask_price: order.yes_limit_price,
                no_ask_price: order.no_limit_price,
                yes_ask_size: order.opportunity.yes_sweep.total_size,
                no_ask_size: order.opportunity.no_sweep.total_size,
                gross_spread: order.opportunity.net_spread,
                net_spread: order.opportunity.net_spread,
                max_size: order.opportunity.matched_size,
                expected_profit: order.opportunity.expected_profit,
                detected_at: order.opportunity.detected_at,
            },
            approved_size: order.approved_size,
            yes_price: order.yes_limit_price,
            no_price: order.no_limit_price,
            use_fok: order.use_fok,
        };

        match (yes_result, no_result) {
            (Ok(yes_resp), Ok(no_resp)) => {
                if yes_resp.success && no_resp.success {
                    let total_cost =
                        (exec_order.yes_price + exec_order.no_price) * exec_order.approved_size;
                    ExecutionResult::FullFill {
                        yes_fill_size: exec_order.approved_size,
                        no_fill_size: exec_order.approved_size,
                        total_cost,
                        order: exec_order,
                    }
                } else if yes_resp.success || no_resp.success {
                    let (yes_fill, no_fill) = if yes_resp.success {
                        (exec_order.approved_size, Decimal::ZERO)
                    } else {
                        (Decimal::ZERO, exec_order.approved_size)
                    };
                    let filled_price = if yes_resp.success {
                        exec_order.yes_price
                    } else {
                        exec_order.no_price
                    };
                    let total_cost = filled_price * exec_order.approved_size;
                    let failed_side = if yes_resp.success { "NO" } else { "YES" };
                    let failed_msg = if yes_resp.success {
                        no_resp.error_msg.unwrap_or_default()
                    } else {
                        yes_resp.error_msg.unwrap_or_default()
                    };

                    ExecutionResult::PartialFill {
                        yes_fill_size: yes_fill,
                        no_fill_size: no_fill,
                        total_cost,
                        detail: format!("{} leg failed: {}", failed_side, failed_msg),
                        order: exec_order,
                    }
                } else {
                    let reason = format!(
                        "YES: {}, NO: {}",
                        yes_resp.error_msg.unwrap_or_else(|| "unknown".to_string()),
                        no_resp.error_msg.unwrap_or_else(|| "unknown".to_string()),
                    );
                    ExecutionResult::NoFill {
                        order: exec_order,
                        reason,
                    }
                }
            }
            (Err(e), _) => ExecutionResult::Error {
                error: format!("YES leg error: {}", e),
                order: exec_order,
            },
            (_, Err(e)) => ExecutionResult::Error {
                error: format!("NO leg error: {}", e),
                order: exec_order,
            },
        }
    }

    /// Execute a single-leg buy order.
    pub async fn execute_leg(&self, order: LegOrder) -> LegExecutionResult {
        if self.dry_run {
            info!(
                condition_id = %order.condition_id,
                side = %order.side,
                target_price = %order.target_price,
                size = %order.size,
                "[DRY RUN] Would execute leg"
            );
            return LegExecutionResult::DryRun { order };
        }

        if self.api_key.is_none() {
            return LegExecutionResult::Error {
                error: "API credentials not configured".to_string(),
                order,
            };
        }

        let order_type = if order.use_fok {
            OrderType::Fok
        } else {
            OrderType::Gtc
        };

        match self
            .place_order(&order.token_id, order.target_price, order.size, &order_type)
            .await
        {
            Ok(resp) => {
                if resp.success {
                    let fill_cost = order.target_price * order.size;
                    LegExecutionResult::Filled {
                        fill_size: order.size,
                        fill_cost,
                        order,
                    }
                } else {
                    let reason = resp.error_msg.unwrap_or_else(|| "unknown".to_string());
                    LegExecutionResult::NoFill { order, reason }
                }
            }
            Err(e) => LegExecutionResult::Error {
                error: e.to_string(),
                order,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use arb_types::{ArbitrageOpportunity, BinaryMarket};
    use rust_decimal_macros::dec;
    use uuid::Uuid;

    fn make_test_config() -> AppConfig {
        arb_config::AppConfig {
            polymarket: arb_config::PolymarketConfig {
                clob_url: "https://clob.polymarket.com".to_string(),
                gamma_url: "https://gamma-api.polymarket.com".to_string(),
                ws_url: "wss://ws-subscriptions-clob.polymarket.com/ws/market".to_string(),
                chain_id: 137,
                wallet_address: String::new(),
            },
            strategy: arb_config::StrategyConfig {
                min_net_spread: dec!(0.005),
                base_fee_rate: dec!(0.0),
                max_price_staleness_ms: 2000,
                use_fok_orders: true,
                mode: Default::default(),
                max_sweep_levels: 5,
                min_sweep_profit_usdc: dec!(0.50),
                asymmetric_target_total_cost: dec!(0.97),
                max_unpaired_hold_secs: 3600,
                max_unpaired_exposure_usdc: dec!(200),
                max_unpaired_legs_per_market: 3,
            },
            risk: arb_config::RiskConfig {
                max_order_size_usdc: dec!(50),
                max_total_exposure_usdc: dec!(500),
                min_reserve_balance_usdc: dec!(50),
                per_market_cooldown_secs: 10,
                max_concurrent_positions: 10,
                max_unpaired_exposure_usdc: dec!(200),
                max_unpaired_per_market_usdc: dec!(50),
            },
            scanner: arb_config::ScannerConfig {
                refresh_interval_secs: 300,
                max_markets: 200,
                min_liquidity_usdc: dec!(100),
                market_keywords: Vec::new(),
            },
            monitor: arb_config::MonitorConfig {
                use_websocket: true,
                poll_interval_ms: 2000,
                max_concurrent_requests: 20,
            },
            logging: arb_config::LoggingConfig {
                level: "info".to_string(),
                json_output: false,
            },
        }
    }

    fn make_test_order() -> ExecutionOrder {
        ExecutionOrder {
            opportunity: ArbitrageOpportunity {
                id: Uuid::new_v4(),
                market: BinaryMarket {
                    condition_id: "test-cond".to_string(),
                    yes_token_id: "yes-tok".to_string(),
                    no_token_id: "no-tok".to_string(),
                    question: "Test market?".to_string(),
                    slug: "test".to_string(),
                    active: true,
                    liquidity: Some(dec!(1000)),
                },
                yes_ask_price: dec!(0.45),
                no_ask_price: dec!(0.50),
                yes_ask_size: dec!(100),
                no_ask_size: dec!(80),
                gross_spread: dec!(0.05),
                net_spread: dec!(0.05),
                max_size: dec!(80),
                expected_profit: dec!(4.0),
                detected_at: chrono::Utc::now(),
            },
            approved_size: dec!(50),
            yes_price: dec!(0.45),
            no_price: dec!(0.50),
            use_fok: true,
        }
    }

    #[tokio::test]
    async fn test_dry_run_returns_dry_run_result() {
        let config = make_test_config();
        let executor = OrderExecutor::new_dry_run(&config);
        let order = make_test_order();

        let result = executor.execute(order).await;
        assert!(matches!(result, ExecutionResult::DryRun { .. }));
    }
}
