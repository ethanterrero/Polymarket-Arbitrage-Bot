pub mod auth;
pub mod order_signing;
pub mod proxy_address;

use arb_config::AppConfig;
use arb_types::{
    BinaryMarket, ExecutionOrder, ExecutionResult, LegExecutionResult, LegOrder, SweepExecutionOrder,
};
use auth::ApiCredentials;
use k256::ecdsa::SigningKey;
use order_signing::{Order, OrderSide, SignatureType};
use primitive_types::{H160, U256};
use reqwest::header::{HeaderMap, HeaderValue, CONTENT_TYPE};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use tracing::{error, info, warn};

/// Signed order payload for the CLOB API.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ClobSignedOrder {
    /// Must be a JSON number that fits in u53.
    salt: u64,
    maker: String,
    signer: String,
    taker: String,
    token_id: String,
    maker_amount: String,
    taker_amount: String,
    expiration: String,
    nonce: String,
    fee_rate_bps: String,
    side: String,
    signature_type: u8,
    signature: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ClobPlaceOrderRequest {
    order: ClobSignedOrder,
    order_type: OrderType,
    owner: String,
}

#[derive(Debug, thiserror::Error)]
pub enum ExecutorError {
    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),
    #[error("serialization error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("API error: {status} — {body}")]
    Api { status: u16, body: String },
    #[error("auth error: {0}")]
    Auth(String),
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

// NOTE: `ClobOrderRequest` (unsigned) was replaced by `ClobPlaceOrderRequest` (signed).

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
    creds: Option<ApiCredentials>,
    signing_key: Option<SigningKey>,
    signature_type: SignatureType,
}

impl OrderExecutor {
    /// Create an executor in dry-run mode (no orders placed).
    pub fn new_dry_run(config: &AppConfig) -> Self {
        Self {
            client: reqwest::Client::new(),
            clob_url: config.polymarket.clob_url.clone(),
            dry_run: true,
            creds: None,
            signing_key: None,
            signature_type: SignatureType::Eoa,
        }
    }

    /// Derive API credentials from `private_key_hex` and return a live executor.
    ///
    /// Calls `POST /auth/derive-api-key` on the CLOB with an EIP-712 signed
    /// proof of wallet ownership. Errors if the network call fails or the key
    /// is malformed.
    pub async fn new_live(
        config: &AppConfig,
        private_key_hex: &str,
    ) -> Result<Self, auth::AuthError> {
        let client = reqwest::Client::new();
        let key_bytes = hex::decode(private_key_hex.trim_start_matches("0x"))?;
        let signing_key = SigningKey::from_bytes(key_bytes.as_slice().into())?;

        let signature_type = match std::env::var("POLYMARKET_SIGNATURE_TYPE")
            .unwrap_or_else(|_| "eoa".to_string())
            .to_lowercase()
            .as_str()
        {
            "eoa" => SignatureType::Eoa,
            "poly_proxy" | "proxy" => SignatureType::PolyProxy,
            "poly_gnosis_safe" | "safe" => SignatureType::PolyGnosisSafe,
            other => {
                warn!(value = other, "Unknown POLYMARKET_SIGNATURE_TYPE; defaulting to eoa");
                SignatureType::Eoa
            }
        };

        let creds = auth::derive_api_key(
            &client,
            &config.polymarket.clob_url,
            private_key_hex,
            config.polymarket.chain_id,
        )
        .await?;
        info!(
            wallet = %creds.wallet_address,
            api_key = %creds.api_key,
            "API key derived — live trading enabled"
        );
        Ok(Self {
            client,
            clob_url: config.polymarket.clob_url.clone(),
            dry_run: false,
            creds: Some(creds),
            signing_key: Some(signing_key),
            signature_type,
        })
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
        if self.creds.is_none() {
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
            &order.opportunity.market,
            &order.opportunity.market.yes_token_id,
            order.yes_price,
            order.approved_size,
            &order_type,
        );

        let no_future = self.place_order(
            &order.opportunity.market,
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
        market: &BinaryMarket,
        token_id: &str,
        price: Decimal,
        size: Decimal,
        order_type: &OrderType,
    ) -> Result<ClobOrderResponse, ExecutorError> {
        let url = format!("{}/order", self.clob_url);
        let (body_json, headers) =
            self.build_signed_order_http_request(market, token_id, price, size, order_type, None, None)?;

        let resp = self
            .client
            .post(&url)
            .headers(headers)
            .body(body_json)
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

    fn build_signed_order_http_request(
        &self,
        market: &BinaryMarket,
        token_id: &str,
        price: Decimal,
        size: Decimal,
        order_type: &OrderType,
        fixed_timestamp: Option<&str>,
        fixed_salt: Option<u64>,
    ) -> Result<(String, HeaderMap), ExecutorError> {
        let creds = self
            .creds
            .as_ref()
            .ok_or_else(|| ExecutorError::Auth("API credentials not configured".to_string()))?;
        let signing_key = self.signing_key.as_ref().ok_or_else(|| {
            ExecutorError::Auth("Signing key not configured (live executor required)".to_string())
        })?;

        // NOTE: chain id should come from config; the executor doesn't retain it today.
        // We allow override for tests / non-polygon via env.
        let chain_id: u64 = std::env::var("POLYMARKET_CHAIN_ID")
            .ok()
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(order_signing::contracts::POLYGON_CHAIN_ID);

        let verifying = order_signing::contracts::verifying_contract(chain_id, market.neg_risk)
            .ok_or_else(|| ExecutorError::Auth(format!("Unsupported chain_id: {}", chain_id)))?;
        let verifying_h160 = order_signing::parse_address(verifying)
            .map_err(|e| ExecutorError::Auth(e.to_string()))?;

        let signer: H160 = order_signing::parse_address(&creds.wallet_address)
            .map_err(|e| ExecutorError::Auth(e.to_string()))?;
        let signature_type = self.signature_type;
        let maker = proxy_address::derive_maker_address(signer, signature_type);

        let (q_price, q_size) = quantize_price_and_size(price, size, market.min_tick_size)?;
        let maker_amount = decimal_to_u256_scaled(q_price * q_size, 6)?;
        let taker_amount = decimal_to_u256_scaled(q_size, 6)?;

        let token_u256 = U256::from_dec_str(token_id.trim()).map_err(|_| {
            ExecutorError::OrderRejected(format!("invalid token_id (not a uint256): {}", token_id))
        })?;

        let salt = fixed_salt.unwrap_or_else(make_salt_u53);

        let order = Order {
            salt: U256::from(salt),
            maker,
            signer,
            taker: H160::zero(),
            token_id: token_u256,
            maker_amount,
            taker_amount,
            expiration: U256::zero(),
            nonce: U256::zero(),
            fee_rate_bps: U256::from(market.fee_rate_bps),
            side: OrderSide::Buy,
            signature_type,
        };

        let sig = order_signing::sign_order(signing_key, &order, chain_id, verifying_h160)
            .map_err(|e| ExecutorError::Auth(e.to_string()))?;
        let sig_hex = format!("0x{}", hex::encode(sig));

        let req_body = ClobPlaceOrderRequest {
            order: ClobSignedOrder {
                salt,
                maker: format!("0x{}", hex::encode(maker.as_bytes())),
                signer: creds.wallet_address.clone(),
                taker: "0x0000000000000000000000000000000000000000".to_string(),
                token_id: token_id.to_string(),
                maker_amount: maker_amount.to_string(),
                taker_amount: taker_amount.to_string(),
                expiration: "0".to_string(),
                nonce: "0".to_string(),
                fee_rate_bps: market.fee_rate_bps.to_string(),
                side: "BUY".to_string(),
                signature_type: signature_type as u8,
                signature: sig_hex,
            },
            order_type: order_type.clone(),
            owner: creds.api_key.clone(),
        };

        let body_json = serde_json::to_string(&req_body)?;

        let mut headers = HeaderMap::new();
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));

        let auth_headers = if let Some(ts) = fixed_timestamp {
            auth::build_auth_headers_at(creds, "POST", "/order", &body_json, ts)
        } else {
            auth::build_auth_headers(creds, "POST", "/order", &body_json)
        }
        .map_err(|e| ExecutorError::Auth(e.to_string()))?;
        headers.extend(auth_headers);

        Ok((body_json, headers))
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

        if self.creds.is_none() {
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

        if self.creds.is_none() {
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

        let market = BinaryMarket {
            condition_id: order.condition_id.clone(),
            yes_token_id: String::new(),
            no_token_id: String::new(),
            question: String::new(),
            slug: String::new(),
            active: true,
            liquidity: None,
            neg_risk: order.neg_risk,
            fee_rate_bps: order.fee_rate_bps,
            min_tick_size: order.min_tick_size,
        };

        match self
            .place_order(&market, &order.token_id, order.target_price, order.size, &order_type)
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

fn make_salt_u53() -> u64 {
    let ms = chrono::Utc::now().timestamp_millis() as u64;
    ms & ((1u64 << 53) - 1)
}

fn quantize_price_and_size(
    price: Decimal,
    size: Decimal,
    tick: Decimal,
) -> Result<(Decimal, Decimal), ExecutorError> {
    if tick <= Decimal::ZERO {
        return Err(ExecutorError::OrderRejected(
            "min_tick_size must be > 0".to_string(),
        ));
    }

    let steps = (price / tick).floor();
    let q_price = steps * tick;

    // Truncate size to 6 decimals (token amount scale) to match signed integer scaling.
    let q_size = size.round_dp_with_strategy(6, rust_decimal::RoundingStrategy::ToZero);

    Ok((q_price, q_size))
}

fn decimal_to_u256_scaled(value: Decimal, scale: u32) -> Result<U256, ExecutorError> {
    let factor = Decimal::from_i128_with_scale(10i128.pow(scale), 0);
    let scaled = (value * factor).round_dp_with_strategy(0, rust_decimal::RoundingStrategy::ToZero);
    let i = scaled
        .to_i128()
        .ok_or_else(|| ExecutorError::OrderRejected("amount scaling overflow".to_string()))?;
    if i < 0 {
        return Err(ExecutorError::OrderRejected(
            "amount must be non-negative".to_string(),
        ));
    }
    Ok(U256::from(i as u128))
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
            execution: arb_config::ExecutionConfig::default(),
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
                    neg_risk: false,
                    fee_rate_bps: 0,
                    min_tick_size: dec!(0.01),
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
