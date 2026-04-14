use arb_types::{InventorySnapshot, OpenLeg, PairedPosition, Side};
use chrono::Utc;
use rust_decimal::Decimal;
use std::collections::HashMap;
use tokio::sync::RwLock;
use tracing::info;
use uuid::Uuid;

/// Manages unpaired leg inventory and pairing logic.
pub struct InventoryManager {
    state: RwLock<InventoryState>,
    max_unpaired_hold_secs: u64,
}

#[derive(Debug, Default)]
struct InventoryState {
    /// Open (unpaired) legs, keyed by condition_id.
    open_legs: HashMap<String, Vec<OpenLeg>>,
    /// Completed pairs.
    paired_positions: Vec<PairedPosition>,
}

impl InventoryManager {
    pub fn new(max_unpaired_hold_secs: u64) -> Self {
        Self {
            state: RwLock::new(InventoryState::default()),
            max_unpaired_hold_secs,
        }
    }

    /// Record a filled leg and attempt to pair with opposite-side inventory.
    ///
    /// Returns any newly created paired positions.
    pub async fn record_leg_fill(
        &self,
        condition_id: &str,
        token_id: &str,
        side: Side,
        fill_size: Decimal,
        avg_cost: Decimal,
    ) -> Vec<PairedPosition> {
        let mut state = self.state.write().await;

        let opposite = match side {
            Side::Yes => Side::No,
            Side::No => Side::Yes,
        };

        // All work that touches `legs` (a mutable ref into `state.open_legs`) is
        // scoped into this block. When the block ends, `legs` is dropped and the
        // borrow on `state.open_legs` is released — making it safe to mutably
        // access `state.paired_positions` afterwards.
        let (new_pairs, remaining) = {
            let legs = state
                .open_legs
                .entry(condition_id.to_string())
                .or_default();

            // Find opposite-side legs to pair with, sorted by avg_cost ascending.
            let mut opposite_legs: Vec<usize> = legs
                .iter()
                .enumerate()
                .filter(|(_, l)| l.side == opposite)
                .map(|(i, _)| i)
                .collect();
            opposite_legs.sort_by(|a, b| legs[*a].avg_cost.cmp(&legs[*b].avg_cost));

            let mut remaining = fill_size;
            let mut new_pairs = Vec::new();
            let mut legs_to_reduce: Vec<(usize, Decimal)> = Vec::new();

            for &idx in &opposite_legs {
                if remaining.is_zero() {
                    break;
                }
                let opp_leg = &legs[idx];
                let pair_size = remaining.min(opp_leg.size);

                let (yes_cost, no_cost) = match side {
                    Side::Yes => (avg_cost * pair_size, opp_leg.avg_cost * pair_size),
                    Side::No => (opp_leg.avg_cost * pair_size, avg_cost * pair_size),
                };

                let locked_profit = pair_size - yes_cost - no_cost;

                let paired = PairedPosition {
                    id: Uuid::new_v4(),
                    condition_id: condition_id.to_string(),
                    paired_size: pair_size,
                    yes_cost,
                    no_cost,
                    locked_profit,
                    paired_at: Utc::now(),
                };

                info!(
                    condition_id = %condition_id,
                    paired_size = %pair_size,
                    locked_profit = %locked_profit,
                    "Legs paired"
                );

                new_pairs.push(paired);
                legs_to_reduce.push((idx, pair_size));
                remaining -= pair_size;
            }

            // Reduce or remove paired opposite legs (process in reverse to keep indices valid).
            for (idx, consumed) in legs_to_reduce.into_iter().rev() {
                legs[idx].size -= consumed;
                if legs[idx].size.is_zero() {
                    legs.remove(idx);
                }
            }

            (new_pairs, remaining)
            // `legs` is dropped here — borrow on `state.open_legs` is released.
        };

        // Safe now: `state.open_legs` borrow is gone, so we can touch `paired_positions`.
        state.paired_positions.extend(new_pairs.clone());

        // Any remaining fill becomes a new open leg — re-borrow open_legs fresh.
        if remaining > Decimal::ZERO {
            state
                .open_legs
                .entry(condition_id.to_string())
                .or_default()
                .push(OpenLeg {
                    id: Uuid::new_v4(),
                    condition_id: condition_id.to_string(),
                    token_id: token_id.to_string(),
                    side,
                    size: remaining,
                    avg_cost,
                    acquired_at: Utc::now(),
                });
        }

        new_pairs
    }

    /// Get a snapshot of current inventory for strategy analysis.
    pub async fn snapshot(&self) -> InventorySnapshot {
        let state = self.state.read().await;

        let mut total_unpaired = Decimal::ZERO;
        for legs in state.open_legs.values() {
            for leg in legs {
                total_unpaired += leg.avg_cost * leg.size;
            }
        }

        InventorySnapshot {
            open_legs: state.open_legs.clone(),
            total_unpaired_exposure: total_unpaired,
        }
    }

    /// Find stale open legs exceeding the max hold duration.
    pub async fn find_stale_legs(&self) -> Vec<OpenLeg> {
        let state = self.state.read().await;
        let now = Utc::now();
        let max_hold = chrono::Duration::seconds(self.max_unpaired_hold_secs as i64);
        let mut stale = Vec::new();

        for legs in state.open_legs.values() {
            for leg in legs {
                if now - leg.acquired_at > max_hold {
                    stale.push(leg.clone());
                }
            }
        }
        stale
    }

    /// Get all paired positions.
    pub async fn paired_positions(&self) -> Vec<PairedPosition> {
        self.state.read().await.paired_positions.clone()
    }

    /// Get total locked profit across all paired positions.
    pub async fn total_locked_profit(&self) -> Decimal {
        self.state
            .read()
            .await
            .paired_positions
            .iter()
            .map(|p| p.locked_profit)
            .sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;

    #[tokio::test]
    async fn test_fill_yes_then_no_pairs() {
        let mgr = InventoryManager::new(3600);

        // Fill YES side.
        let pairs = mgr
            .record_leg_fill("market1", "yes-tok", Side::Yes, dec!(100), dec!(0.45))
            .await;
        assert!(pairs.is_empty()); // No opposite side yet.

        // Fill NO side — should pair.
        let pairs = mgr
            .record_leg_fill("market1", "no-tok", Side::No, dec!(100), dec!(0.50))
            .await;
        assert_eq!(pairs.len(), 1);
        assert_eq!(pairs[0].paired_size, dec!(100));
        // locked_profit = 100 - (0.45 * 100) - (0.50 * 100) = 100 - 45 - 50 = 5
        assert_eq!(pairs[0].locked_profit, dec!(5));
    }

    #[tokio::test]
    async fn test_partial_pairing() {
        let mgr = InventoryManager::new(3600);

        // Fill 100 YES.
        mgr.record_leg_fill("market1", "yes-tok", Side::Yes, dec!(100), dec!(0.45))
            .await;

        // Fill 60 NO — should partially pair.
        let pairs = mgr
            .record_leg_fill("market1", "no-tok", Side::No, dec!(60), dec!(0.50))
            .await;
        assert_eq!(pairs.len(), 1);
        assert_eq!(pairs[0].paired_size, dec!(60));
        // locked_profit = 60 - (0.45 * 60) - (0.50 * 60) = 60 - 27 - 30 = 3
        assert_eq!(pairs[0].locked_profit, dec!(3));

        // Snapshot should show 40 YES remaining.
        let snap = mgr.snapshot().await;
        let legs = snap.open_legs.get("market1").unwrap();
        assert_eq!(legs.len(), 1);
        assert_eq!(legs[0].side, Side::Yes);
        assert_eq!(legs[0].size, dec!(40));
    }

    #[tokio::test]
    async fn test_multi_leg_pairing() {
        let mgr = InventoryManager::new(3600);

        // Fill two YES legs at different costs.
        mgr.record_leg_fill("market1", "yes-tok", Side::Yes, dec!(50), dec!(0.40))
            .await;
        mgr.record_leg_fill("market1", "yes-tok", Side::Yes, dec!(50), dec!(0.45))
            .await;

        // Fill 80 NO — pairs with cheapest first.
        let pairs = mgr
            .record_leg_fill("market1", "no-tok", Side::No, dec!(80), dec!(0.50))
            .await;

        // Should pair 50 with the 0.40 leg and 30 with the 0.45 leg.
        assert_eq!(pairs.len(), 2);
        assert_eq!(pairs[0].paired_size, dec!(50));
        // 50 - (0.40*50) - (0.50*50) = 50 - 20 - 25 = 5
        assert_eq!(pairs[0].locked_profit, dec!(5));
        assert_eq!(pairs[1].paired_size, dec!(30));
        // 30 - (0.45*30) - (0.50*30) = 30 - 13.5 - 15 = 1.5
        assert_eq!(pairs[1].locked_profit, dec!(1.5));

        // 20 YES remaining from the 0.45 leg.
        let snap = mgr.snapshot().await;
        let legs = snap.open_legs.get("market1").unwrap();
        assert_eq!(legs.len(), 1);
        assert_eq!(legs[0].size, dec!(20));
        assert_eq!(legs[0].avg_cost, dec!(0.45));
    }

    #[tokio::test]
    async fn test_snapshot_exposure() {
        let mgr = InventoryManager::new(3600);

        mgr.record_leg_fill("market1", "yes-tok", Side::Yes, dec!(100), dec!(0.45))
            .await;
        mgr.record_leg_fill("market2", "yes-tok2", Side::Yes, dec!(50), dec!(0.30))
            .await;

        let snap = mgr.snapshot().await;
        // 100 * 0.45 + 50 * 0.30 = 45 + 15 = 60
        assert_eq!(snap.total_unpaired_exposure, dec!(60));
    }

    #[tokio::test]
    async fn test_stale_detection() {
        let mgr = InventoryManager::new(0); // 0 secs = everything is stale immediately

        mgr.record_leg_fill("market1", "yes-tok", Side::Yes, dec!(100), dec!(0.45))
            .await;

        // With max_hold=0, the leg is immediately stale.
        let stale = mgr.find_stale_legs().await;
        assert_eq!(stale.len(), 1);
    }
}
