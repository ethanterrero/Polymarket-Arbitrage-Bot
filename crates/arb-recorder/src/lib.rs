//! Telemetry sink for the arbitrage bot.
//!
//! `arb-recorder` turns the bot's in-memory execution results into rows in a
//! Supabase (Postgres) database so a frontend dashboard can visualize what the
//! bot is doing in real time. It is *purely additive*: the trading core
//! (`arb-risk`, `arb-executor`) does not depend on this crate, and every write
//! is fire-and-forget — a Supabase outage logs a warning and never blocks or
//! crashes the trading path.
//!
//! Two destinations, matching the two tables created in Supabase:
//! - `activity` — one append-only row per event (opportunity detected, dry-run,
//!   fill, partial fill, no-fill, error).
//! - `snapshots` — periodic bot state (balance, exposure, open positions) for
//!   time-series charts.
//!
//! Credentials come from the environment, never from the committed config:
//! - `SUPABASE_URL` — e.g. `https://<ref>.supabase.co`
//! - `SUPABASE_SERVICE_KEY` — the `service_role` key (bypasses RLS for inserts).

use std::sync::Arc;

use arb_types::{ArbitrageOpportunity, ExecutionResult, LegExecutionResult, Side};
use rust_decimal::prelude::ToPrimitive;
use rust_decimal::Decimal;
use serde::Serialize;
use serde_json::Value;
use tracing::{info, warn};

/// One row destined for the `activity` table.
///
/// Decimal fields are carried as `f64` so they serialize as JSON numbers that
/// PostgREST inserts cleanly into `numeric` columns. The lossless original is
/// preserved in `detail` (the full serialized result) for audit.
#[derive(Debug, Clone, Serialize)]
struct ActivityRow {
    kind: &'static str,
    is_live: bool,
    strategy_mode: Option<String>,
    condition_id: Option<String>,
    market_question: Option<String>,
    yes_price: Option<f64>,
    no_price: Option<f64>,
    size: Option<f64>,
    net_spread: Option<f64>,
    expected_profit: Option<f64>,
    total_cost: Option<f64>,
    detail: Option<Value>,
}

/// One row destined for the `snapshots` table.
#[derive(Debug, Clone, Serialize)]
struct SnapshotRow {
    is_live: bool,
    balance: Option<f64>,
    total_exposure: Option<f64>,
    open_positions: i64,
}

/// Resolved Supabase endpoint + credentials.
struct SupabaseSink {
    client: reqwest::Client,
    activity_url: String,
    snapshots_url: String,
    api_key: String,
}

/// Records bot activity to Supabase, or does nothing when telemetry is disabled.
///
/// Cheap to clone (the sink lives behind an `Arc`), so spawned execution tasks
/// can each hold their own handle.
#[derive(Clone)]
pub struct Recorder {
    sink: Option<Arc<SupabaseSink>>,
    is_live: bool,
    strategy_mode: String,
}

impl Recorder {
    /// Build a recorder from config + environment.
    ///
    /// When `enabled` is false, or `SUPABASE_URL` / `SUPABASE_SERVICE_KEY` are
    /// missing, returns a no-op recorder so the bot runs unchanged.
    pub fn new(enabled: bool, is_live: bool, strategy_mode: String) -> Self {
        if !enabled {
            return Self::disabled(is_live, strategy_mode);
        }
        let url = std::env::var("SUPABASE_URL").ok().filter(|s| !s.is_empty());
        let key = std::env::var("SUPABASE_SERVICE_KEY")
            .ok()
            .filter(|s| !s.is_empty());
        match (url, key) {
            (Some(url), Some(key)) => {
                info!(
                    endpoint = %url.trim_end_matches('/'),
                    "Telemetry enabled — recording activity to Supabase"
                );
                Self::with_supabase(url, key, is_live, strategy_mode)
            }
            _ => {
                warn!(
                    "telemetry.enabled=true but SUPABASE_URL / SUPABASE_SERVICE_KEY are not set \
                     — telemetry disabled"
                );
                Self::disabled(is_live, strategy_mode)
            }
        }
    }

    /// A recorder that drops every event.
    pub fn disabled(is_live: bool, strategy_mode: String) -> Self {
        Self {
            sink: None,
            is_live,
            strategy_mode,
        }
    }

    /// Build a recorder pointed at an explicit Supabase base URL + service key.
    /// Used by `new` and by tests (against a mock server).
    pub fn with_supabase(
        base_url: String,
        api_key: String,
        is_live: bool,
        strategy_mode: String,
    ) -> Self {
        let base = base_url.trim_end_matches('/');
        let sink = SupabaseSink {
            client: reqwest::Client::new(),
            activity_url: format!("{base}/rest/v1/activity"),
            snapshots_url: format!("{base}/rest/v1/snapshots"),
            api_key,
        };
        Self {
            sink: Some(Arc::new(sink)),
            is_live,
            strategy_mode,
        }
    }

    /// True when telemetry is actually wired to Supabase.
    pub fn is_active(&self) -> bool {
        self.sink.is_some()
    }

    /// Record an arbitrage opportunity at detection time — before risk checks,
    /// so the feed shows what the bot found even if risk later rejects it.
    pub fn record_opportunity(&self, opp: &ArbitrageOpportunity) {
        if self.sink.is_none() {
            return;
        }
        self.emit_activity(ActivityRow {
            kind: "opportunity_detected",
            is_live: self.is_live,
            strategy_mode: Some(self.strategy_mode.clone()),
            condition_id: Some(opp.market.condition_id.clone()),
            market_question: Some(opp.market.question.clone()),
            yes_price: opp.yes_ask_price.to_f64(),
            no_price: opp.no_ask_price.to_f64(),
            size: opp.max_size.to_f64(),
            net_spread: opp.net_spread.to_f64(),
            expected_profit: opp.expected_profit.to_f64(),
            total_cost: None,
            detail: serde_json::to_value(opp).ok(),
        });
    }

    /// Record the outcome of a simultaneous / sweep execution.
    pub fn record_execution(&self, result: &ExecutionResult) {
        if self.sink.is_none() {
            return;
        }
        let (order, kind, total_cost) = match result {
            ExecutionResult::FullFill {
                order, total_cost, ..
            } => (order, "full_fill", Some(*total_cost)),
            ExecutionResult::PartialFill {
                order, total_cost, ..
            } => (order, "partial_fill", Some(*total_cost)),
            ExecutionResult::NoFill { order, .. } => (order, "no_fill", None),
            ExecutionResult::Error { order, .. } => (order, "error", None),
            ExecutionResult::DryRun { order } => (order, "dry_run", None),
        };
        let opp = &order.opportunity;
        self.emit_activity(ActivityRow {
            kind,
            is_live: self.is_live,
            strategy_mode: Some(self.strategy_mode.clone()),
            condition_id: Some(opp.market.condition_id.clone()),
            market_question: Some(opp.market.question.clone()),
            yes_price: order.yes_price.to_f64(),
            no_price: order.no_price.to_f64(),
            size: order.approved_size.to_f64(),
            net_spread: opp.net_spread.to_f64(),
            expected_profit: opp.expected_profit.to_f64(),
            total_cost: total_cost.and_then(|d| d.to_f64()),
            detail: serde_json::to_value(result).ok(),
        });
    }

    /// Record the outcome of a single-leg (asymmetric) execution.
    pub fn record_leg_execution(&self, result: &LegExecutionResult) {
        if self.sink.is_none() {
            return;
        }
        let (order, kind, total_cost) = match result {
            LegExecutionResult::Filled {
                order, fill_cost, ..
            } => (order, "full_fill", Some(*fill_cost)),
            LegExecutionResult::NoFill { order, .. } => (order, "no_fill", None),
            LegExecutionResult::Error { order, .. } => (order, "error", None),
            LegExecutionResult::DryRun { order } => (order, "dry_run", None),
            // A GTC order resting on the book (maker mode, Phase 1+) is not
            // surfaced on the dashboard yet — that needs a dedicated 'resting'
            // activity kind plus a DB CHECK-constraint migration. Skip for now
            // rather than mislabel it as a fill/no-fill.
            // TODO: add a 'resting' event kind for maker-mode visibility.
            LegExecutionResult::Resting { .. } => return,
        };
        let (yes_price, no_price) = match order.side {
            Side::Yes => (order.target_price.to_f64(), None),
            Side::No => (None, order.target_price.to_f64()),
        };
        self.emit_activity(ActivityRow {
            kind,
            is_live: self.is_live,
            strategy_mode: Some(self.strategy_mode.clone()),
            condition_id: Some(order.condition_id.clone()),
            market_question: None,
            yes_price,
            no_price,
            size: order.size.to_f64(),
            net_spread: None,
            expected_profit: None,
            total_cost: total_cost.and_then(|d| d.to_f64()),
            detail: serde_json::to_value(result).ok(),
        });
    }

    /// Record a periodic snapshot of bot state for the time-series charts.
    pub fn record_snapshot(&self, balance: Decimal, total_exposure: Decimal, open_positions: usize) {
        let Some(sink) = self.sink.clone() else {
            return;
        };
        let row = SnapshotRow {
            is_live: self.is_live,
            balance: balance.to_f64(),
            total_exposure: total_exposure.to_f64(),
            open_positions: open_positions as i64,
        };
        let url = sink.snapshots_url.clone();
        match serde_json::to_value(&row) {
            Ok(value) => spawn_insert(sink, url, value),
            Err(e) => warn!(error = %e, "failed to serialize snapshot row"),
        }
    }

    fn emit_activity(&self, row: ActivityRow) {
        let Some(sink) = self.sink.clone() else {
            return;
        };
        let url = sink.activity_url.clone();
        match serde_json::to_value(&row) {
            Ok(value) => spawn_insert(sink, url, value),
            Err(e) => warn!(error = %e, "failed to serialize activity row"),
        }
    }
}

/// Fire-and-forget POST to a PostgREST endpoint. Errors are logged, never
/// propagated — telemetry must never affect the trading path.
fn spawn_insert(sink: Arc<SupabaseSink>, url: String, body: Value) {
    tokio::spawn(async move {
        let res = sink
            .client
            .post(&url)
            .header("apikey", &sink.api_key)
            .header("authorization", format!("Bearer {}", sink.api_key))
            .header("content-type", "application/json")
            .header("prefer", "return=minimal")
            .json(&body)
            .send()
            .await;
        match res {
            Ok(r) if r.status().is_success() => {}
            Ok(r) => warn!(status = %r.status(), url = %url, "telemetry insert rejected"),
            Err(e) => warn!(error = %e, url = %url, "telemetry insert failed"),
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use arb_types::{BinaryMarket, ExecutionOrder};
    use chrono::Utc;
    use httpmock::Method::POST;
    use httpmock::MockServer;
    use rust_decimal_macros::dec;
    use std::time::Duration;
    use uuid::Uuid;

    fn sample_market() -> BinaryMarket {
        BinaryMarket {
            condition_id: "0xcond".to_string(),
            yes_token_id: "yes123".to_string(),
            no_token_id: "no456".to_string(),
            question: "Will it rain tomorrow?".to_string(),
            slug: "rain-tomorrow".to_string(),
            active: true,
            liquidity: Some(dec!(1000)),
            neg_risk: false,
            fee_rate_bps: 0,
            min_tick_size: dec!(0.01),
            volume_24h: None,
            end_date: None,
        }
    }

    fn sample_opportunity() -> ArbitrageOpportunity {
        ArbitrageOpportunity {
            id: Uuid::nil(),
            market: sample_market(),
            yes_ask_price: dec!(0.48),
            no_ask_price: dec!(0.49),
            yes_ask_size: dec!(100),
            no_ask_size: dec!(100),
            gross_spread: dec!(0.03),
            net_spread: dec!(0.03),
            max_size: dec!(100),
            expected_profit: dec!(3.0),
            detected_at: Utc::now(),
        }
    }

    fn sample_execution_order() -> ExecutionOrder {
        ExecutionOrder {
            opportunity: sample_opportunity(),
            approved_size: dec!(50),
            yes_price: dec!(0.48),
            no_price: dec!(0.49),
            use_fok: true,
        }
    }

    #[tokio::test]
    async fn disabled_recorder_emits_nothing() {
        // No sink → calls are inert and never panic.
        let rec = Recorder::disabled(false, "simultaneous".to_string());
        assert!(!rec.is_active());
        rec.record_opportunity(&sample_opportunity());
        rec.record_execution(&ExecutionResult::DryRun {
            order: sample_execution_order(),
        });
        rec.record_snapshot(dec!(100), dec!(0), 0);
        // Nothing to assert beyond "did not panic"; give any (non-existent)
        // spawned tasks a moment in case of regressions.
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    #[tokio::test]
    async fn new_without_env_is_disabled() {
        // enabled=true but env not guaranteed set in test → must degrade to noop,
        // not panic. (We don't set the env here, so it should be inactive.)
        std::env::remove_var("SUPABASE_URL");
        std::env::remove_var("SUPABASE_SERVICE_KEY");
        let rec = Recorder::new(true, false, "simultaneous".to_string());
        assert!(!rec.is_active());
    }

    #[tokio::test]
    async fn posts_opportunity_to_activity_endpoint() {
        let server = MockServer::start_async().await;
        let mock = server
            .mock_async(|when, then| {
                when.method(POST).path("/rest/v1/activity");
                then.status(201);
            })
            .await;

        let rec = Recorder::with_supabase(
            server.base_url(),
            "service-key".to_string(),
            false,
            "simultaneous".to_string(),
        );
        rec.record_opportunity(&sample_opportunity());

        // Insert is fire-and-forget; wait for the spawned task to land.
        tokio::time::sleep(Duration::from_millis(300)).await;
        mock.assert_async().await;
    }

    #[tokio::test]
    async fn posts_dry_run_execution_with_expected_fields() {
        let server = MockServer::start_async().await;
        let mock = server
            .mock_async(|when, then| {
                when.method(POST)
                    .path("/rest/v1/activity")
                    .body_includes(r#""kind":"dry_run""#)
                    .body_includes(r#""condition_id":"0xcond""#)
                    .body_includes(r#""market_question":"Will it rain tomorrow?""#);
                then.status(201);
            })
            .await;

        let rec = Recorder::with_supabase(
            server.base_url(),
            "service-key".to_string(),
            false,
            "simultaneous".to_string(),
        );
        rec.record_execution(&ExecutionResult::DryRun {
            order: sample_execution_order(),
        });

        tokio::time::sleep(Duration::from_millis(300)).await;
        mock.assert_async().await;
    }

    #[tokio::test]
    async fn posts_snapshot_to_snapshots_endpoint() {
        let server = MockServer::start_async().await;
        let mock = server
            .mock_async(|when, then| {
                when.method(POST)
                    .path("/rest/v1/snapshots")
                    .body_includes(r#""open_positions":2"#)
                    .body_includes(r#""is_live":false"#);
                then.status(201);
            })
            .await;

        let rec = Recorder::with_supabase(
            server.base_url(),
            "service-key".to_string(),
            false,
            "simultaneous".to_string(),
        );
        rec.record_snapshot(dec!(250.5), dec!(40), 2);

        tokio::time::sleep(Duration::from_millis(300)).await;
        mock.assert_async().await;
    }
}
