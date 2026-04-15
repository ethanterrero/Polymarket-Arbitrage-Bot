use anyhow::Result;
use arb_config::AppConfig;
use arb_executor::OrderExecutor;
use arb_inventory::InventoryManager;
use arb_monitor::PriceMonitor;
use arb_risk::RiskManager;
use arb_scanner::MarketScanner;
use arb_strategy::ArbitrageDetector;
use arb_types::{BinaryMarket, BinaryOrderBook, StrategyMode};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::{error, info, warn};

#[tokio::main]
async fn main() -> Result<()> {
    // Load config.
    let config = AppConfig::load()?;

    // Initialize tracing.
    init_tracing(&config.logging);

    let strategy_mode = config.strategy.mode;
    let max_sweep_levels = config.strategy.max_sweep_levels;

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

    // Determine execution mode.
    let executor = match AppConfig::load_private_key() {
        Ok(_key) => {
            info!("Private key found, but using DRY RUN mode until API key derivation is implemented");
            Arc::new(OrderExecutor::new_dry_run(&config))
        }
        Err(_) => {
            info!("No private key found — running in DETECT-ONLY mode");
            Arc::new(OrderExecutor::new_dry_run(&config))
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
            info!(
                balance = %balance,
                exposure = %exposure,
                "Balance check"
            );
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
                    &inventory,
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
                        &inventory,
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
                    tokio::spawn(async move {
                        let result = executor.execute_sweep(approved).await;
                        if result.is_success() {
                            info!("Sweep trade executed successfully!");
                        }
                        risk_manager.record_execution(&result).await;
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
        let order = detector.to_execution_order(opportunity);
        match risk_manager.evaluate(order).await {
            Ok(approved_order) => {
                let executor = executor.clone();
                let risk_manager = risk_manager.clone();
                tokio::spawn(async move {
                    let result = executor.execute(approved_order).await;
                    if result.is_success() {
                        info!("Trade executed successfully!");
                    }
                    risk_manager.record_execution(&result).await;
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
    inventory: &Arc<InventoryManager>,
    opportunities_detected: &mut u64,
) {
    let snap = inventory.snapshot().await;

    let leg_orders = match detector.analyze_asymmetric(book, market, &snap) {
        Ok(orders) => orders,
        Err(_) => return,
    };

    for mut leg_order in leg_orders {
        *opportunities_detected += 1;

        // Risk evaluation returns the approved size.
        let approved_size = match risk_manager.evaluate_leg(&leg_order).await {
            Ok(size) => size,
            Err(e) => {
                info!(error = %e, side = %leg_order.side, "Risk check rejected leg");
                continue;
            }
        };

        leg_order.size = approved_size;

        let executor = executor.clone();
        let risk_manager = risk_manager.clone();
        let inventory = inventory.clone();

        tokio::spawn(async move {
            let result = executor.execute_leg(leg_order).await;

            if let arb_types::LegExecutionResult::Filled {
                ref order,
                fill_size,
                fill_cost,
            } = result
            {
                info!(
                    condition_id = %order.condition_id,
                    side = %order.side,
                    fill_size = %fill_size,
                    "Leg filled"
                );

                let avg_cost = if fill_size > rust_decimal::Decimal::ZERO {
                    fill_cost / fill_size
                } else {
                    rust_decimal::Decimal::ZERO
                };

                let pairs = inventory
                    .record_leg_fill(
                        &order.condition_id,
                        &order.token_id,
                        order.side,
                        fill_size,
                        avg_cost,
                    )
                    .await;

                // Reduce unpaired exposure for any new pairings.
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

            risk_manager.record_leg_execution(&result).await;
        });
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
