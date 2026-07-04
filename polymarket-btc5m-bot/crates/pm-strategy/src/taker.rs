//! Module 2 — décision TAKER : capturer les incohérences prix/probabilité.
//!
//! Idée : quand le modèle donne p (fiable) et que le marché vend le même
//! résultat à un prix très inférieur, l'écart (edge) rémunère le risque.
//! Cas d'école du cahier des charges : forte variation du BTC tard dans la
//! fenêtre, carnet encore « en retard » → |z| énorme, ask loin de 1.
//!
//! Garde-fous non négociables (jamais assouplis) :
//! - flux stale, strike douteux, spot vieux ⇒ AUCUN trade,
//! - profondeur réelle du carnet vérifiée (prix moyen d'exécution, pas le
//!   meilleur ask affiché),
//! - taille bornée par fraction de Kelly ET par plafond absolu.

use crate::model::{MarketSnapshot, ProbEstimate};
use pm_core::events::Side;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct TakerConfig {
    /// Edge minimal (probabilité modèle − prix payé, après coussin de coûts).
    pub min_edge: f64,
    /// |z| minimal : on ne prend que les vraies incohérences, pas le bruit.
    pub min_abs_z: f64,
    /// Confiance minimale sur le strike reconstruit.
    pub min_strike_confidence: f64,
    /// Âge maximal du dernier tick de résolution (ms).
    pub max_spot_age_ms: u64,
    /// Ne jamais entrer avant que la fenêtre soit « lisible ».
    pub min_elapsed_s: f64,
    /// Ne plus entrer sous ce temps restant (latence d'exécution + règlement).
    pub min_tau_s: f64,
    /// Coussin de coûts/slippage retranché de l'edge (en probabilité).
    pub cost_buffer: f64,
    /// Fraction de Kelly (0.25 = quart de Kelly).
    pub kelly_fraction: f64,
    /// Bankroll de référence (USDC) pour le sizing.
    pub bankroll: f64,
    /// Plafond absolu par ordre (USDC).
    pub max_notional: f64,
    /// Slippage max accepté entre meilleur ask et prix moyen d'exécution.
    pub max_slippage: f64,
}

impl Default for TakerConfig {
    fn default() -> Self {
        Self {
            min_edge: 0.06,
            min_abs_z: 2.0,
            min_strike_confidence: 0.8,
            max_spot_age_ms: 3_000,
            min_elapsed_s: 10.0,
            min_tau_s: 3.0,
            cost_buffer: 0.01,
            kelly_fraction: 0.25,
            bankroll: 1_000.0,
            max_notional: 250.0,
            max_slippage: 0.02,
        }
    }
}

/// Ordre taker proposé (IOC/FOK marketable sur le CLOB).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TakerDecision {
    /// true = token Up, false = token Down.
    pub buy_up: bool,
    /// Prix limite (croise le spread ; protège contre le slippage au-delà).
    pub limit_price: f64,
    /// Taille en parts.
    pub size: f64,
    /// Edge net estimé au prix moyen d'exécution.
    pub edge: f64,
    pub p_model: f64,
    pub z: f64,
    pub reason: String,
}

pub struct TakerStrategy {
    pub cfg: TakerConfig,
}

impl TakerStrategy {
    pub fn new(cfg: TakerConfig) -> Self {
        Self { cfg }
    }

    /// Décision pure : Some(ordre) si et seulement si toutes les conditions
    /// sont réunies.
    pub fn decide(&self, snap: &MarketSnapshot, est: &ProbEstimate) -> Option<TakerDecision> {
        let c = &self.cfg;
        // --- Garde-fous d'intégrité (jamais assouplis) ---
        if !est.reliable
            || snap.any_feed_stale
            || snap.strike.confidence < c.min_strike_confidence
            || snap.spot_age_ms() > c.max_spot_age_ms
        {
            return None;
        }
        let elapsed_s = (snap.now_ms.saturating_sub(snap.t0_ms)) as f64 / 1000.0;
        if elapsed_s < c.min_elapsed_s || snap.tau_s() < c.min_tau_s {
            return None;
        }
        if est.z.abs() < c.min_abs_z {
            return None;
        }

        // --- Choix du côté : probabilité modèle vs prix demandé ---
        let (buy_up, p_side, book) = if est.z > 0.0 {
            (true, est.p_up, &snap.book_up)
        } else {
            (false, 1.0 - est.p_up, &snap.book_down)
        };
        let best_ask = book.best_ask()?;

        // Edge brut au meilleur ask, avant vérification de profondeur.
        let gross_edge = p_side - best_ask.price - c.cost_buffer;
        if gross_edge < c.min_edge {
            return None;
        }

        // --- Sizing : quart de Kelly borné, puis contrôle de profondeur ---
        // Kelly binaire : f* = (p − a)/(1 − a), en fraction de bankroll.
        let a = best_ask.price;
        if a >= 1.0 {
            return None;
        }
        let kelly = ((p_side - a) / (1.0 - a)).clamp(0.0, 1.0);
        let notional = (c.bankroll * kelly * c.kelly_fraction).min(c.max_notional);
        let mut size = (notional / a).floor();
        if size < 1.0 {
            return None;
        }

        // Prix moyen réel en marchant le carnet ; réduit la taille si besoin.
        let mut avg = book.taker_fill_price(Side::Buy, size);
        while avg.is_none() && size > 1.0 {
            size = (size / 2.0).floor();
            avg = book.taker_fill_price(Side::Buy, size);
        }
        let avg = avg?;
        if avg - best_ask.price > c.max_slippage {
            return None;
        }
        let net_edge = p_side - avg - c.cost_buffer;
        if net_edge < c.min_edge {
            return None;
        }

        Some(TakerDecision {
            buy_up,
            // La limite protège : on accepte jusqu'à avg + un tick de marge.
            limit_price: (avg + 0.01).min(0.99),
            size,
            edge: net_edge,
            p_model: p_side,
            z: est.z,
            reason: format!(
                "z={:.2} p={:.3} ask={:.3} avg={:.3} tau={:.0}s",
                est.z, p_side, best_ask.price, avg, snap.tau_s()
            ),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::test_support::{book, snapshot};
    use crate::model::ProbModel;

    fn strategy() -> TakerStrategy {
        TakerStrategy::new(TakerConfig::default())
    }

    /// Le scénario du cahier des charges déclenche un achat Up.
    #[test]
    fn fires_on_late_incoherence() {
        let mut snap = snapshot(80_250.0, 80_000.0, 2e-5, 20.0);
        // Carnet « en retard » : Up se vend encore 0.80 alors que p≈1.
        snap.book_up = book(0.78, 500.0, 0.80, 400.0);
        let est = ProbModel::default().estimate(&snap);
        let d = strategy().decide(&snap, &est).expect("doit trader");
        assert!(d.buy_up);
        assert!(d.edge > 0.15, "edge={}", d.edge);
        assert!(d.size >= 100.0, "size={}", d.size);
        assert!(d.limit_price <= 0.99);
    }

    #[test]
    fn no_trade_when_market_is_fair() {
        // Spot au strike, carnet 0.48/0.52 : pas d'edge.
        let snap = snapshot(80_000.0, 80_000.0, 1e-4, 120.0);
        let est = ProbModel::default().estimate(&snap);
        assert!(strategy().decide(&snap, &est).is_none());
    }

    #[test]
    fn hard_guards_block_everything() {
        let base = {
            let mut s = snapshot(80_250.0, 80_000.0, 2e-5, 20.0);
            s.book_up = book(0.78, 500.0, 0.80, 400.0);
            s
        };
        let m = ProbModel::default();

        // Flux stale.
        let mut s = base.clone();
        s.any_feed_stale = true;
        let e = m.estimate(&s);
        assert!(strategy().decide(&s, &e).is_none(), "stale ⇒ pas de trade");

        // Strike douteux.
        let mut s = base.clone();
        s.strike.confidence = 0.3;
        let e = m.estimate(&s);
        assert!(strategy().decide(&s, &e).is_none(), "strike douteux ⇒ pas de trade");

        // Spot trop vieux.
        let mut s = base.clone();
        s.spot_source_ts_ms = s.now_ms - 10_000;
        let e = m.estimate(&s);
        assert!(strategy().decide(&s, &e).is_none(), "spot périmé ⇒ pas de trade");

        // Trop près de la résolution.
        let mut s = base.clone();
        s.now_ms = s.t_end_ms - 1_000;
        let e = m.estimate(&s);
        assert!(strategy().decide(&s, &e).is_none(), "tau < min ⇒ pas de trade");
    }

    #[test]
    fn buys_down_when_below_strike() {
        let mut snap = snapshot(79_750.0, 80_000.0, 2e-5, 20.0);
        snap.book_down = book(0.75, 500.0, 0.78, 400.0);
        let est = ProbModel::default().estimate(&snap);
        let d = strategy().decide(&snap, &est).expect("doit trader Down");
        assert!(!d.buy_up);
        assert!(d.z < -2.0);
    }

    #[test]
    fn size_shrinks_to_available_depth() {
        let mut snap = snapshot(80_250.0, 80_000.0, 2e-5, 20.0);
        // Très peu de profondeur : 20 parts seulement.
        snap.book_up = book(0.78, 10.0, 0.80, 20.0);
        let est = ProbModel::default().estimate(&snap);
        let d = strategy().decide(&snap, &est).expect("doit trader petit");
        assert!(d.size <= 20.0, "size={} doit tenir dans la profondeur", d.size);
    }

    #[test]
    fn respects_max_notional() {
        let mut cfg = TakerConfig::default();
        cfg.bankroll = 1_000_000.0; // Kelly énorme
        let mut snap = snapshot(80_250.0, 80_000.0, 2e-5, 20.0);
        snap.book_up = book(0.78, 100_000.0, 0.80, 100_000.0);
        let est = ProbModel::default().estimate(&snap);
        let d = TakerStrategy::new(cfg).decide(&snap, &est).unwrap();
        assert!(d.size * 0.80 <= cfg.max_notional * 1.01, "notional plafonné");
    }
}
