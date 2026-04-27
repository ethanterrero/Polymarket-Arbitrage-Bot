//! Per-token CLOB fee rate fetch with in-memory cache.
//!
//! Mirrors `Polymarket/py-clob-client`'s `get_fee_rate_bps` flow:
//! `GET /fee-rate?token_id=<id>` → `{"base_fee": <bps>}`, cached client-side.
//! `base_fee` is per-token and currently 0 for the vast majority of markets,
//! but the on-chain `Order.feeRateBps` is part of the EIP-712 digest, so any
//! market with a non-zero fee will silently reject orders unless this value
//! is correct.

use std::collections::HashMap;
use tokio::sync::RwLock;

#[derive(Debug, thiserror::Error)]
pub enum FeeRateError {
    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),
    #[error("CLOB returned {status} for /fee-rate (token_id={token_id})")]
    Status { status: u16, token_id: String },
}

/// Per-`token_id` cache of `base_fee` rates in basis points.
///
/// Keyed by token_id (not condition_id) to match upstream behavior — each
/// outcome token can in principle have its own fee rate.
pub struct FeeRateCache {
    inner: RwLock<HashMap<String, u32>>,
}

impl FeeRateCache {
    pub fn new() -> Self {
        Self {
            inner: RwLock::new(HashMap::new()),
        }
    }

    /// Resolve the fee rate for `token_id`. Hits cache first; on miss, fetches
    /// `GET <clob_url>/fee-rate?token_id=<token_id>` and caches the result.
    /// A 200 response without a `base_fee` key returns `0` and caches `0`,
    /// matching `py-clob-client`'s `result.get("base_fee") or 0`.
    pub async fn get(
        &self,
        client: &reqwest::Client,
        clob_url: &str,
        token_id: &str,
    ) -> Result<u32, FeeRateError> {
        if let Some(rate) = self.inner.read().await.get(token_id) {
            return Ok(*rate);
        }
        let url = format!("{}/fee-rate?token_id={}", clob_url, token_id);
        let resp = client.get(&url).send().await?;
        let status = resp.status();
        if !status.is_success() {
            return Err(FeeRateError::Status {
                status: status.as_u16(),
                token_id: token_id.to_string(),
            });
        }
        let json: serde_json::Value = resp.json().await?;
        let rate = json
            .get("base_fee")
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as u32;
        self.inner.write().await.insert(token_id.to_string(), rate);
        Ok(rate)
    }

    #[cfg(test)]
    pub async fn insert(&self, token_id: &str, rate: u32) {
        self.inner.write().await.insert(token_id.to_string(), rate);
    }
}

impl Default for FeeRateCache {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use httpmock::Method::GET;
    use httpmock::MockServer;

    #[tokio::test]
    async fn fetches_then_caches_token_fee_rate() {
        let server = MockServer::start();
        let m = server.mock(|when, then| {
            when.method(GET)
                .path("/fee-rate")
                .query_param("token_id", "tok-1");
            then.status(200)
                .header("content-type", "application/json")
                .body(r#"{"base_fee":12}"#);
        });

        let cache = FeeRateCache::new();
        let client = reqwest::Client::new();

        let r1 = cache.get(&client, &server.url(""), "tok-1").await.unwrap();
        let r2 = cache.get(&client, &server.url(""), "tok-1").await.unwrap();

        assert_eq!(r1, 12);
        assert_eq!(r2, 12);
        // Second call hits the cache, not the network.
        m.assert_calls(1);
    }

    #[tokio::test]
    async fn missing_base_fee_key_returns_zero() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method(GET).path("/fee-rate");
            then.status(200)
                .header("content-type", "application/json")
                .body(r#"{}"#);
        });

        let cache = FeeRateCache::new();
        let client = reqwest::Client::new();
        let r = cache.get(&client, &server.url(""), "tok-x").await.unwrap();
        assert_eq!(r, 0);
    }

    #[tokio::test]
    async fn http_error_propagates_and_does_not_cache() {
        let server = MockServer::start();
        let m = server.mock(|when, then| {
            when.method(GET).path("/fee-rate");
            then.status(500).body("oops");
        });

        let cache = FeeRateCache::new();
        let client = reqwest::Client::new();
        let err = cache
            .get(&client, &server.url(""), "tok-err")
            .await
            .unwrap_err();
        assert!(matches!(err, FeeRateError::Status { status: 500, .. }));

        // Retry should hit the server again — no negative caching.
        let _ = cache.get(&client, &server.url(""), "tok-err").await;
        m.assert_calls(2);
    }

    #[tokio::test]
    async fn distinct_tokens_get_distinct_rates() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method(GET)
                .path("/fee-rate")
                .query_param("token_id", "yes-tok");
            then.status(200).body(r#"{"base_fee":7}"#);
        });
        server.mock(|when, then| {
            when.method(GET)
                .path("/fee-rate")
                .query_param("token_id", "no-tok");
            then.status(200).body(r#"{"base_fee":13}"#);
        });

        let cache = FeeRateCache::new();
        let client = reqwest::Client::new();
        let yes = cache.get(&client, &server.url(""), "yes-tok").await.unwrap();
        let no = cache.get(&client, &server.url(""), "no-tok").await.unwrap();
        assert_eq!(yes, 7);
        assert_eq!(no, 13);
    }
}
