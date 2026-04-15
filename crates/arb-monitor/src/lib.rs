use arb_config::AppConfig;
use arb_types::{BinaryMarket, BinaryOrderBook, PriceLevel, Side, TokenOrderBook};
use chrono::Utc;
use futures_util::{SinkExt, StreamExt};
use rust_decimal::Decimal;
use serde::Deserialize;
use std::collections::HashMap;
use std::str::FromStr;
use std::sync::Arc;
use tokio::sync::{mpsc, RwLock, Semaphore};
use tokio_tungstenite::tungstenite::Message;
use tracing::{debug, error, info, warn};

#[derive(Debug, thiserror::Error)]
pub enum MonitorError {
    #[error("HTTP request failed: {0}")]
    Http(#[from] reqwest::Error),
    #[error("WebSocket error: {0}")]
    WebSocket(String),
    #[error("Parse error: {0}")]
    Parse(String),
}

/// Response from the CLOB `/balance` endpoint.
#[derive(Debug, Deserialize)]
struct ClobBalanceResponse {
    balance: Option<String>,
    #[serde(rename = "USDCBalance")]
    usdc_balance: Option<String>,
    // Some CLOB versions return a plain string — serde will try both fields.
}

/// Raw orderbook response from the CLOB REST API.
#[derive(Debug, Deserialize)]
struct ClobBookResponse {
    #[serde(default)]
    asks: Vec<ClobPriceLevel>,
    #[serde(default)]
    bids: Vec<ClobPriceLevel>,
}

#[derive(Debug, Deserialize)]
struct ClobPriceLevel {
    price: String,
    size: String,
}

/// WebSocket message types from the CLOB.
#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct WsMessage {
    #[serde(default)]
    event_type: Option<String>,
    #[serde(default)]
    asset_id: Option<String>,
    #[serde(default)]
    market: Option<String>,
    #[serde(default)]
    asks: Option<Vec<ClobPriceLevel>>,
    #[serde(default)]
    bids: Option<Vec<ClobPriceLevel>>,
}

/// Monitors orderbooks for binary markets via REST polling or WebSocket.
pub struct PriceMonitor {
    client: reqwest::Client,
    clob_url: String,
    ws_url: String,
    use_websocket: bool,
    poll_interval_ms: u64,
    max_concurrent: usize,
}

impl PriceMonitor {
    pub fn new(config: &AppConfig) -> Self {
        Self {
            client: reqwest::Client::new(),
            clob_url: config.polymarket.clob_url.clone(),
            ws_url: config.polymarket.ws_url.clone(),
            use_websocket: config.monitor.use_websocket,
            poll_interval_ms: config.monitor.poll_interval_ms,
            max_concurrent: config.monitor.max_concurrent_requests,
        }
    }

    /// Fetch the orderbook for a single token via REST.
    async fn fetch_book(
        &self,
        token_id: &str,
    ) -> Result<(Vec<PriceLevel>, Vec<PriceLevel>), MonitorError> {
        let url = format!("{}/book?token_id={}", self.clob_url, token_id);

        let resp: ClobBookResponse = self.client.get(&url).send().await?.json().await?;

        let asks = parse_price_levels(resp.asks);
        let bids = parse_price_levels(resp.bids);

        Ok((asks, bids))
    }

    /// Fetch the full binary orderbook (YES + NO) for a single market via REST.
    pub async fn fetch_binary_book(
        &self,
        market: &BinaryMarket,
    ) -> Result<BinaryOrderBook, MonitorError> {
        let (yes_asks, yes_bids) = self.fetch_book(&market.yes_token_id).await?;
        let (no_asks, no_bids) = self.fetch_book(&market.no_token_id).await?;

        Ok(BinaryOrderBook {
            condition_id: market.condition_id.clone(),
            yes_book: TokenOrderBook {
                token_id: market.yes_token_id.clone(),
                side: Side::Yes,
                asks: yes_asks,
                bids: yes_bids,
            },
            no_book: TokenOrderBook {
                token_id: market.no_token_id.clone(),
                side: Side::No,
                asks: no_asks,
                bids: no_bids,
            },
            timestamp: Utc::now(),
        })
    }

    /// Start the REST polling loop for all markets, sending updates to the channel.
    pub async fn start_rest_polling(
        &self,
        markets: Arc<RwLock<Vec<BinaryMarket>>>,
        tx: mpsc::Sender<BinaryOrderBook>,
    ) {
        let semaphore = Arc::new(Semaphore::new(self.max_concurrent));
        let poll_interval = tokio::time::Duration::from_millis(self.poll_interval_ms);

        info!(
            interval_ms = self.poll_interval_ms,
            max_concurrent = self.max_concurrent,
            "Starting REST polling loop"
        );

        loop {
            let current_markets = markets.read().await.clone();

            for market in current_markets {
                let permit = semaphore.clone().acquire_owned().await;
                let tx = tx.clone();
                let monitor_client = self.client.clone();
                let clob_url = self.clob_url.clone();

                tokio::spawn(async move {
                    let _permit = permit;

                    let fetch = async {
                        let yes_url =
                            format!("{}/book?token_id={}", clob_url, market.yes_token_id);
                        let no_url =
                            format!("{}/book?token_id={}", clob_url, market.no_token_id);

                        let (yes_resp, no_resp) = tokio::join!(
                            monitor_client.get(&yes_url).send(),
                            monitor_client.get(&no_url).send(),
                        );

                        let yes_book: ClobBookResponse = yes_resp?.json().await?;
                        let no_book: ClobBookResponse = no_resp?.json().await?;

                        Ok::<_, reqwest::Error>(BinaryOrderBook {
                            condition_id: market.condition_id.clone(),
                            yes_book: TokenOrderBook {
                                token_id: market.yes_token_id.clone(),
                                side: Side::Yes,
                                asks: parse_price_levels(yes_book.asks),
                                bids: parse_price_levels(yes_book.bids),
                            },
                            no_book: TokenOrderBook {
                                token_id: market.no_token_id.clone(),
                                side: Side::No,
                                asks: parse_price_levels(no_book.asks),
                                bids: parse_price_levels(no_book.bids),
                            },
                            timestamp: Utc::now(),
                        })
                    };

                    match fetch.await {
                        Ok(book) => {
                            if tx.send(book).await.is_err() {
                                debug!("Channel closed, stopping polling for market");
                            }
                        }
                        Err(e) => {
                            warn!(
                                condition_id = %market.condition_id,
                                error = %e,
                                "Failed to fetch orderbook"
                            );
                        }
                    }
                });
            }

            tokio::time::sleep(poll_interval).await;
        }
    }

    /// Start the WebSocket subscription for live orderbook updates.
    pub async fn start_websocket(
        &self,
        markets: Arc<RwLock<Vec<BinaryMarket>>>,
        tx: mpsc::Sender<BinaryOrderBook>,
    ) -> Result<(), MonitorError> {
        let current_markets = markets.read().await;

        // Build asset_id → market lookup and collect all token IDs.
        let mut token_to_market: HashMap<String, BinaryMarket> = HashMap::new();
        let mut all_asset_ids: Vec<String> = Vec::new();

        for market in current_markets.iter() {
            token_to_market.insert(market.yes_token_id.clone(), market.clone());
            token_to_market.insert(market.no_token_id.clone(), market.clone());
            all_asset_ids.push(market.yes_token_id.clone());
            all_asset_ids.push(market.no_token_id.clone());
        }
        drop(current_markets);

        if all_asset_ids.is_empty() {
            warn!("No markets to subscribe to via WebSocket");
            return Ok(());
        }

        info!(
            asset_count = all_asset_ids.len(),
            "Connecting to CLOB WebSocket"
        );

        let (ws_stream, _) = tokio_tungstenite::connect_async(&self.ws_url)
            .await
            .map_err(|e| MonitorError::WebSocket(format!("Connection failed: {e}")))?;

        let (mut write, mut read) = ws_stream.split();

        // Subscribe to all asset IDs.
        // Polymarket expects batched subscriptions.
        for chunk in all_asset_ids.chunks(100) {
            let subscribe_msg = serde_json::json!({
                "type": "subscribe",
                "assets_ids": chunk,
            });

            write
                .send(Message::Text(subscribe_msg.to_string()))
                .await
                .map_err(|e| MonitorError::WebSocket(format!("Send failed: {e}")))?;
        }

        info!("WebSocket subscribed, listening for orderbook updates");

        // Track partial books — we get updates per-token, need to combine into binary books.
        let mut latest_books: HashMap<String, TokenOrderBook> = HashMap::new();

        while let Some(msg) = read.next().await {
            let msg = match msg {
                Ok(Message::Text(text)) => text,
                Ok(Message::Ping(data)) => {
                    let _ = write.send(Message::Pong(data)).await;
                    continue;
                }
                Ok(Message::Close(_)) => {
                    warn!("WebSocket closed by server");
                    break;
                }
                Ok(_) => continue,
                Err(e) => {
                    error!(error = %e, "WebSocket read error");
                    break;
                }
            };

            let ws_msg: WsMessage = match serde_json::from_str(&msg) {
                Ok(m) => m,
                Err(_) => continue,
            };

            let asset_id = match ws_msg.asset_id.as_deref().or(ws_msg.market.as_deref()) {
                Some(id) => id.to_string(),
                None => continue,
            };

            let market = match token_to_market.get(&asset_id) {
                Some(m) => m,
                None => continue,
            };

            // Determine which side this token represents.
            let side = if asset_id == market.yes_token_id {
                Side::Yes
            } else {
                Side::No
            };

            // Update the token book if we got orderbook data.
            if let (Some(asks), Some(bids)) = (ws_msg.asks, ws_msg.bids) {
                let token_book = TokenOrderBook {
                    token_id: asset_id.clone(),
                    side,
                    asks: parse_price_levels(asks),
                    bids: parse_price_levels(bids),
                };

                latest_books.insert(asset_id.clone(), token_book);

                // Try to emit a complete binary book if we have both sides.
                let other_token_id = if side == Side::Yes {
                    &market.no_token_id
                } else {
                    &market.yes_token_id
                };

                if let Some(other_book) = latest_books.get(other_token_id) {
                    let (yes_book, no_book) = if side == Side::Yes {
                        (
                            latest_books.get(&asset_id).unwrap().clone(),
                            other_book.clone(),
                        )
                    } else {
                        (
                            other_book.clone(),
                            latest_books.get(&asset_id).unwrap().clone(),
                        )
                    };

                    let binary_book = BinaryOrderBook {
                        condition_id: market.condition_id.clone(),
                        yes_book,
                        no_book,
                        timestamp: Utc::now(),
                    };

                    if tx.send(binary_book).await.is_err() {
                        debug!("Channel closed, stopping WebSocket");
                        break;
                    }
                }
            }
        }

        Ok(())
    }

    /// Fetch the current USDC balance from the CLOB REST API.
    pub async fn fetch_balance(&self) -> Result<Decimal, MonitorError> {
        let url = format!("{}/balance", self.clob_url);
        let resp: ClobBalanceResponse = self.client.get(&url).send().await?.json().await?;

        let raw = resp
            .usdc_balance
            .or(resp.balance)
            .ok_or_else(|| MonitorError::Parse("No balance field in response".to_string()))?;

        Decimal::from_str(&raw)
            .map_err(|e| MonitorError::Parse(format!("Invalid balance value '{raw}': {e}")))
    }

    /// Start monitoring — dispatches to REST or WebSocket based on config.
    pub async fn start(
        &self,
        markets: Arc<RwLock<Vec<BinaryMarket>>>,
        tx: mpsc::Sender<BinaryOrderBook>,
    ) {
        if self.use_websocket {
            info!("Starting WebSocket monitor");
            loop {
                match self.start_websocket(markets.clone(), tx.clone()).await {
                    Ok(()) => {
                        warn!("WebSocket connection ended, reconnecting in 5s...");
                    }
                    Err(e) => {
                        error!(error = %e, "WebSocket error, reconnecting in 5s...");
                    }
                }
                tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
            }
        } else {
            info!("Starting REST polling monitor");
            self.start_rest_polling(markets, tx).await;
        }
    }
}

/// Parse raw price level strings into typed PriceLevels.
fn parse_price_levels(levels: Vec<ClobPriceLevel>) -> Vec<PriceLevel> {
    levels
        .into_iter()
        .filter_map(|l| {
            let price = Decimal::from_str(&l.price).ok()?;
            let size = Decimal::from_str(&l.size).ok()?;
            Some(PriceLevel { price, size })
        })
        .collect()
}
