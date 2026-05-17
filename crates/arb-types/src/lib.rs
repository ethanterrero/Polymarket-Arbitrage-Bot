use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Metadata for a binary prediction market on Polymarket.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BinaryMarket {
    /// The on-chain condition ID for this market.
    pub condition_id: String,
    /// CLOB token ID for the YES outcome.
    pub yes_token_id: String,
    /// CLOB token ID for the NO outcome.
    pub no_token_id: String,
    /// Human-readable market question.
    pub question: String,
    /// URL slug for the market page.
    pub slug: String,
    /// Whether the market is currently active.
    pub active: bool,
    /// Estimated total liquidity in USDC.
    pub liquidity: Option<Decimal>,
    /// Whether this market settles via the Neg Risk CTF Exchange. Determines
    /// which `verifyingContract` is used in the EIP-712 domain when signing
    /// orders for this market.
    pub neg_risk: bool,
    /// Per-market trading fee in basis points (1 bp = 0.01%). Currently
    /// derived from the global `base_fee_rate` config since Gamma does not
    /// expose this per-market; the CLOB API exposes a per-market rate which
    /// can replace this default once wired in.
    pub fee_rate_bps: u32,
    /// Minimum order price increment for this market (e.g. 0.01 or 0.001).
    /// Limit prices on the CLOB must be a multiple of this.
    pub min_tick_size: Decimal,
}

/// A single price level in an orderbook (price + available size).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PriceLevel {
    /// Price in USDC (0.0 to 1.0 range for prediction markets).
    pub price: Decimal,
    /// Available size at this price level.
    pub size: Decimal,
}

/// Side of the orderbook.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Side {
    Yes,
    No,
}

impl std::fmt::Display for Side {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Side::Yes => write!(f, "YES"),
            Side::No => write!(f, "NO"),
        }
    }
}

/// Full orderbook snapshot for a single token (one side of a binary market).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenOrderBook {
    pub token_id: String,
    pub side: Side,
    /// Ask levels sorted by price ascending (best ask first).
    pub asks: Vec<PriceLevel>,
    /// Bid levels sorted by price descending (best bid first).
    pub bids: Vec<PriceLevel>,
}

/// Combined orderbook snapshot for both YES and NO sides of a binary market.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BinaryOrderBook {
    pub condition_id: String,
    pub yes_book: TokenOrderBook,
    pub no_book: TokenOrderBook,
    /// Timestamp when this snapshot was captured.
    pub timestamp: DateTime<Utc>,
}

impl BinaryOrderBook {
    /// Returns the best (lowest) ask price for the YES side, if any.
    pub fn best_yes_ask(&self) -> Option<&PriceLevel> {
        self.yes_book.asks.first()
    }

    /// Returns the best (lowest) ask price for the NO side, if any.
    pub fn best_no_ask(&self) -> Option<&PriceLevel> {
        self.no_book.asks.first()
    }

    /// Returns the best (highest) bid price for the YES side, if any.
    pub fn best_yes_bid(&self) -> Option<&PriceLevel> {
        self.yes_book.bids.first()
    }

    /// Returns the best (highest) bid price for the NO side, if any.
    pub fn best_no_bid(&self) -> Option<&PriceLevel> {
        self.no_book.bids.first()
    }

    /// Age of this orderbook snapshot in milliseconds.
    pub fn age_ms(&self) -> i64 {
        (Utc::now() - self.timestamp).num_milliseconds()
    }
}

/// A detected arbitrage opportunity in a binary market.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArbitrageOpportunity {
    /// Unique ID for this opportunity.
    pub id: Uuid,
    /// The market where the opportunity was found.
    pub market: BinaryMarket,
    /// Best YES ask price used in detection.
    pub yes_ask_price: Decimal,
    /// Best NO ask price used in detection.
    pub no_ask_price: Decimal,
    /// Available size at the YES ask.
    pub yes_ask_size: Decimal,
    /// Available size at the NO ask.
    pub no_ask_size: Decimal,
    /// Gross spread: 1.0 - (yes_ask + no_ask). Positive means potential profit.
    pub gross_spread: Decimal,
    /// Net spread after estimated fees.
    pub net_spread: Decimal,
    /// Maximum executable size (min of both sides' liquidity).
    pub max_size: Decimal,
    /// Expected profit if fully filled: net_spread * max_size.
    pub expected_profit: Decimal,
    /// When this opportunity was detected.
    pub detected_at: DateTime<Utc>,
}

/// A risk-approved order ready for execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionOrder {
    /// The underlying opportunity.
    pub opportunity: ArbitrageOpportunity,
    /// Approved size (may be less than opportunity's max_size due to risk limits).
    pub approved_size: Decimal,
    /// Price to use for the YES buy order.
    pub yes_price: Decimal,
    /// Price to use for the NO buy order.
    pub no_price: Decimal,
    /// Whether to use Fill-or-Kill order type.
    pub use_fok: bool,
}

/// Result of attempting to execute an arbitrage trade.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ExecutionResult {
    /// Both legs filled completely.
    FullFill {
        order: ExecutionOrder,
        yes_fill_size: Decimal,
        no_fill_size: Decimal,
        total_cost: Decimal,
    },
    /// One or both legs partially filled.
    PartialFill {
        order: ExecutionOrder,
        yes_fill_size: Decimal,
        no_fill_size: Decimal,
        total_cost: Decimal,
        /// Description of what happened.
        detail: String,
    },
    /// Neither leg filled (e.g., prices moved).
    NoFill {
        order: ExecutionOrder,
        reason: String,
    },
    /// Execution error.
    Error {
        order: ExecutionOrder,
        error: String,
    },
    /// Dry run — opportunity logged but not executed.
    DryRun {
        order: ExecutionOrder,
    },
}

impl ExecutionResult {
    pub fn is_success(&self) -> bool {
        matches!(self, ExecutionResult::FullFill { .. })
    }

    pub fn filled_size(&self) -> Decimal {
        match self {
            ExecutionResult::FullFill {
                yes_fill_size,
                no_fill_size,
                ..
            } => (*yes_fill_size).min(*no_fill_size),
            ExecutionResult::PartialFill {
                yes_fill_size,
                no_fill_size,
                ..
            } => (*yes_fill_size).min(*no_fill_size),
            _ => Decimal::ZERO,
        }
    }

    pub fn total_cost(&self) -> Decimal {
        match self {
            ExecutionResult::FullFill { total_cost, .. } => *total_cost,
            ExecutionResult::PartialFill { total_cost, .. } => *total_cost,
            _ => Decimal::ZERO,
        }
    }
}

// ─── Multi-Level Sweep Types ───────────────────────────────────────────────

/// Levels consumed on one side of the book during a sweep.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SweepPlan {
    /// Individual price levels consumed.
    pub levels: Vec<PriceLevel>,
    /// Total size across all consumed levels.
    pub total_size: Decimal,
    /// Volume-weighted average price across consumed levels.
    pub vwap: Decimal,
    /// Worst (highest) price in the consumed levels.
    pub worst_price: Decimal,
}

/// A multi-level arbitrage opportunity spanning multiple book levels.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SweepOpportunity {
    pub id: Uuid,
    pub market: BinaryMarket,
    pub yes_sweep: SweepPlan,
    pub no_sweep: SweepPlan,
    /// Size matched between both sides (min of cumulative sizes).
    pub matched_size: Decimal,
    /// Net spread at the matched size after fees.
    pub net_spread: Decimal,
    /// Expected profit: net_spread * matched_size.
    pub expected_profit: Decimal,
    pub detected_at: DateTime<Utc>,
}

/// A risk-approved sweep order ready for execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SweepExecutionOrder {
    pub opportunity: SweepOpportunity,
    /// Approved size (may be capped by risk).
    pub approved_size: Decimal,
    /// Limit price for the YES side (worst_price from sweep plan).
    pub yes_limit_price: Decimal,
    /// Limit price for the NO side (worst_price from sweep plan).
    pub no_limit_price: Decimal,
    pub use_fok: bool,
}

// ─── Strategy Mode ─────────────────────────────────────────────────────────

/// Operating mode for the arbitrage strategy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum StrategyMode {
    /// Buy YES+NO simultaneously (original behavior).
    Simultaneous,
    /// Buy each side independently when prices dip, pairing over time.
    Asymmetric,
    /// Try simultaneous first, fall back to asymmetric.
    Hybrid,
}

impl Default for StrategyMode {
    fn default() -> Self {
        Self::Simultaneous
    }
}

impl std::fmt::Display for StrategyMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Simultaneous => write!(f, "simultaneous"),
            Self::Asymmetric => write!(f, "asymmetric"),
            Self::Hybrid => write!(f, "hybrid"),
        }
    }
}

// ─── Asymmetric / Inventory Types ──────────────────────────────────────────

/// An unpaired leg held in inventory.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenLeg {
    pub id: Uuid,
    pub condition_id: String,
    pub token_id: String,
    pub side: Side,
    pub size: Decimal,
    pub avg_cost: Decimal,
    pub acquired_at: DateTime<Utc>,
}

/// A completed pair with locked-in profit.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PairedPosition {
    pub id: Uuid,
    pub condition_id: String,
    pub paired_size: Decimal,
    pub yes_cost: Decimal,
    pub no_cost: Decimal,
    pub locked_profit: Decimal,
    pub paired_at: DateTime<Utc>,
}

/// Direction of a single-leg order. Buy means we pay USDC and receive
/// outcome tokens; Sell is the inverse and is used by Phase 4's stale-leg
/// unwinder to close unpaired inventory.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TradeDirection {
    Buy,
    Sell,
}

impl Default for TradeDirection {
    fn default() -> Self {
        Self::Buy
    }
}

impl std::fmt::Display for TradeDirection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Buy => write!(f, "BUY"),
            Self::Sell => write!(f, "SELL"),
        }
    }
}

/// Instruction to buy or sell a single side.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LegOrder {
    pub condition_id: String,
    pub token_id: String,
    pub side: Side,
    pub size: Decimal,
    pub target_price: Decimal,
    pub use_fok: bool,
    /// Whether this market uses the Neg Risk exchange variant (affects EIP-712 domain).
    pub neg_risk: bool,
    /// Trading fee in basis points (encoded into the signed order struct).
    pub fee_rate_bps: u32,
    /// Minimum order price increment for this market (used for quantization).
    pub min_tick_size: Decimal,
    /// Buy or Sell. Defaults to Buy for backward compatibility with the
    /// asymmetric strategy's open-leg path; Phase 4's unwinder sets this to
    /// Sell to close unpaired inventory.
    #[serde(default)]
    pub direction: TradeDirection,
}

/// Result of executing a single-leg buy.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum LegExecutionResult {
    /// Order was matched immediately (FOK fill, or GTC that crossed the book).
    Filled {
        order: LegOrder,
        order_id: Option<String>,
        fill_size: Decimal,
        fill_cost: Decimal,
    },
    /// GTC order accepted by the CLOB and now resting on the book. Phase 1+ owns
    /// polling this id for fill status and pairing it through the inventory.
    Resting {
        order: LegOrder,
        order_id: String,
        posted_at: DateTime<Utc>,
    },
    NoFill {
        order: LegOrder,
        reason: String,
    },
    Error {
        order: LegOrder,
        error: String,
    },
    DryRun {
        order: LegOrder,
    },
}

impl LegExecutionResult {
    pub fn is_filled(&self) -> bool {
        matches!(self, Self::Filled { .. })
    }

    pub fn is_resting(&self) -> bool {
        matches!(self, Self::Resting { .. })
    }

    pub fn fill_size(&self) -> Decimal {
        match self {
            Self::Filled { fill_size, .. } => *fill_size,
            _ => Decimal::ZERO,
        }
    }

    pub fn fill_cost(&self) -> Decimal {
        match self {
            Self::Filled { fill_cost, .. } => *fill_cost,
            _ => Decimal::ZERO,
        }
    }
}

/// A GTC order posted to the CLOB that has not yet fully filled or cancelled.
/// Tracked in the `RestingOrderBook` so the poller can transition it to a
/// fill via `InventoryManager::record_leg_fill` when the CLOB reports a match.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RestingOrder {
    pub order_id: String,
    pub condition_id: String,
    pub token_id: String,
    pub side: Side,
    pub size: Decimal,
    pub price: Decimal,
    pub posted_at: DateTime<Utc>,
}

/// Status of a single order on the CLOB, as returned by `GET /order/{id}`.
/// We map the CLOB's free-form `status` string into this enum so callers don't
/// have to string-compare; unknown values become `Unknown` and are logged but
/// not acted on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum OrderState {
    /// Order is on the book, awaiting fills.
    Live,
    /// Order has fully matched.
    Matched,
    /// Order was cancelled (by us or by the CLOB).
    Cancelled,
    /// Status string the CLOB returned that we don't recognize.
    Unknown,
}

/// Polled status of a CLOB order. `size_matched` is cumulative across all
/// fills so far; `avg_fill_price` is what the CLOB reports as the realized
/// price (None when there are no fills yet).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrderStatus {
    pub order_id: String,
    pub state: OrderState,
    pub size_matched: Decimal,
    pub original_size: Decimal,
    pub avg_fill_price: Option<Decimal>,
}

/// Read-only snapshot of inventory state, passed to strategy to avoid circular deps.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct InventorySnapshot {
    /// Open (unpaired) legs per market. Key = condition_id.
    pub open_legs: std::collections::HashMap<String, Vec<OpenLeg>>,
    /// Total USDC exposure in unpaired legs.
    pub total_unpaired_exposure: Decimal,
}
