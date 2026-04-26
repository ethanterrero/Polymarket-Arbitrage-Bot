use arb_config::AppConfig;
use arb_types::BinaryMarket;
use rust_decimal::Decimal;
use serde::Deserialize;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, info, warn};

#[derive(Debug, thiserror::Error)]
pub enum ScannerError {
    #[error("HTTP request failed: {0}")]
    Http(#[from] reqwest::Error),
    #[error("Failed to parse market data: {0}")]
    Parse(String),
}

/// Response from the Gamma API `/markets` endpoint.
///
/// Field names mirror Gamma's camelCase wire format. The previous version of
/// this struct expected `condition_id` (snake_case), which silently never
/// matched and caused every market to be dropped — kept that in mind when
/// adding new fields here.
#[derive(Debug, Deserialize)]
struct GammaMarketResponse {
    #[serde(default, rename = "conditionId")]
    condition_id: Option<String>,
    #[serde(default)]
    question: Option<String>,
    #[serde(default)]
    slug: Option<String>,
    #[serde(default)]
    active: Option<bool>,
    #[serde(default)]
    closed: Option<bool>,
    /// JSON-encoded string like `["token_id_yes","token_id_no"]`
    #[serde(default, rename = "clobTokenIds")]
    clob_token_ids: Option<String>,
    #[serde(default)]
    liquidity: Option<String>,
    /// Whether this market routes to the Neg Risk CTF Exchange variant.
    #[serde(default, rename = "negRisk")]
    neg_risk: Option<bool>,
    /// Minimum tick size as a JSON number (e.g. `0.01` or `0.001`).
    #[serde(default, rename = "orderPriceMinTickSize")]
    order_price_min_tick_size: Option<Decimal>,
}

/// Default tick size used when Gamma omits `orderPriceMinTickSize`. Most
/// Polymarket binary markets settle at 1¢ resolution.
const DEFAULT_MIN_TICK_SIZE: Decimal = Decimal::from_parts(1, 0, 0, false, 2); // 0.01

/// Scans Polymarket's Gamma API for active binary markets.
pub struct MarketScanner {
    client: reqwest::Client,
    gamma_url: String,
    max_markets: usize,
    min_liquidity: Decimal,
    /// Default per-market fee in basis points, derived from the global
    /// `strategy.base_fee_rate`. TODO: replace with per-market rates fetched
    /// from the CLOB `/fee-rate-bps` endpoint when wiring the order body.
    default_fee_rate_bps: u32,
    markets: Arc<RwLock<Vec<BinaryMarket>>>,
}

impl MarketScanner {
    pub fn new(config: &AppConfig) -> Self {
        let default_fee_rate_bps = decimal_to_bps(config.strategy.base_fee_rate);
        Self {
            client: reqwest::Client::new(),
            gamma_url: config.polymarket.gamma_url.clone(),
            max_markets: config.scanner.max_markets,
            min_liquidity: config.scanner.min_liquidity_usdc,
            default_fee_rate_bps,
            markets: Arc::new(RwLock::new(Vec::new())),
        }
    }

    /// Get a shared reference to the current market list.
    pub fn markets(&self) -> Arc<RwLock<Vec<BinaryMarket>>> {
        self.markets.clone()
    }

    /// Fetch and update the list of active binary markets from Gamma API.
    pub async fn refresh(&self) -> Result<usize, ScannerError> {
        let mut all_markets = Vec::new();
        let mut offset = 0;
        let limit = 100;

        loop {
            let url = format!(
                "{}/markets?active=true&closed=false&limit={}&offset={}",
                self.gamma_url, limit, offset
            );

            debug!(url = %url, "Fetching markets from Gamma API");

            let response: Vec<GammaMarketResponse> =
                self.client.get(&url).send().await?.json().await?;

            let batch_len = response.len();

            for raw in response {
                if let Some(market) = self.parse_binary_market(raw) {
                    all_markets.push(market);
                }

                if all_markets.len() >= self.max_markets {
                    break;
                }
            }

            if batch_len < limit || all_markets.len() >= self.max_markets {
                break;
            }

            offset += limit;
        }

        let count = all_markets.len();
        info!(count, "Discovered binary markets");

        let mut markets = self.markets.write().await;
        *markets = all_markets;

        Ok(count)
    }

    /// Parse a raw Gamma API response into a BinaryMarket, if it qualifies.
    fn parse_binary_market(&self, raw: GammaMarketResponse) -> Option<BinaryMarket> {
        let condition_id = raw.condition_id?;
        let question = raw.question.unwrap_or_default();
        let slug = raw.slug.unwrap_or_default();
        let active = raw.active.unwrap_or(false);

        if !active || raw.closed.unwrap_or(false) {
            return None;
        }

        // Parse clobTokenIds — a JSON string like `["id1","id2"]`
        let token_ids_str = raw.clob_token_ids?;
        let token_ids: Vec<String> = serde_json::from_str(&token_ids_str)
            .map_err(|e| {
                debug!(
                    condition_id = %condition_id,
                    error = %e,
                    "Failed to parse clobTokenIds"
                );
                e
            })
            .ok()?;

        // Must have exactly 2 tokens for a binary market.
        if token_ids.len() != 2 {
            debug!(
                condition_id = %condition_id,
                token_count = token_ids.len(),
                "Skipping non-binary market"
            );
            return None;
        }

        // Check minimum liquidity.
        let liquidity = raw
            .liquidity
            .and_then(|s| s.parse::<Decimal>().ok());

        if let Some(liq) = liquidity {
            if liq < self.min_liquidity {
                return None;
            }
        }

        let neg_risk = raw.neg_risk.unwrap_or(false);
        let min_tick_size = raw.order_price_min_tick_size.unwrap_or(DEFAULT_MIN_TICK_SIZE);

        Some(BinaryMarket {
            condition_id,
            yes_token_id: token_ids[0].clone(),
            no_token_id: token_ids[1].clone(),
            question,
            slug,
            active,
            liquidity,
            neg_risk,
            fee_rate_bps: self.default_fee_rate_bps,
            min_tick_size,
        })
    }

    /// Spawn a background task that refreshes markets on a timer.
    pub fn spawn_refresh_loop(
        self: &Arc<Self>,
        interval_secs: u64,
    ) -> tokio::task::JoinHandle<()> {
        let scanner = self.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(
                tokio::time::Duration::from_secs(interval_secs),
            );

            loop {
                interval.tick().await;
                match scanner.refresh().await {
                    Ok(count) => {
                        info!(count, "Market refresh complete");
                    }
                    Err(e) => {
                        warn!(error = %e, "Market refresh failed");
                    }
                }
            }
        })
    }
}

/// Convert a fractional fee rate (e.g. `0.0005` = 5 bps) into integer basis
/// points. Truncates toward zero — sub-bp precision isn't meaningful for the
/// CTF Exchange, which encodes fees as `uint256` bps.
fn decimal_to_bps(rate: Decimal) -> u32 {
    let bps = rate * Decimal::from(10_000u32);
    bps.trunc()
        .to_string()
        .parse::<u32>()
        .unwrap_or(0)
}

// Allow Arc<MarketScanner> usage in spawn_refresh_loop
impl std::ops::Deref for MarketScanner {
    type Target = Arc<RwLock<Vec<BinaryMarket>>>;
    fn deref(&self) -> &Self::Target {
        &self.markets
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;

    #[test]
    fn test_parse_clob_token_ids() {
        let json_str = r#"["71321045679252212594626385532706912750332728571942532289631379312455583992563","48331043336612883890938759509493159234755048973440113902679143305172568892391"]"#;
        let parsed: Vec<String> = serde_json::from_str(json_str).unwrap();
        assert_eq!(parsed.len(), 2);
    }

    #[test]
    fn decimal_to_bps_basic_cases() {
        assert_eq!(decimal_to_bps(dec!(0.0)), 0);
        assert_eq!(decimal_to_bps(dec!(0.0001)), 1);
        assert_eq!(decimal_to_bps(dec!(0.005)), 50);
        assert_eq!(decimal_to_bps(dec!(0.01)), 100);
        // Sub-bp truncates toward zero, not rounds.
        assert_eq!(decimal_to_bps(dec!(0.00019)), 1);
    }

    /// Trimmed fixture captured from a real `gamma-api.polymarket.com/markets`
    /// response on 2026-04-25. Field names are exactly what the live API
    /// returns (camelCase), which is the bug class this struct redesign is
    /// guarding against.
    const SAMPLE_GAMMA_MARKET: &str = r#"{
        "id": "540816",
        "question": "Russia-Ukraine Ceasefire before GTA VI?",
        "conditionId": "0x9c1a953fe92c8357f1b646ba25d983aa83e90c525992db14fb726fa895cb5763",
        "slug": "russia-ukraine-ceasefire-before-gta-vi-554",
        "liquidity": "48036.6073",
        "active": true,
        "closed": false,
        "clobTokenIds": "[\"8501497159083948713316135768103773293754490207922884688769443031624417212426\", \"2527312495175492857904889758552137141356236738032676480522356889996545113869\"]",
        "orderPriceMinTickSize": 0.01,
        "negRisk": false
    }"#;

    #[test]
    fn deserializes_real_gamma_field_names() {
        let raw: GammaMarketResponse = serde_json::from_str(SAMPLE_GAMMA_MARKET).unwrap();
        assert_eq!(
            raw.condition_id.as_deref(),
            Some("0x9c1a953fe92c8357f1b646ba25d983aa83e90c525992db14fb726fa895cb5763"),
            "conditionId rename must match Gamma's camelCase wire format"
        );
        assert_eq!(raw.neg_risk, Some(false));
        assert_eq!(raw.order_price_min_tick_size, Some(dec!(0.01)));
        assert_eq!(raw.active, Some(true));
        assert_eq!(raw.closed, Some(false));
        assert!(raw.clob_token_ids.is_some());
    }

    #[test]
    fn defaults_apply_when_neg_risk_and_tick_size_are_missing() {
        let json = r#"{
            "conditionId": "0xabc",
            "active": true,
            "closed": false,
            "clobTokenIds": "[\"1\",\"2\"]"
        }"#;
        let raw: GammaMarketResponse = serde_json::from_str(json).unwrap();
        assert_eq!(raw.neg_risk, None);
        assert_eq!(raw.order_price_min_tick_size, None);
    }
}
