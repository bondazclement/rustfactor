//! Carnet d'ordres L2 reconstruit depuis le canal market CLOB.
//!
//! Application stricte des règles documentées :
//! - `book`         → remplacement complet (snapshot),
//! - `price_change` → upsert du niveau, `size == 0` supprime le niveau.
//!
//! Fournit les métriques utilisées par les stratégies : best bid/ask, mid,
//! microprice (pondéré par l'imbalance), profondeur cumulée, détection de
//! carnet croisé/périmé.

use crate::events::{Level, PriceChangeLevel, Side};
use ordered_float::OrderedFloat;
use std::collections::BTreeMap;

#[derive(Debug, Clone, Default)]
pub struct OrderBook {
    bids: BTreeMap<OrderedFloat<f64>, f64>,
    asks: BTreeMap<OrderedFloat<f64>, f64>,
    /// Timestamp (ms) du dernier événement appliqué (horloge CLOB).
    pub last_event_ms: u64,
    /// Horloge locale de la dernière application.
    pub last_recv_ms: u64,
}

impl OrderBook {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn apply_snapshot(&mut self, bids: &[Level], asks: &[Level], ts_ms: u64, recv_ms: u64) {
        self.bids.clear();
        self.asks.clear();
        for l in bids {
            if l.size > 0.0 && l.price > 0.0 {
                self.bids.insert(OrderedFloat(l.price), l.size);
            }
        }
        for l in asks {
            if l.size > 0.0 && l.price > 0.0 {
                self.asks.insert(OrderedFloat(l.price), l.size);
            }
        }
        self.last_event_ms = ts_ms;
        self.last_recv_ms = recv_ms;
    }

    pub fn apply_delta(&mut self, ch: &PriceChangeLevel, ts_ms: u64, recv_ms: u64) {
        let side = match ch.side {
            Side::Buy => &mut self.bids,
            Side::Sell => &mut self.asks,
        };
        if ch.size <= 0.0 {
            side.remove(&OrderedFloat(ch.price));
        } else {
            side.insert(OrderedFloat(ch.price), ch.size);
        }
        self.last_event_ms = ts_ms;
        self.last_recv_ms = recv_ms;
    }

    pub fn best_bid(&self) -> Option<Level> {
        self.bids.iter().next_back().map(|(p, s)| Level {
            price: p.0,
            size: *s,
        })
    }

    pub fn best_ask(&self) -> Option<Level> {
        self.asks.iter().next().map(|(p, s)| Level {
            price: p.0,
            size: *s,
        })
    }

    pub fn mid(&self) -> Option<f64> {
        Some((self.best_bid()?.price + self.best_ask()?.price) / 2.0)
    }

    pub fn spread(&self) -> Option<f64> {
        Some(self.best_ask()?.price - self.best_bid()?.price)
    }

    /// Microprice : mid pondéré par les tailles au meilleur niveau.
    /// Plus robuste que le mid quand le carnet est déséquilibré.
    pub fn microprice(&self) -> Option<f64> {
        let b = self.best_bid()?;
        let a = self.best_ask()?;
        let tot = a.size + b.size;
        if tot <= 0.0 {
            return self.mid();
        }
        Some((a.price * b.size + b.price * a.size) / tot)
    }

    /// Imbalance au meilleur niveau, dans [-1, 1] (positif = pression acheteuse).
    pub fn imbalance(&self) -> Option<f64> {
        let b = self.best_bid()?;
        let a = self.best_ask()?;
        let tot = a.size + b.size;
        (tot > 0.0).then(|| (b.size - a.size) / tot)
    }

    /// Taille cumulée achetable sous `limit_price` (asks ≤ limit).
    pub fn ask_depth_to(&self, limit_price: f64) -> f64 {
        self.asks
            .range(..=OrderedFloat(limit_price))
            .map(|(_, s)| *s)
            .sum()
    }

    /// Taille cumulée vendable au-dessus de `limit_price` (bids ≥ limit).
    pub fn bid_depth_to(&self, limit_price: f64) -> f64 {
        self.bids
            .range(OrderedFloat(limit_price)..)
            .map(|(_, s)| *s)
            .sum()
    }

    /// Prix moyen d'exécution pour un ordre taker de `size` (None si liquidité insuffisante).
    pub fn taker_fill_price(&self, side: Side, size: f64) -> Option<f64> {
        let mut remaining = size;
        let mut cost = 0.0;
        let levels: Vec<Level> = match side {
            Side::Buy => self
                .asks
                .iter()
                .map(|(p, s)| Level {
                    price: p.0,
                    size: *s,
                })
                .collect(),
            Side::Sell => self
                .bids
                .iter()
                .rev()
                .map(|(p, s)| Level {
                    price: p.0,
                    size: *s,
                })
                .collect(),
        };
        for l in levels {
            let take = remaining.min(l.size);
            cost += take * l.price;
            remaining -= take;
            if remaining <= 1e-9 {
                return Some(cost / size);
            }
        }
        None
    }

    pub fn is_crossed(&self) -> bool {
        matches!((self.best_bid(), self.best_ask()),
            (Some(b), Some(a)) if b.price >= a.price)
    }

    pub fn is_empty(&self) -> bool {
        self.bids.is_empty() && self.asks.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lvl(price: f64, size: f64) -> Level {
        Level { price, size }
    }

    fn sample() -> OrderBook {
        let mut ob = OrderBook::new();
        ob.apply_snapshot(
            &[lvl(0.48, 30.0), lvl(0.49, 20.0), lvl(0.50, 15.0)],
            &[lvl(0.52, 25.0), lvl(0.53, 60.0), lvl(0.54, 10.0)],
            1000,
            1001,
        );
        ob
    }

    #[test]
    fn snapshot_and_best() {
        let ob = sample();
        assert_eq!(ob.best_bid().unwrap().price, 0.50);
        assert_eq!(ob.best_ask().unwrap().price, 0.52);
        assert!((ob.mid().unwrap() - 0.51).abs() < 1e-12);
        assert!((ob.spread().unwrap() - 0.02).abs() < 1e-12);
        assert!(!ob.is_crossed());
    }

    #[test]
    fn delta_upsert_and_remove() {
        let mut ob = sample();
        // Nouveau meilleur bid.
        ob.apply_delta(
            &PriceChangeLevel {
                asset_id: "x".into(),
                price: 0.51,
                size: 5.0,
                side: Side::Buy,
            },
            2000,
            2001,
        );
        assert_eq!(ob.best_bid().unwrap().price, 0.51);
        // size 0 supprime le niveau.
        ob.apply_delta(
            &PriceChangeLevel {
                asset_id: "x".into(),
                price: 0.51,
                size: 0.0,
                side: Side::Buy,
            },
            3000,
            3001,
        );
        assert_eq!(ob.best_bid().unwrap().price, 0.50);
        assert_eq!(ob.last_event_ms, 3000);
    }

    #[test]
    fn microprice_leans_toward_pressure() {
        let mut ob = OrderBook::new();
        // Gros bid, petit ask → pression acheteuse → microprice proche de l'ask.
        ob.apply_snapshot(&[lvl(0.50, 100.0)], &[lvl(0.52, 10.0)], 0, 0);
        let mp = ob.microprice().unwrap();
        assert!(mp > 0.515, "microprice {mp} devrait pencher vers l'ask");
        assert!(ob.imbalance().unwrap() > 0.8);
    }

    #[test]
    fn taker_fill_walks_levels() {
        let ob = sample();
        // 30 achetés : 25 @0.52 + 5 @0.53.
        let px = ob.taker_fill_price(Side::Buy, 30.0).unwrap();
        assert!((px - (25.0 * 0.52 + 5.0 * 0.53) / 30.0).abs() < 1e-12);
        // Liquidité insuffisante.
        assert!(ob.taker_fill_price(Side::Buy, 1000.0).is_none());
    }

    #[test]
    fn depth_metrics() {
        let ob = sample();
        assert_eq!(ob.ask_depth_to(0.53), 85.0);
        assert_eq!(ob.bid_depth_to(0.49), 35.0);
    }
}
