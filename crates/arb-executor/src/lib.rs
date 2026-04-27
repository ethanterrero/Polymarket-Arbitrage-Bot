pub mod auth;
pub mod fee_rates;
pub mod order_signing;
pub mod proxy_address;

use arb_config::AppConfig;
use arb_types::{
    BinaryMarket, ExecutionOrder, ExecutionResult, LegExecutionResult, LegOrder, SweepExecutionOrder,
};
use auth::ApiCredentials;
use fee_rates::FeeRateCache;
use k256::ecdsa::SigningKey;
use order_signing::{Order, OrderSide, SignatureType};
use primitive_types::{H160, U256};
use reqwest::header::{HeaderMap, HeaderValue, CONTENT_TYPE};
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive;
use serde::{Deserialize, Serialize};
use tracing::{info, warn};

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
    #[error("fee-rate fetch error: {0}")]
    FeeRate(#[from] fee_rates::FeeRateError),
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
    chain_id: u64,
    creds: Option<ApiCredentials>,
    signing_key: Option<SigningKey>,
    signature_type: SignatureType,
    /// Per-token-id cache of CLOB `base_fee` rates, fetched lazily before
    /// signing each order. Replaces the config-derived default that lived
    /// on `BinaryMarket.fee_rate_bps`.
    fee_rates: FeeRateCache,
}

impl OrderExecutor {
    /// Create an executor in dry-run mode (no orders placed).
    pub fn new_dry_run(config: &AppConfig) -> Self {
        Self {
            client: reqwest::Client::new(),
            clob_url: config.polymarket.clob_url.clone(),
            dry_run: true,
            chain_id: config.polymarket.chain_id,
            creds: None,
            signing_key: None,
            signature_type: SignatureType::Eoa,
            fee_rates: FeeRateCache::new(),
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
            chain_id: config.polymarket.chain_id,
            creds: Some(creds),
            signing_key: Some(signing_key),
            signature_type,
            fee_rates: FeeRateCache::new(),
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
        let fee_rate_bps = self
            .fee_rates
            .get(&self.client, &self.clob_url, token_id)
            .await?;
        let (body_json, headers) = self.build_signed_order_http_request(
            market,
            token_id,
            price,
            size,
            order_type,
            fee_rate_bps,
            None,
            None,
        )?;

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
        fee_rate_bps: u32,
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

        let chain_id = self.chain_id;
        let verifying = order_signing::contracts::verifying_contract(chain_id, market.neg_risk)
            .ok_or_else(|| ExecutorError::Auth(format!("Unsupported chain_id: {}", chain_id)))?;
        let verifying_h160 = order_signing::parse_address(verifying)
            .map_err(|e| ExecutorError::Auth(e.to_string()))?;

        let signer: H160 = order_signing::parse_address(&creds.wallet_address)
            .map_err(|e| ExecutorError::Auth(e.to_string()))?;
        let signature_type = self.signature_type;
        let maker = proxy_address::derive_maker_address(signer, signature_type);

        let (maker_amount, taker_amount) =
            buy_order_amounts(price, size, market.min_tick_size)?;

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
            fee_rate_bps: U256::from(fee_rate_bps),
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
                fee_rate_bps: fee_rate_bps.to_string(),
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
            &order.opportunity.market,
            &order.opportunity.market.yes_token_id,
            order.yes_limit_price,
            order.approved_size,
            &order_type,
        );
        let no_future = self.place_order(
            &order.opportunity.market,
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

/// Decimal places for each supported `min_tick_size`. Mirrors the
/// `ROUNDING_CONFIG` table in `Polymarket/py-clob-client`
/// (`order_builder/builder.py`): only `0.1`, `0.01`, `0.001`, `0.0001` are
/// valid CLOB tick sizes.
fn tick_decimals(tick: Decimal) -> Result<u32, ExecutorError> {
    if tick <= Decimal::ZERO {
        return Err(ExecutorError::OrderRejected(
            "min_tick_size must be > 0".to_string(),
        ));
    }
    let t = tick.normalize();
    let scale = t.scale();
    if t.mantissa() == 1 && (1..=4).contains(&scale) {
        Ok(scale)
    } else {
        Err(ExecutorError::OrderRejected(format!(
            "unsupported min_tick_size: {} (expected 0.1, 0.01, 0.001, or 0.0001)",
            tick
        )))
    }
}

/// Quantize a (price, size) pair for a Polymarket BUY order, matching
/// `py-clob-client`'s `get_order_amounts` / `ROUNDING_CONFIG`:
///
/// - price → banker's-rounded ("round_normal") to the tick's decimal places.
/// - size  → truncated ("round_down") to 2 decimal places.
///
/// Polymarket orders are constrained to size in 0.01-token units; truncating
/// to 6 dp would let the bot send amounts the CLOB rejects. The tick decimals
/// table above is the per-market price precision.
fn quantize_price_and_size(
    price: Decimal,
    size: Decimal,
    tick: Decimal,
) -> Result<(Decimal, Decimal), ExecutorError> {
    let price_dp = tick_decimals(tick)?;
    let q_price =
        price.round_dp_with_strategy(price_dp, rust_decimal::RoundingStrategy::MidpointNearestEven);
    let q_size = size.round_dp_with_strategy(2, rust_decimal::RoundingStrategy::ToZero);
    Ok((q_price, q_size))
}

/// Scale a `Decimal` by `10^scale` and convert to a `U256`. Mirrors
/// `to_token_decimals` in `py-clob-client/order_builder/helpers.py`: scale
/// then banker's-round to integer. With the upstream-matching quantization
/// above (price ≤ 4 dp, size ≤ 2 dp), products always fit in 6 dp so the
/// rounding step is a no-op, but it stays for defense-in-depth.
fn decimal_to_u256_scaled(value: Decimal, scale: u32) -> Result<U256, ExecutorError> {
    let factor = Decimal::from_i128_with_scale(10i128.pow(scale), 0);
    let scaled = (value * factor)
        .round_dp_with_strategy(0, rust_decimal::RoundingStrategy::MidpointNearestEven);
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

/// Compute `(maker_amount, taker_amount)` as `U256` token-decimal integers
/// for a BUY order, applying the same quantization the CLOB expects.
/// Pinned against `py-clob-client`'s `get_order_amounts` for the BUY branch.
fn buy_order_amounts(
    price: Decimal,
    size: Decimal,
    tick: Decimal,
) -> Result<(U256, U256), ExecutorError> {
    let (q_price, q_size) = quantize_price_and_size(price, size, tick)?;
    let maker_amount = decimal_to_u256_scaled(q_price * q_size, 6)?;
    let taker_amount = decimal_to_u256_scaled(q_size, 6)?;
    Ok((maker_amount, taker_amount))
}

#[cfg(test)]
mod tests {
    use super::*;
    use arb_types::{ArbitrageOpportunity, BinaryMarket};
    use base64::Engine;
    use httpmock::Method::POST;
    use httpmock::MockServer;
    use k256::ecdsa::SigningKey;
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

    // ─── BUY order amount quantization — pinned against upstream ──────────
    //
    // Reference: `Polymarket/py-clob-client`, `order_builder/builder.py`
    // (`get_order_amounts`, BUY branch) + `order_builder/helpers.py`
    // (`round_normal`, `round_down`, `to_token_decimals`). The expected
    // integer amounts below are computed by hand using the same algorithm.

    #[test]
    fn buy_amounts_tick_001_basic() {
        // tick=0.01: round_normal(0.45, 2) = 0.45; round_down(10.0, 2) = 10.0;
        // raw_maker = 4.5; maker = 4_500_000; taker = 10_000_000.
        let (maker, taker) = buy_order_amounts(dec!(0.45), dec!(10.0), dec!(0.01)).unwrap();
        assert_eq!(maker, U256::from(4_500_000u64));
        assert_eq!(taker, U256::from(10_000_000u64));
    }

    #[test]
    fn buy_amounts_size_truncates_to_two_dp() {
        // tick=0.01, size=10.789 → round_down(size, 2) = 10.78 (NOT 10.79).
        // raw_maker = 10.78 * 0.50 = 5.39; maker = 5_390_000; taker = 10_780_000.
        // Catches the previous 6-dp size truncation bug.
        let (maker, taker) = buy_order_amounts(dec!(0.50), dec!(10.789), dec!(0.01)).unwrap();
        assert_eq!(maker, U256::from(5_390_000u64));
        assert_eq!(taker, U256::from(10_780_000u64));
    }

    #[test]
    fn buy_amounts_tick_0001_preserves_three_dp_price() {
        // tick=0.001, price=0.567 (already 3-dp, round_normal is a no-op),
        // size=12.345 → 12.34. raw_maker = 12.34 * 0.567 = 6.99678.
        // maker = 6_996_780; taker = 12_340_000.
        let (maker, taker) = buy_order_amounts(dec!(0.567), dec!(12.345), dec!(0.001)).unwrap();
        assert_eq!(maker, U256::from(6_996_780u64));
        assert_eq!(taker, U256::from(12_340_000u64));
    }

    #[test]
    fn buy_amounts_tick_00001_preserves_four_dp_price() {
        // tick=0.0001, price=0.1234, size=10.0. raw_maker = 1.234.
        // maker = 1_234_000; taker = 10_000_000.
        let (maker, taker) = buy_order_amounts(dec!(0.1234), dec!(10.0), dec!(0.0001)).unwrap();
        assert_eq!(maker, U256::from(1_234_000u64));
        assert_eq!(taker, U256::from(10_000_000u64));
    }

    #[test]
    fn buy_amounts_price_quantizes_to_tick_decimals() {
        // tick=0.01, price=0.4567 → round_normal(0.4567, 2) = 0.46
        // (3rd decimal 6 > 5, round up). size=10.0. raw_maker = 4.6.
        // maker = 4_600_000; taker = 10_000_000.
        let (maker, taker) = buy_order_amounts(dec!(0.4567), dec!(10.0), dec!(0.01)).unwrap();
        assert_eq!(maker, U256::from(4_600_000u64));
        assert_eq!(taker, U256::from(10_000_000u64));
    }

    #[test]
    fn buy_amounts_tick_01_one_decimal_price() {
        // tick=0.1, price=0.5, size=10.0 → maker = 5_000_000; taker = 10_000_000.
        let (maker, taker) = buy_order_amounts(dec!(0.5), dec!(10.0), dec!(0.1)).unwrap();
        assert_eq!(maker, U256::from(5_000_000u64));
        assert_eq!(taker, U256::from(10_000_000u64));
    }

    #[test]
    fn buy_amounts_rejects_unsupported_tick() {
        // py-clob-client only defines ROUNDING_CONFIG for {0.1, 0.01, 0.001, 0.0001};
        // anything else should fail loudly, not silently produce malformed amounts.
        let err = buy_order_amounts(dec!(0.45), dec!(10.0), dec!(0.005)).unwrap_err();
        assert!(matches!(err, ExecutorError::OrderRejected(_)));
    }

    #[test]
    fn buy_amounts_rejects_zero_or_negative_tick() {
        assert!(buy_order_amounts(dec!(0.45), dec!(10.0), dec!(0)).is_err());
        assert!(buy_order_amounts(dec!(0.45), dec!(10.0), dec!(-0.01)).is_err());
    }

    #[tokio::test]
    async fn test_dry_run_returns_dry_run_result() {
        let config = make_test_config();
        let executor = OrderExecutor::new_dry_run(&config);
        let order = make_test_order();

        let result = executor.execute(order).await;
        assert!(matches!(result, ExecutionResult::DryRun { .. }));
    }

    #[tokio::test]
    async fn live_place_order_sends_signed_body_and_auth_headers() {
        let server = MockServer::start();

        // Minimal successful response shape.
        let m = server.mock(|when, then| {
            when.method(POST).path("/order");
            then.status(200)
                .header("content-type", "application/json")
                .body(r#"{"success":true,"order_id":"abc"}"#);
        });

        // Deterministic signing key.
        let key_bytes =
            hex::decode("ac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80")
                .unwrap();
        let signing_key = SigningKey::from_bytes(key_bytes.as_slice().into()).unwrap();

        // Deterministic creds (auth headers use api_secret; make it decodable base64url).
        let secret = base64::engine::general_purpose::URL_SAFE.encode(b"test-secret-key-32-bytes-padding");
        let creds = ApiCredentials {
            api_key: "test-owner-api-key".to_string(),
            api_secret: secret,
            api_passphrase: "pass".to_string(),
            wallet_address: "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266".to_string(),
        };

        let market = BinaryMarket {
            condition_id: "cond".to_string(),
            yes_token_id: "1234".to_string(),
            no_token_id: "5678".to_string(),
            question: "q".to_string(),
            slug: "s".to_string(),
            active: true,
            liquidity: None,
            neg_risk: false,
            fee_rate_bps: 12,
            min_tick_size: dec!(0.01),
        };

        let exec = OrderExecutor {
            client: reqwest::Client::new(),
            clob_url: server.url(""),
            dry_run: false,
            chain_id: order_signing::contracts::POLYGON_CHAIN_ID,
            creds: Some(creds),
            signing_key: Some(signing_key),
            signature_type: SignatureType::Eoa,
            fee_rates: FeeRateCache::new(),
        };

        // Force deterministic timestamp + salt so auth signature is stable.
        let (body, headers) = exec
            .build_signed_order_http_request(
                &market,
                &market.yes_token_id,
                dec!(0.45),
                dec!(10.0),
                &OrderType::Gtc,
                12,
                Some("1700000000"),
                Some(42),
            )
            .unwrap();

        // Quick sanity: body contains expected top-level keys.
        let v: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert!(v.get("order").is_some());
        assert_eq!(v["owner"], "test-owner-api-key");

        let resp = exec
            .client
            .post(format!("{}/order", exec.clob_url))
            .headers(headers)
            .body(body)
            .send()
            .await
            .unwrap();
        assert!(resp.status().is_success());

        m.assert();
    }

    #[tokio::test]
    async fn place_order_resolves_fee_rate_via_clob_and_caches() {
        use httpmock::Method::GET;
        let server = MockServer::start();

        // /fee-rate returns base_fee = 17 bps for the queried token.
        let fee_mock = server.mock(|when, then| {
            when.method(GET)
                .path("/fee-rate")
                .query_param("token_id", "1234");
            then.status(200)
                .header("content-type", "application/json")
                .body(r#"{"base_fee":17}"#);
        });

        // /order accepts and returns success. Capture the body so we can assert
        // that the fetched fee_rate_bps (17), not the BinaryMarket default (99),
        // ended up in the signed payload.
        let order_mock = server.mock(|when, then| {
            when.method(POST)
                .path("/order")
                .body_includes(r#""feeRateBps":"17""#);
            then.status(200)
                .header("content-type", "application/json")
                .body(r#"{"success":true,"order_id":"abc"}"#);
        });

        let key_bytes =
            hex::decode("ac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80")
                .unwrap();
        let signing_key = SigningKey::from_bytes(key_bytes.as_slice().into()).unwrap();
        let secret = base64::engine::general_purpose::URL_SAFE.encode(b"test-secret-key-32-bytes-padding");
        let creds = ApiCredentials {
            api_key: "owner".to_string(),
            api_secret: secret,
            api_passphrase: "pass".to_string(),
            wallet_address: "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266".to_string(),
        };

        let market = BinaryMarket {
            condition_id: "cond".to_string(),
            yes_token_id: "1234".to_string(),
            no_token_id: "5678".to_string(),
            question: "q".to_string(),
            slug: "s".to_string(),
            active: true,
            liquidity: None,
            neg_risk: false,
            // Stale per-market default — must be ignored in favor of the
            // /fee-rate fetch.
            fee_rate_bps: 99,
            min_tick_size: dec!(0.01),
        };

        let exec = OrderExecutor {
            client: reqwest::Client::new(),
            clob_url: server.url(""),
            dry_run: false,
            chain_id: order_signing::contracts::POLYGON_CHAIN_ID,
            creds: Some(creds),
            signing_key: Some(signing_key),
            signature_type: SignatureType::Eoa,
            fee_rates: FeeRateCache::new(),
        };

        // First call: /fee-rate hit + /order hit.
        exec.place_order(&market, "1234", dec!(0.45), dec!(10.0), &OrderType::Gtc)
            .await
            .unwrap();
        // Second call: cache hit, no extra /fee-rate; /order hit again.
        exec.place_order(&market, "1234", dec!(0.45), dec!(10.0), &OrderType::Gtc)
            .await
            .unwrap();

        fee_mock.assert_calls(1);
        order_mock.assert_calls(2);
    }
}
