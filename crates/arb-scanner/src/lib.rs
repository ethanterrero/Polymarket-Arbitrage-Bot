use arb_config::AppConfig;
use arb_types::BinaryMarket;
use chrono::{DateTime, Utc};
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
    /// Trailing 24-hour traded volume. Gamma uses several field names
    /// historically; we accept `volume24hrClob` (canonical for CLOB markets)
    /// first, then fall back to `volume24hr`.
    #[serde(default, rename = "volume24hrClob")]
    volume_24h_clob: Option<Decimal>,
    #[serde(default, rename = "volume24hr")]
    volume_24h: Option<Decimal>,
    /// Scheduled resolution time. RFC3339, e.g. `"2026-06-01T00:00:00Z"`.
    #[serde(default, rename = "endDate")]
    end_date: Option<DateTime<Utc>>,
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
    /// Skip markets with 24h volume below this. `0` disables.
    min_24h_volume: Decimal,
    /// Skip markets within this many seconds of resolution. `0` disables.
    min_secs_to_resolution: u64,
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
            min_24h_volume: config.scanner.min_24h_volume_usdc,
            min_secs_to_resolution: config.scanner.min_secs_to_resolution,
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
        let volume_24h = raw.volume_24h_clob.or(raw.volume_24h);
        let end_date = raw.end_date;

        // 24h volume filter: skip dormant markets. When Gamma omits the
        // field we keep the market — the alternative (rejecting all
        // unreported volumes) would drop too much of the universe.
        if self.min_24h_volume > Decimal::ZERO {
            match volume_24h {
                Some(v) if v < self.min_24h_volume => return None,
                _ => {}
            }
        }

        // Time-to-resolution filter: skip markets resolving too soon. When
        // end_date is missing or already in the past we keep the market
        // (Gamma's `closed=true` filter above already removed resolved
        // markets, so a missing end_date here is most likely an unscheduled
        // market rather than one that's silently about to resolve).
        if self.min_secs_to_resolution > 0 {
            if let Some(end) = end_date {
                let secs_left = (end - Utc::now()).num_seconds();
                if secs_left > 0 && (secs_left as u64) < self.min_secs_to_resolution {
                    return None;
                }
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
            neg_risk,
            fee_rate_bps: self.default_fee_rate_bps,
            min_tick_size,
            volume_24h,
            end_date,
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

    // ─── Phase 5: scanner filters ─────────────────────────────────────────

    fn make_scanner_with_filters(min_volume: Decimal, min_secs: u64) -> MarketScanner {
        MarketScanner {
            client: reqwest::Client::new(),
            gamma_url: "https://test".to_string(),
            max_markets: 100,
            min_liquidity: dec!(0),
            default_fee_rate_bps: 0,
            min_24h_volume: min_volume,
            min_secs_to_resolution: min_secs,
            markets: Arc::new(RwLock::new(Vec::new())),
        }
    }

    fn make_raw(
        condition_id: &str,
        volume_24hr_clob: Option<&str>,
        volume_24hr: Option<&str>,
        end_date: Option<&str>,
    ) -> GammaMarketResponse {
        GammaMarketResponse {
            condition_id: Some(condition_id.to_string()),
            question: Some("Q?".to_string()),
            slug: Some("slug".to_string()),
            active: Some(true),
            closed: Some(false),
            clob_token_ids: Some(r#"["1","2"]"#.to_string()),
            liquidity: Some("10000".to_string()),
            neg_risk: Some(false),
            order_price_min_tick_size: Some(dec!(0.01)),
            volume_24h_clob: volume_24hr_clob.and_then(|s| s.parse().ok()),
            volume_24h: volume_24hr.and_then(|s| s.parse().ok()),
            end_date: end_date.and_then(|s| DateTime::parse_from_rfc3339(s).ok().map(Into::into)),
        }
    }

    #[test]
    fn volume_filter_drops_low_volume_markets() {
        let scanner = make_scanner_with_filters(dec!(1000), 0);
        let raw = make_raw("0xabc", Some("500"), None, None);
        assert!(scanner.parse_binary_market(raw).is_none());
    }

    #[test]
    fn volume_filter_keeps_high_volume_markets() {
        let scanner = make_scanner_with_filters(dec!(1000), 0);
        let raw = make_raw("0xabc", Some("5000"), None, None);
        let m = scanner.parse_binary_market(raw).unwrap();
        assert_eq!(m.volume_24h, Some(dec!(5000)));
    }

    #[test]
    fn volume_filter_falls_back_to_legacy_field() {
        // volume24hrClob missing, volume24hr present.
        let scanner = make_scanner_with_filters(dec!(1000), 0);
        let raw = make_raw("0xabc", None, Some("5000"), None);
        let m = scanner.parse_binary_market(raw).unwrap();
        assert_eq!(m.volume_24h, Some(dec!(5000)));
    }

    #[test]
    fn volume_filter_keeps_market_when_volume_missing() {
        // Don't drop everything Gamma fails to report on; safer to keep.
        let scanner = make_scanner_with_filters(dec!(1000), 0);
        let raw = make_raw("0xabc", None, None, None);
        assert!(scanner.parse_binary_market(raw).is_some());
    }

    #[test]
    fn volume_filter_disabled_when_threshold_is_zero() {
        let scanner = make_scanner_with_filters(dec!(0), 0);
        let raw = make_raw("0xabc", Some("0.1"), None, None);
        assert!(scanner.parse_binary_market(raw).is_some());
    }

    #[test]
    fn resolution_filter_drops_imminent_markets() {
        let scanner = make_scanner_with_filters(dec!(0), 3 * 24 * 3600); // 72h
        let in_1h = (Utc::now() + chrono::Duration::hours(1)).to_rfc3339();
        let raw = make_raw("0xabc", None, None, Some(&in_1h));
        assert!(scanner.parse_binary_market(raw).is_none());
    }

    #[test]
    fn resolution_filter_keeps_far_off_markets() {
        let scanner = make_scanner_with_filters(dec!(0), 3 * 24 * 3600);
        let in_30d = (Utc::now() + chrono::Duration::days(30)).to_rfc3339();
        let raw = make_raw("0xabc", None, None, Some(&in_30d));
        let m = scanner.parse_binary_market(raw).unwrap();
        assert!(m.end_date.is_some());
    }

    #[test]
    fn resolution_filter_keeps_market_with_no_end_date() {
        // Gamma's `closed=true` filter already removes resolved markets, so a
        // missing end_date is more likely a long-tail unscheduled market
        // than something silently about to resolve.
        let scanner = make_scanner_with_filters(dec!(0), 3 * 24 * 3600);
        let raw = make_raw("0xabc", None, None, None);
        assert!(scanner.parse_binary_market(raw).is_some());
    }

    #[test]
    fn resolution_filter_ignores_past_dates() {
        // end_date in the past → don't apply the filter (market is presumably
        // resolved/closing imminently; let other filters or `closed` deal).
        let scanner = make_scanner_with_filters(dec!(0), 3 * 24 * 3600);
        let past = (Utc::now() - chrono::Duration::days(1)).to_rfc3339();
        let raw = make_raw("0xabc", None, None, Some(&past));
        assert!(scanner.parse_binary_market(raw).is_some());
    }

    #[test]
    fn deserializes_volume_and_end_date_fields() {
        let json = r#"{
            "conditionId": "0xabc",
            "active": true,
            "closed": false,
            "clobTokenIds": "[\"1\",\"2\"]",
            "volume24hrClob": "12345.67",
            "endDate": "2026-12-31T23:59:00Z"
        }"#;
        let raw: GammaMarketResponse = serde_json::from_str(json).unwrap();
        assert_eq!(raw.volume_24h_clob, Some(dec!(12345.67)));
        assert!(raw.end_date.is_some());
    }
}
