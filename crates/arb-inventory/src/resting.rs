//! Resting-order tracking.
//!
//! When a GTC leg is accepted by the CLOB but not immediately matched, the
//! executor returns a `LegExecutionResult::Resting`. The bot stores it here so
//! a background poller can later transition it to a fill (via
//! `InventoryManager::record_leg_fill`) or remove it on cancel.
//!
//! The book is keyed by the CLOB-assigned `order_id`, which is the same id
//! used by `cancel_order` and `get_order_status` on the executor.

use arb_types::RestingOrder;
use std::collections::HashMap;
use tokio::sync::RwLock;

#[derive(Default)]
pub struct RestingOrderBook {
    orders: RwLock<HashMap<String, RestingOrder>>,
}

impl RestingOrderBook {
    pub fn new() -> Self {
        Self::default()
    }

    /// Insert a resting order. If `order_id` already exists, the entry is replaced.
    pub async fn add(&self, order: RestingOrder) {
        self.orders.write().await.insert(order.order_id.clone(), order);
    }

    /// Remove and return the order with the given id, if any.
    pub async fn remove(&self, order_id: &str) -> Option<RestingOrder> {
        self.orders.write().await.remove(order_id)
    }

    /// Get a clone of the order with the given id, if any.
    pub async fn get(&self, order_id: &str) -> Option<RestingOrder> {
        self.orders.read().await.get(order_id).cloned()
    }

    /// Snapshot of all currently resting orders.
    pub async fn all(&self) -> Vec<RestingOrder> {
        self.orders.read().await.values().cloned().collect()
    }

    pub async fn len(&self) -> usize {
        self.orders.read().await.len()
    }

    pub async fn is_empty(&self) -> bool {
        self.orders.read().await.is_empty()
    }

    /// Count of resting orders for a given market.
    pub async fn count_for_market(&self, condition_id: &str) -> usize {
        self.orders
            .read()
            .await
            .values()
            .filter(|o| o.condition_id == condition_id)
            .count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use arb_types::Side;
    use chrono::Utc;
    use rust_decimal_macros::dec;

    fn make_order(id: &str, condition_id: &str, side: Side) -> RestingOrder {
        RestingOrder {
            order_id: id.to_string(),
            condition_id: condition_id.to_string(),
            token_id: "tok".to_string(),
            side,
            size: dec!(10),
            price: dec!(0.45),
            posted_at: Utc::now(),
        }
    }

    #[tokio::test]
    async fn add_get_remove_round_trip() {
        let book = RestingOrderBook::new();
        let o = make_order("abc", "market1", Side::Yes);
        book.add(o.clone()).await;
        assert_eq!(book.len().await, 1);
        let got = book.get("abc").await.unwrap();
        assert_eq!(got.condition_id, "market1");
        let removed = book.remove("abc").await.unwrap();
        assert_eq!(removed.order_id, "abc");
        assert!(book.is_empty().await);
    }

    #[tokio::test]
    async fn add_replaces_same_id() {
        let book = RestingOrderBook::new();
        let mut o = make_order("abc", "m1", Side::Yes);
        book.add(o.clone()).await;
        o.size = dec!(99);
        book.add(o).await;
        assert_eq!(book.len().await, 1);
        assert_eq!(book.get("abc").await.unwrap().size, dec!(99));
    }

    #[tokio::test]
    async fn count_for_market_filters_by_condition_id() {
        let book = RestingOrderBook::new();
        book.add(make_order("a", "m1", Side::Yes)).await;
        book.add(make_order("b", "m1", Side::No)).await;
        book.add(make_order("c", "m2", Side::Yes)).await;
        assert_eq!(book.count_for_market("m1").await, 2);
        assert_eq!(book.count_for_market("m2").await, 1);
        assert_eq!(book.count_for_market("nope").await, 0);
    }

    #[tokio::test]
    async fn all_returns_snapshot() {
        let book = RestingOrderBook::new();
        book.add(make_order("a", "m1", Side::Yes)).await;
        book.add(make_order("b", "m2", Side::No)).await;
        let mut snap = book.all().await;
        snap.sort_by(|x, y| x.order_id.cmp(&y.order_id));
        assert_eq!(snap.len(), 2);
        assert_eq!(snap[0].order_id, "a");
        assert_eq!(snap[1].order_id, "b");
    }
}
