use anyhow::Result;
use arb_config::{AppConfig, ExecutionMode};
use arb_executor::OrderExecutor;
use arb_inventory::{InventoryManager, RestingOrderBook};
use arb_monitor::PriceMonitor;
use arb_recorder::Recorder;
use arb_risk::RiskManager;
use arb_scanner::MarketScanner;
use arb_strategy::ArbitrageDetector;
use arb_types::{BinaryMarket, BinaryOrderBook, OrderState, RestingOrder, StrategyMode};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::{debug, error, info, warn};

#[tokio::main]
async fn main() -> Result<()> {
    // Load config.
    let config = AppConfig::load()?;

    // Initialize tracing.
    init_tracing(&config.logging);

    let strategy_mode = config.strategy.mode;
    let max_sweep_levels = config.strategy.max_sweep_levels;
    let is_live = matches!(config.execution.mode, ExecutionMode::Live);

    info!("Polymarket Arbitrage Bot starting");
    info!(
        clob_url = %config.polymarket.clob_url,
        chain_id = config.polymarket.chain_id,
        min_spread = %config.strategy.min_net_spread,
        max_order = %config.risk.max_order_size_usdc,
        strategy_mode = %strategy_mode,
        max_sweep_levels = max_sweep_levels,
        "Configuration loaded"
    );

    // Initialize components.
    let scanner = Arc::new(MarketScanner::new(&config));
    let monitor = Arc::new(PriceMonitor::new(&config));
    let detector = ArbitrageDetector::new(&config.strategy);
    let risk_manager = Arc::new(RiskManager::new(&config.risk));
    let inventory = Arc::new(InventoryManager::new(config.strategy.max_unpaired_hold_secs));
    let resting_orders = Arc::new(RestingOrderBook::new());

    // Telemetry recorder for the dashboard. No-op unless telemetry.enabled is
    // set and SUPABASE_URL / SUPABASE_SERVICE_KEY are present in the env.
    let recorder = Arc::new(Recorder::new(
        config.telemetry.enabled,
        is_live,
        strategy_mode.to_string(),
    ));

    // Determine execution mode.
    let executor: Arc<OrderExecutor> = match config.execution.mode {
        ExecutionMode::DryRun => {
            if std::env::var("POLYMARKET_PRIVATE_KEY").is_ok() {
                warn!(
                    "POLYMARKET_PRIVATE_KEY is set, but execution.mode=dry_run — live trading is disabled"
                );
            } else {
                info!("execution.mode=dry_run — running in dry-run mode");
            }
            Arc::new(OrderExecutor::new_dry_run(&config))
        }
        ExecutionMode::Live => {
            let key = AppConfig::load_private_key()?;
            let exec = OrderExecutor::new_live(&config, &key).await?;
            info!(
                rpc_url = %config.polymarket.polygon_rpc_url,
                "Verifying on-chain approvals before enabling live mode..."
            );
            if let Err(e) = exec
                .enforce_startup_allowances(
                    &config.polymarket.polygon_rpc_url,
                    config.risk.max_total_exposure_usdc,
                )
                .await
            {
                error!(
                    error = %e,
                    "Live mode blocked: on-chain approvals are not satisfied. \
                     Approve USDC.e and ConditionalTokens to both Polymarket CTF \
                     Exchange contracts on Polygon, then restart."
                );
                return Err(anyhow::anyhow!("startup allowance check failed: {}", e));
            }
            info!("execution.mode=live — live trading enabled");
            Arc::new(exec)
        }
    };

    // Initial market scan.
    info!("Scanning for active binary markets...");
    match scanner.refresh().await {
        Ok(count) => info!(count, "Initial market scan complete"),
        Err(e) => {
            error!(error = %e, "Failed to scan markets — continuing with empty list");
        }
    }

    let markets = scanner.markets();

    {
        let m = markets.read().await;
        if m.is_empty() {
            warn!("No markets discovered. The bot will wait for the next refresh.");
        }
    }

    // Build a lookup from condition_id → market for fast access in the main loop.
    let market_lookup = {
        let m = markets.read().await;
        let mut lookup = HashMap::new();
        for market in m.iter() {
            lookup.insert(market.condition_id.clone(), market.clone());
        }
        Arc::new(tokio::sync::RwLock::new(lookup))
    };

    // Channel for orderbook updates.
    let (tx, mut rx) = mpsc::channel::<BinaryOrderBook>(1000);

    // Spawn periodic market refresh.
    let scanner_clone = scanner.clone();
    let market_lookup_clone = market_lookup.clone();
    let refresh_interval = config.scanner.refresh_interval_secs;
    tokio::spawn(async move {
        let mut interval =
            tokio::time::interval(tokio::time::Duration::from_secs(refresh_interval));
        interval.tick().await;

        loop {
            interval.tick().await;
            match scanner_clone.refresh().await {
                Ok(count) => {
                    info!(count, "Periodic market refresh complete");
                    let markets_arc = scanner_clone.markets();
                    let m = markets_arc.read().await;
                    let mut lookup = market_lookup_clone.write().await;
                    lookup.clear();
                    for market in m.iter() {
                        lookup.insert(market.condition_id.clone(), market.clone());
                    }
                }
                Err(e) => {
                    warn!(error = %e, "Periodic market refresh failed");
                }
            }
        }
    });

    // Spawn periodic balance check.
    let risk_for_balance = risk_manager.clone();
    let monitor_for_balance = monitor.clone();
    let recorder_for_balance = recorder.clone();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(30));
        loop {
            interval.tick().await;
            match monitor_for_balance.fetch_balance().await {
                Ok(fetched) => risk_for_balance.update_balance(fetched).await,
                Err(e) => warn!(error = %e, "Failed to fetch balance from CLOB"),
            }
            let balance = risk_for_balance.balance().await;
            let exposure = risk_for_balance.total_exposure().await;
            let open_positions = risk_for_balance.open_positions().await;
            info!(
                balance = %balance,
                exposure = %exposure,
                "Balance check"
            );
            recorder_for_balance.record_snapshot(balance, exposure, open_positions);
        }
    });

    // Spawn the resting-order poller. Iterates resting orders, asks the CLOB
    // for current status, and transitions matched→inventory.record_leg_fill
    // (which then pairs with opposite-side legs when possible).
    let resting_for_poller = resting_orders.clone();
    let executor_for_poller = executor.clone();
    let inventory_for_poller = inventory.clone();
    let risk_for_poller = risk_manager.clone();
    let poll_interval_secs = config.monitor.resting_order_poll_interval_secs;
    tokio::spawn(async move {
        let mut interval =
            tokio::time::interval(tokio::time::Duration::from_secs(poll_interval_secs));
        interval.tick().await;
        loop {
            interval.tick().await;
            poll_resting_orders(
                &resting_for_poller,
                &executor_for_poller,
                &inventory_for_poller,
                &risk_for_poller,
            )
            .await;
        }
    });

    // Spawn stale leg monitor (60s interval).
    let inventory_for_stale = inventory.clone();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(60));
        loop {
            interval.tick().await;
            let stale = inventory_for_stale.find_stale_legs().await;
            for leg in &stale {
                warn!(
                    condition_id = %leg.condition_id,
                    side = %leg.side,
                    size = %leg.size,
                    avg_cost = %leg.avg_cost,
                    acquired_at = %leg.acquired_at,
                    "Stale unpaired leg detected"
                );
            }
        }
    });

    // Spawn price monitor.
    let monitor_markets = markets.clone();
    tokio::spawn(async move {
        monitor.start(monitor_markets, tx).await;
    });

    // Main event loop.
    info!(
        mode = %strategy_mode,
        "Entering main event loop — listening for orderbook updates..."
    );

    let mut opportunities_detected: u64 = 0;
    let mut books_processed: u64 = 0;

    while let Some(book) = rx.recv().await {
        books_processed += 1;

        if books_processed % 100 == 0 {
            let locked_profit = inventory.total_locked_profit().await;
            info!(
                books_processed,
                opportunities_detected,
                locked_profit = %locked_profit,
                "Processing stats"
            );
        }

        let market = {
            let lookup = market_lookup.read().await;
            match lookup.get(&book.condition_id) {
                Some(m) => m.clone(),
                None => continue,
            }
        };

        match strategy_mode {
            StrategyMode::Simultaneous => {
                if try_simultaneous(
                    &detector,
                    &book,
                    &market,
                    &risk_manager,
                    &executor,
                    &recorder,
                    max_sweep_levels,
                    &mut opportunities_detected,
                )
                .await
                {
                    continue;
                }
            }
            StrategyMode::Asymmetric => {
                try_asymmetric(
                    &detector,
                    &book,
                    &market,
                    &risk_manager,
                    &executor,
                    &recorder,
                    &inventory,
                    &resting_orders,
                    &mut opportunities_detected,
                )
                .await;
            }
            StrategyMode::Hybrid => {
                let found = try_simultaneous(
                    &detector,
                    &book,
                    &market,
                    &risk_manager,
                    &executor,
                    &recorder,
                    max_sweep_levels,
                    &mut opportunities_detected,
                )
                .await;
                if !found {
                    try_asymmetric(
                        &detector,
                        &book,
                        &market,
                        &risk_manager,
                        &executor,
                        &recorder,
                        &inventory,
                        &resting_orders,
                        &mut opportunities_detected,
                    )
                    .await;
                }
            }
        }
    }

    info!("Channel closed — shutting down");
    Ok(())
}

/// Try simultaneous strategy (sweep first if configured, then single-level fallback).
/// Returns true if an opportunity was found and dispatched.
async fn try_simultaneous(
    detector: &ArbitrageDetector,
    book: &BinaryOrderBook,
    market: &BinaryMarket,
    risk_manager: &Arc<RiskManager>,
    executor: &Arc<OrderExecutor>,
    recorder: &Arc<Recorder>,
    max_sweep_levels: usize,
    opportunities_detected: &mut u64,
) -> bool {
    // Try multi-level sweep first.
    if max_sweep_levels > 0 {
        if let Ok(sweep_opp) = detector.analyze_sweep(book, market) {
            *opportunities_detected += 1;
            let sweep_order = detector.to_sweep_execution_order(sweep_opp);
            match risk_manager.evaluate_sweep(sweep_order).await {
                Ok(approved) => {
                    let executor = executor.clone();
                    let risk_manager = risk_manager.clone();
                    let recorder = recorder.clone();
                    tokio::spawn(async move {
                        let result = executor.execute_sweep(approved).await;
                        if result.is_success() {
                            info!("Sweep trade executed successfully!");
                        }
                        risk_manager.record_execution(&result).await;
                        recorder.record_execution(&result);
                    });
                    return true;
                }
                Err(e) => {
                    info!(error = %e, "Risk check rejected sweep");
                }
            }
        }
    }

    // Fallback to single-level analysis.
    if let Ok(opportunity) = detector.analyze(book, market) {
        *opportunities_detected += 1;
        recorder.record_opportunity(&opportunity);
        let order = detector.to_execution_order(opportunity);
        match risk_manager.evaluate(order).await {
            Ok(approved_order) => {
                let executor = executor.clone();
                let risk_manager = risk_manager.clone();
                let recorder = recorder.clone();
                tokio::spawn(async move {
                    let result = executor.execute(approved_order).await;
                    if result.is_success() {
                        info!("Trade executed successfully!");
                    }
                    risk_manager.record_execution(&result).await;
                    recorder.record_execution(&result);
                });
                return true;
            }
            Err(e) => {
                info!(error = %e, "Risk check rejected opportunity");
            }
        }
    }

    false
}

/// Try asymmetric strategy — buy individual legs when price dips below target.
async fn try_asymmetric(
    detector: &ArbitrageDetector,
    book: &BinaryOrderBook,
    market: &BinaryMarket,
    risk_manager: &Arc<RiskManager>,
    executor: &Arc<OrderExecutor>,
    recorder: &Arc<Recorder>,
    inventory: &Arc<InventoryManager>,
    resting_orders: &Arc<RestingOrderBook>,
    opportunities_detected: &mut u64,
) {
    let snap = inventory.snapshot().await;

    let leg_orders = match detector.analyze_asymmetric(book, market, &snap) {
        Ok(orders) => orders,
        Err(_) => return,
    };

    for mut leg_order in leg_orders {
        *opportunities_detected += 1;

        // Count of filled-but-unpaired legs in this market — feeds the new
        // max_unpaired_legs_per_market cap. Phase 1's resting-order tracker
        // (#11) has landed; folding resting-order counts in here is a follow-up.
        let current_market_leg_count = snap
            .open_legs
            .get(&leg_order.condition_id)
            .map_or(0, |v| v.len());

        let approved_size = match risk_manager
            .evaluate_leg(&leg_order, current_market_leg_count)
            .await
        {
            Ok(size) => size,
            Err(e) => {
                info!(error = %e, side = %leg_order.side, "Risk check rejected leg");
                continue;
            }
        };

        leg_order.size = approved_size;

        let executor = executor.clone();
        let risk_manager = risk_manager.clone();
        let recorder = recorder.clone();
        let inventory = inventory.clone();
        let resting_orders = resting_orders.clone();

        tokio::spawn(async move {
            let result = executor.execute_leg(leg_order).await;

            match &result {
                arb_types::LegExecutionResult::Filled {
                    order,
                    fill_size,
                    fill_cost,
                    ..
                } => {
                    record_leg_fill_and_pair(
                        &inventory,
                        &risk_manager,
                        &order.condition_id,
                        &order.token_id,
                        order.side,
                        *fill_size,
                        *fill_cost,
                    )
                    .await;
                }
                arb_types::LegExecutionResult::Resting {
                    order,
                    order_id,
                    posted_at,
                } => {
                    info!(
                        condition_id = %order.condition_id,
                        side = %order.side,
                        order_id = %order_id,
                        size = %order.size,
                        target_price = %order.target_price,
                        "Leg resting on the book — poller will track"
                    );
                    resting_orders
                        .add(RestingOrder {
                            order_id: order_id.clone(),
                            condition_id: order.condition_id.clone(),
                            token_id: order.token_id.clone(),
                            side: order.side,
                            size: order.size,
                            price: order.target_price,
                            posted_at: *posted_at,
                        })
                        .await;
                }
                _ => {}
            }

            risk_manager.record_leg_execution(&result).await;
            recorder.record_leg_execution(&result);
        });
    }
}

/// Record a leg fill against inventory and credit any paired positions
/// against the risk manager's unpaired-exposure counter.
async fn record_leg_fill_and_pair(
    inventory: &Arc<InventoryManager>,
    risk_manager: &Arc<RiskManager>,
    condition_id: &str,
    token_id: &str,
    side: arb_types::Side,
    fill_size: rust_decimal::Decimal,
    fill_cost: rust_decimal::Decimal,
) {
    info!(
        condition_id = %condition_id,
        side = %side,
        fill_size = %fill_size,
        "Leg filled"
    );

    let avg_cost = if fill_size > rust_decimal::Decimal::ZERO {
        fill_cost / fill_size
    } else {
        rust_decimal::Decimal::ZERO
    };

    let pairs = inventory
        .record_leg_fill(condition_id, token_id, side, fill_size, avg_cost)
        .await;

    for pair in &pairs {
        let paired_cost = pair.yes_cost + pair.no_cost;
        risk_manager
            .record_pairing(&pair.condition_id, paired_cost)
            .await;
        info!(
            condition_id = %pair.condition_id,
            locked_profit = %pair.locked_profit,
            paired_size = %pair.paired_size,
            "Position paired"
        );
    }
}

/// One tick of the resting-order poller: for each resting order, fetch its
/// CLOB status and act on it.
///
/// - `Matched`: synthesize a `Filled` event for the inventory layer, then
///   remove from the resting book.
/// - `Cancelled`: just remove from the resting book.
/// - `Live` / `Unknown`: leave in place.
///
/// Partial fills (Live with `size_matched > 0`) are intentionally left for a
/// later phase; v0 records the fill only when the CLOB transitions the order
/// out of `Live`.
async fn poll_resting_orders(
    resting_orders: &Arc<RestingOrderBook>,
    executor: &Arc<OrderExecutor>,
    inventory: &Arc<InventoryManager>,
    risk_manager: &Arc<RiskManager>,
) {
    let orders = resting_orders.all().await;
    if orders.is_empty() {
        return;
    }
    debug!(count = orders.len(), "Polling resting orders");

    for order in orders {
        let status = match executor.get_order_status(&order.order_id).await {
            Ok(s) => s,
            Err(e) => {
                debug!(order_id = %order.order_id, error = %e, "Order status poll failed");
                continue;
            }
        };

        match status.state {
            OrderState::Matched => {
                let fill_size = if status.size_matched > rust_decimal::Decimal::ZERO {
                    status.size_matched
                } else {
                    order.size
                };
                let avg_price = status.avg_fill_price.unwrap_or(order.price);
                let fill_cost = avg_price * fill_size;
                record_leg_fill_and_pair(
                    inventory,
                    risk_manager,
                    &order.condition_id,
                    &order.token_id,
                    order.side,
                    fill_size,
                    fill_cost,
                )
                .await;
                resting_orders.remove(&order.order_id).await;
            }
            OrderState::Cancelled => {
                info!(order_id = %order.order_id, "Resting order cancelled — removing from book");
                resting_orders.remove(&order.order_id).await;
            }
            OrderState::Live | OrderState::Unknown => {}
        }
    }
}

fn init_tracing(config: &arb_config::LoggingConfig) {
    use tracing_subscriber::{fmt, EnvFilter};

    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new(&config.level));

    if config.json_output {
        fmt()
            .with_env_filter(filter)
            .json()
            .init();
    } else {
        fmt()
            .with_env_filter(filter)
            .with_target(true)
            .with_thread_ids(false)
            .init();
    }
}
