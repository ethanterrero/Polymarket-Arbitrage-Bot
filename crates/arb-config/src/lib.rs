use arb_types::StrategyMode;
use rust_decimal::Decimal;
use serde::Deserialize;
use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("Failed to load configuration: {0}")]
    Load(#[from] config::ConfigError),
    #[error("Missing environment variable: {0}")]
    MissingEnv(String),
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionMode {
    DryRun,
    Live,
}

fn default_execution_mode() -> ExecutionMode {
    ExecutionMode::DryRun
}

#[derive(Debug, Clone, Deserialize)]
pub struct ExecutionConfig {
    #[serde(default = "default_execution_mode")]
    pub mode: ExecutionMode,
}

/// Top-level application configuration.
#[derive(Debug, Clone, Deserialize)]
pub struct AppConfig {
    pub polymarket: PolymarketConfig,
    #[serde(default)]
    pub execution: ExecutionConfig,
    pub strategy: StrategyConfig,
    pub risk: RiskConfig,
    pub scanner: ScannerConfig,
    pub monitor: MonitorConfig,
    pub logging: LoggingConfig,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PolymarketConfig {
    pub clob_url: String,
    pub gamma_url: String,
    pub ws_url: String,
    pub chain_id: u64,
    /// Wallet address used to fetch USDC balance from the CLOB.
    /// Leave empty to skip live balance checks (bot runs with balance = 0).
    #[serde(default)]
    pub wallet_address: String,
    /// Polygon JSON-RPC endpoint used by the startup allowance check.
    /// Default is the public Polygon RPC; users hitting rate limits should
    /// supply their own (Alchemy/Infura/QuickNode/etc.).
    #[serde(default = "default_polygon_rpc_url")]
    pub polygon_rpc_url: String,
}

fn default_polygon_rpc_url() -> String {
    "https://polygon-rpc.com".to_string()
}

#[derive(Debug, Clone, Deserialize)]
pub struct StrategyConfig {
    pub min_net_spread: Decimal,
    pub base_fee_rate: Decimal,
    pub max_price_staleness_ms: u64,
    pub use_fok_orders: bool,
    #[serde(default)]
    pub mode: StrategyMode,
    #[serde(default)]
    pub max_sweep_levels: usize,
    #[serde(default = "default_min_sweep_profit")]
    pub min_sweep_profit_usdc: Decimal,
    #[serde(default = "default_asymmetric_target")]
    pub asymmetric_target_total_cost: Decimal,
    #[serde(default = "default_max_unpaired_hold")]
    pub max_unpaired_hold_secs: u64,
    #[serde(default = "default_max_unpaired_exposure")]
    pub max_unpaired_exposure_usdc: Decimal,
    #[serde(default = "default_max_unpaired_legs")]
    pub max_unpaired_legs_per_market: usize,
}

fn default_min_sweep_profit() -> Decimal {
    Decimal::new(50, 2) // 0.50
}

fn default_asymmetric_target() -> Decimal {
    Decimal::new(97, 2) // 0.97
}

fn default_max_unpaired_hold() -> u64 {
    3600
}

fn default_max_unpaired_exposure() -> Decimal {
    Decimal::new(200, 0) // 200.0
}

fn default_max_unpaired_legs() -> usize {
    3
}

#[derive(Debug, Clone, Deserialize)]
pub struct RiskConfig {
    pub max_order_size_usdc: Decimal,
    pub max_total_exposure_usdc: Decimal,
    pub min_reserve_balance_usdc: Decimal,
    pub per_market_cooldown_secs: u64,
    pub max_concurrent_positions: usize,
    #[serde(default = "default_risk_unpaired_exposure")]
    pub max_unpaired_exposure_usdc: Decimal,
    #[serde(default = "default_risk_unpaired_per_market")]
    pub max_unpaired_per_market_usdc: Decimal,
}

fn default_risk_unpaired_exposure() -> Decimal {
    Decimal::new(200, 0) // 200.0
}

fn default_risk_unpaired_per_market() -> Decimal {
    Decimal::new(50, 0) // 50.0
}

#[derive(Debug, Clone, Deserialize)]
pub struct ScannerConfig {
    pub refresh_interval_secs: u64,
    pub max_markets: usize,
    pub min_liquidity_usdc: Decimal,
    /// If non-empty, only include markets whose question contains at least one of these keywords (case-insensitive).
    /// Leave empty to scan all markets.
    #[serde(default)]
    pub market_keywords: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct MonitorConfig {
    pub use_websocket: bool,
    pub poll_interval_ms: u64,
    pub max_concurrent_requests: usize,
}

#[derive(Debug, Clone, Deserialize)]
pub struct LoggingConfig {
    pub level: String,
    pub json_output: bool,
}

impl Default for ExecutionConfig {
    fn default() -> Self {
        Self {
            mode: default_execution_mode(),
        }
    }
}

impl AppConfig {
    /// Load configuration from `config/default.toml`, with environment variable overrides.
    ///
    /// Environment variables use `__` as separator, e.g. `STRATEGY__MIN_NET_SPREAD`.
    pub fn load() -> Result<Self, ConfigError> {
        Self::load_from(None)
    }

    /// Load configuration from a specific path, or default if None.
    pub fn load_from(config_path: Option<PathBuf>) -> Result<Self, ConfigError> {
        // Load .env file if present (ignore errors — file may not exist).
        let _ = dotenvy::dotenv();

        let default_path = config_path.unwrap_or_else(|| PathBuf::from("config/default.toml"));

        let settings = config::Config::builder()
            .add_source(config::File::from(default_path).required(true))
            .add_source(
                config::Environment::default()
                    .separator("__")
                    .try_parsing(true),
            )
            .build()?;

        let config: AppConfig = settings.try_deserialize()?;
        Ok(config)
    }

    /// Load the private key from the POLYMARKET_PRIVATE_KEY environment variable.
    /// This is intentionally separate from config file loading for security.
    pub fn load_private_key() -> Result<String, ConfigError> {
        std::env::var("POLYMARKET_PRIVATE_KEY")
            .map_err(|_| ConfigError::MissingEnv("POLYMARKET_PRIVATE_KEY".to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn test_load_config_from_toml() {
        let dir = std::env::temp_dir().join("arb-config-test");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("test.toml");

        let toml_content = r#"
[polymarket]
clob_url = "https://clob.polymarket.com"
gamma_url = "https://gamma-api.polymarket.com"
ws_url = "wss://ws-subscriptions-clob.polymarket.com/ws/market"
chain_id = 137

[execution]
mode = "dry_run"

[strategy]
min_net_spread = "0.005"
base_fee_rate = "0.0"
max_price_staleness_ms = 2000
use_fok_orders = true

[risk]
max_order_size_usdc = "50.0"
max_total_exposure_usdc = "500.0"
min_reserve_balance_usdc = "50.0"
per_market_cooldown_secs = 10
max_concurrent_positions = 10

[scanner]
refresh_interval_secs = 300
max_markets = 200
min_liquidity_usdc = "100.0"

[monitor]
use_websocket = true
poll_interval_ms = 2000
max_concurrent_requests = 20

[logging]
level = "info"
json_output = false
"#;

        let mut f = std::fs::File::create(&path).unwrap();
        f.write_all(toml_content.as_bytes()).unwrap();

        let config = AppConfig::load_from(Some(path)).unwrap();
        assert_eq!(config.polymarket.chain_id, 137);
        assert_eq!(
            config.strategy.min_net_spread,
            Decimal::new(5, 3) // 0.005
        );
        assert_eq!(config.risk.max_concurrent_positions, 10);
        assert_eq!(config.scanner.max_markets, 200);
        assert!(config.monitor.use_websocket);
        assert_eq!(config.execution.mode, ExecutionMode::DryRun);

        std::fs::remove_dir_all(dir).ok();
    }
}
