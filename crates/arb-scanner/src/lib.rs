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
#[derive(Debug, Deserialize)]
struct GammaMarketResponse {
    #[serde(default)]
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
}

/// Scans Polymarket's Gamma API for active binary markets.
pub struct MarketScanner {
    client: reqwest::Client,
    gamma_url: String,
    max_markets: usize,
    min_liquidity: Decimal,
    markets: Arc<RwLock<Vec<BinaryMarket>>>,
}

impl MarketScanner {
    pub fn new(config: &AppConfig) -> Self {
        Self {
            client: reqwest::Client::new(),
            gamma_url: config.polymarket.gamma_url.clone(),
            max_markets: config.scanner.max_markets,
            min_liquidity: config.scanner.min_liquidity_usdc,
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

        Some(BinaryMarket {
            condition_id,
            yes_token_id: token_ids[0].clone(),
            no_token_id: token_ids[1].clone(),
            question,
            slug,
            active,
            liquidity,
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

// Allow Arc<MarketScanner> usage in spawn_refresh_loop
impl std::ops::Deref for MarketScanner {
    type Target = Arc<RwLock<Vec<BinaryMarket>>>;
    fn deref(&self) -> &Self::Target {
        &self.markets
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn test_parse_clob_token_ids() {
        let json_str = r#"["71321045679252212594626385532706912750332728571942532289631379312455583992563","48331043336612883890938759509493159234755048973440113902679143305172568892391"]"#;
        let parsed: Vec<String> = serde_json::from_str(json_str).unwrap();
        assert_eq!(parsed.len(), 2);
    }
}
