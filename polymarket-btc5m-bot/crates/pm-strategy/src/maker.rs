//! Module 3 — décision MARKET MAKER : « achat 60 → légère évolution → revente 72 ».
//!
//! Philosophie : prudent mais sûr. On ne porte jamais une position jusqu'à la
//! résolution par choix ; on capture le déplacement du fair price.
//!
//! Cycle par token :
//! 1. ENTRÉE : poser un bid sous le fair modèle (marge d'adversité), quand le
//!    sens du modèle est favorable et l'environnement sain.
//! 2. SORTIE : dès le fill, poser l'ask de take-profit ; si le modèle se
//!    retourne au-delà du stop, sortir au marché (limite marketable).
//! 3. ANNULATION : tout ordre au repos est annulé si les conditions se
//!    dégradent (flux stale, vol spike, fin de fenêtre proche).
//!
//! Les décisions sont pures : (snapshot, estimation, inventaire) → actions.

use crate::model::{MarketSnapshot, ProbEstimate};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct MakerConfig {
    /// Zone de prix quotable : on ne quote pas les extrêmes (gamma risk).
    pub min_quote_price: f64,
    pub max_quote_price: f64,
    /// Marge d'adversité : bid ≤ fair − edge_margin.
    pub edge_margin: f64,
    /// Objectif de take-profit (en probabilité, ex. +0.08 = 60c → 68c).
    pub take_profit: f64,
    /// Stop : si le fair passe sous entry − stop_loss, on sort au marché.
    pub stop_loss: f64,
    /// Taille de quote (parts).
    pub quote_size: f64,
    /// Inventaire max par token (parts).
    pub max_inventory: f64,
    /// Ne plus OUVRIR sous ce temps restant (s).
    pub min_tau_open_s: f64,
    /// Liquider/annuler tout sous ce temps restant (s).
    pub min_tau_flat_s: f64,
    /// |z| au-delà duquel on ne quote plus (le taker prend le relais).
    pub max_abs_z_quote: f64,
    /// σ (par √s) au-delà duquel on retire les quotes (marché trop nerveux).
    pub sigma_panic_per_sqrt_s: f64,
    pub min_strike_confidence: f64,
    pub max_spot_age_ms: u64,
}

impl Default for MakerConfig {
    fn default() -> Self {
        Self {
            min_quote_price: 0.15,
            max_quote_price: 0.85,
            edge_margin: 0.03,
            take_profit: 0.08,
            stop_loss: 0.10,
            quote_size: 50.0,
            max_inventory: 150.0,
            min_tau_open_s: 60.0,
            min_tau_flat_s: 25.0,
            max_abs_z_quote: 2.5,
            sigma_panic_per_sqrt_s: 5e-4,
            min_strike_confidence: 0.8,
            max_spot_age_ms: 3_000,
        }
    }
}

/// Inventaire courant sur UN token.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct Inventory {
    pub position: f64,
    /// Prix moyen d'entrée de la position courante.
    pub avg_entry: f64,
}

/// Contexte de décision maker (leçons du run v4 du 2026-07-04 : −147 $ sur
/// la fenêtre 1783194600 en portant de l'inventaire des DEUX côtés après
/// des stops en cascade).
#[derive(Debug, Clone, Copy, Default)]
pub struct MakerContext {
    pub inv: Inventory,
    /// Position ouverte sur l'AUTRE token : inventaire mono-côté, on ne
    /// s'expose jamais long Up ET long Down en même temps.
    pub other_position: f64,
    /// Un stop a déjà été déclenché dans cette fenêtre : plus AUCUNE
    /// nouvelle entrée jusqu'au règlement (le régime est instable).
    pub window_frozen: bool,
}

/// Actions demandées à l'exécution (idempotentes : l'exécuteur compare à
/// l'état de ses ordres au repos et ne fait que le delta).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum QuoteAction {
    /// Maintenir un bid (prix, taille). Absence = annuler le bid existant.
    Bid { price: f64, size: f64 },
    /// Maintenir un ask de sortie (prix, taille).
    Ask { price: f64, size: f64 },
    /// Sortir immédiatement (limite marketable au meilleur bid).
    ExitNow { limit_price: f64, size: f64 },
}

/// Décision pour un token (Up ou Down).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct MakerDecision {
    pub actions: Vec<QuoteAction>,
    pub reason: String,
    /// Vrai si un stop-loss vient d'être déclenché (l'appelant doit geler la
    /// fenêtre via ce signal).
    pub stop_triggered: bool,
}

pub struct MakerStrategy {
    pub cfg: MakerConfig,
}

impl MakerStrategy {
    pub fn new(cfg: MakerConfig) -> Self {
        Self { cfg }
    }

    /// Décision pour le token Up (`fair = p_up`) ou Down (`fair = 1 − p_up`).
    pub fn decide_token(
        &self,
        snap: &MarketSnapshot,
        est: &ProbEstimate,
        for_up_token: bool,
        ctx: MakerContext,
    ) -> MakerDecision {
        let c = &self.cfg;
        let inv = ctx.inv;
        let fair = if for_up_token {
            est.p_up
        } else {
            1.0 - est.p_up
        };
        let book = if for_up_token {
            &snap.book_up
        } else {
            &snap.book_down
        };
        let tau = snap.tau_s();

        // --- Environnement dégradé : plus de quotes ; position → sortie. ---
        let panic = !est.reliable
            || snap.any_feed_stale
            || snap.strike.confidence < c.min_strike_confidence
            || snap.spot_age_ms() > c.max_spot_age_ms
            || est.sigma_used > c.sigma_panic_per_sqrt_s
            || tau < c.min_tau_flat_s;
        if panic {
            let mut actions = vec![];
            if inv.position > 0.0 {
                if let Some(bb) = book.best_bid() {
                    actions.push(QuoteAction::ExitNow {
                        limit_price: (bb.price - 0.02).max(0.01),
                        size: inv.position,
                    });
                }
            }
            return MakerDecision {
                actions,
                reason: "environnement dégradé → flat".into(),
                stop_triggered: false,
            };
        }

        let mut actions = vec![];
        let mut reasons = vec![];
        let mut stop_triggered = false;

        // --- Gestion de la position existante ---
        if inv.position > 0.0 {
            let target = (inv.avg_entry + c.take_profit).min(0.99);
            if fair <= inv.avg_entry - c.stop_loss {
                // Le modèle s'est retourné : on coupe.
                if let Some(bb) = book.best_bid() {
                    actions.push(QuoteAction::ExitNow {
                        limit_price: (bb.price - 0.02).max(0.01),
                        size: inv.position,
                    });
                    reasons.push(format!("stop: fair={fair:.3} < entry−{:.2}", c.stop_loss));
                    stop_triggered = true;
                }
            } else {
                // Take-profit au repos ; jamais sous le fair (on ne « donne » pas).
                // Aligné au tick supérieur : un ask non aligné est rejeté par le
                // CLOB et créait du churn de quotes en paper (run 2026-07-04).
                let ask_px = ((target.max(fair + 0.01)).min(0.99) * 100.0).ceil() / 100.0;
                actions.push(QuoteAction::Ask {
                    price: ask_px,
                    size: inv.position,
                });
                reasons.push(format!("TP ask@{ask_px:.2}"));
            }
        }

        // --- Nouvelle entrée ? ---
        let can_open = !ctx.window_frozen
            && ctx.other_position <= 0.0
            && tau >= c.min_tau_open_s
            && est.z.abs() <= c.max_abs_z_quote
            && inv.position + c.quote_size <= c.max_inventory
            && fair >= c.min_quote_price
            && fair <= c.max_quote_price
            // On n'ouvre que si le modèle est nettement de notre côté : fair
            // au-dessus du mid d'au moins la moitié de la marge d'adversité.
            && book.mid().is_some_and(|m| fair >= m + c.edge_margin / 2.0);
        if can_open {
            let bid_cap = fair - c.edge_margin;
            // Se placer au-dessus du meilleur bid si possible, sans dépasser le cap.
            let bid_px = match book.best_bid() {
                Some(bb) => (bb.price + snap_tick(book)).min(bid_cap),
                None => bid_cap,
            };
            let bid_px = (bid_px * 100.0).floor() / 100.0; // aligne au tick 0.01
            if bid_px >= c.min_quote_price && bid_px > 0.0 {
                actions.push(QuoteAction::Bid {
                    price: bid_px,
                    size: c.quote_size,
                });
                reasons.push(format!("bid@{bid_px:.2} fair={fair:.3}"));
            }
        }

        MakerDecision {
            actions,
            reason: reasons.join(" | "),
            stop_triggered,
        }
    }
}

/// Tick du carnet — 0.01 par défaut (0.001 aux extrêmes, géré plus tard via
/// tick_size_change).
fn snap_tick(_book: &pm_core::book::OrderBook) -> f64 {
    0.01
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::test_support::{book, snapshot};
    use crate::model::ProbModel;

    fn mk() -> MakerStrategy {
        MakerStrategy::new(MakerConfig::default())
    }

    fn est_for(snap: &MarketSnapshot) -> ProbEstimate {
        ProbModel::default().estimate(snap)
    }

    #[test]
    fn opens_bid_below_fair_when_model_favors_up() {
        // Spot légèrement au-dessus du strike, beaucoup de temps : fair ≈ 0.6.
        let mut snap = snapshot(80_030.0, 80_000.0, 1e-4, 240.0);
        snap.book_up = book(0.50, 100.0, 0.56, 100.0); // marché en retard sur le fair
        let e = est_for(&snap);
        assert!(e.p_up > 0.55 && e.p_up < 0.85, "p_up={}", e.p_up);
        let d = mk().decide_token(&snap, &e, true, MakerContext::default());
        let Some(QuoteAction::Bid { price, size }) = d.actions.first() else {
            panic!("attendu un bid, obtenu {:?}", d)
        };
        assert!(*price <= e.p_up - 0.03 + 1e-9, "bid {price} ≤ fair − marge");
        assert_eq!(*size, 50.0);
    }

    #[test]
    fn no_open_when_market_already_fair_priced() {
        // fair ≈ mid : pas de valeur à capter.
        let mut snap = snapshot(80_000.0, 80_000.0, 1e-4, 240.0);
        snap.book_up = book(0.49, 100.0, 0.51, 100.0);
        let e = est_for(&snap);
        let d = mk().decide_token(&snap, &e, true, MakerContext::default());
        assert!(d.actions.is_empty(), "{:?}", d);
    }

    #[test]
    fn take_profit_ask_after_fill() {
        let mut snap = snapshot(80_030.0, 80_000.0, 1e-4, 200.0);
        snap.book_up = book(0.58, 100.0, 0.64, 100.0);
        let e = est_for(&snap);
        let ctx = MakerContext {
            inv: Inventory {
                position: 50.0,
                avg_entry: 0.60,
            },
            ..Default::default()
        };
        let d = mk().decide_token(&snap, &e, true, ctx);
        assert!(
            d.actions
                .iter()
                .any(|a| matches!(a, QuoteAction::Ask { price, size }
                if (*price - 0.68).abs() < 0.05 && *size == 50.0)),
            "attendu ask TP ~0.68 : {:?}",
            d
        );
    }

    #[test]
    fn stop_loss_exits_at_market() {
        // Entré à 0.60, le spot est repassé nettement sous le strike.
        let mut snap = snapshot(79_920.0, 80_000.0, 1e-4, 200.0);
        snap.book_up = book(0.38, 100.0, 0.42, 100.0);
        let e = est_for(&snap);
        assert!(e.p_up < 0.5);
        let ctx = MakerContext {
            inv: Inventory {
                position: 50.0,
                avg_entry: 0.60,
            },
            ..Default::default()
        };
        let d = mk().decide_token(&snap, &e, true, ctx);
        assert!(
            d.actions
                .iter()
                .any(|a| matches!(a, QuoteAction::ExitNow { .. })),
            "stop attendu: {:?}",
            d
        );
    }

    #[test]
    fn no_new_open_near_resolution_but_flatten_everything_late() {
        // Sous min_tau_open : pas de nouveau bid.
        let mut snap = snapshot(80_030.0, 80_000.0, 1e-4, 45.0);
        snap.book_up = book(0.50, 100.0, 0.56, 100.0);
        let e = est_for(&snap);
        let d = mk().decide_token(&snap, &e, true, MakerContext::default());
        assert!(!d
            .actions
            .iter()
            .any(|a| matches!(a, QuoteAction::Bid { .. })));

        // Sous min_tau_flat : position liquidée.
        let mut snap2 = snapshot(80_030.0, 80_000.0, 1e-4, 15.0);
        snap2.book_up = book(0.60, 100.0, 0.66, 100.0);
        let e2 = est_for(&snap2);
        let ctx = MakerContext {
            inv: Inventory {
                position: 50.0,
                avg_entry: 0.55,
            },
            ..Default::default()
        };
        let d2 = mk().decide_token(&snap2, &e2, true, ctx);
        assert!(
            d2.actions
                .iter()
                .any(|a| matches!(a, QuoteAction::ExitNow { .. })),
            "liquidation attendue: {:?}",
            d2
        );
    }

    #[test]
    fn stale_feed_cancels_quotes_and_exits() {
        let mut snap = snapshot(80_030.0, 80_000.0, 1e-4, 200.0);
        snap.book_up = book(0.55, 100.0, 0.60, 100.0);
        snap.any_feed_stale = true;
        let e = est_for(&snap);
        let d = mk().decide_token(
            &snap,
            &e,
            true,
            MakerContext {
                inv: Inventory {
                    position: 50.0,
                    avg_entry: 0.55,
                },
                ..Default::default()
            },
        );
        assert_eq!(d.actions.len(), 1);
        assert!(matches!(d.actions[0], QuoteAction::ExitNow { .. }));
    }

    #[test]
    fn inventory_cap_blocks_new_bids() {
        let mut snap = snapshot(80_030.0, 80_000.0, 1e-4, 240.0);
        snap.book_up = book(0.50, 100.0, 0.56, 100.0);
        let e = est_for(&snap);
        let ctx = MakerContext {
            inv: Inventory {
                position: 150.0,
                avg_entry: 0.55,
            },
            ..Default::default()
        };
        let d = mk().decide_token(&snap, &e, true, ctx);
        assert!(!d
            .actions
            .iter()
            .any(|a| matches!(a, QuoteAction::Bid { .. })));
    }

    #[test]
    fn single_side_inventory_blocks_other_token() {
        let mut snap = snapshot(80_030.0, 80_000.0, 1e-4, 240.0);
        snap.book_up = book(0.50, 100.0, 0.56, 100.0);
        let e = est_for(&snap);
        // Position ouverte sur l'autre token ⇒ pas de nouveau bid ici.
        let ctx = MakerContext {
            other_position: 50.0,
            ..Default::default()
        };
        let d = mk().decide_token(&snap, &e, true, ctx);
        assert!(
            !d.actions
                .iter()
                .any(|a| matches!(a, QuoteAction::Bid { .. })),
            "mono-côté : {:?}",
            d
        );
    }

    #[test]
    fn frozen_window_blocks_new_entries_but_manages_position() {
        let mut snap = snapshot(80_030.0, 80_000.0, 1e-4, 240.0);
        snap.book_up = book(0.50, 100.0, 0.56, 100.0);
        let e = est_for(&snap);
        // Fenêtre gelée sans position : rien.
        let d = mk().decide_token(
            &snap,
            &e,
            true,
            MakerContext {
                window_frozen: true,
                ..Default::default()
            },
        );
        assert!(d.actions.is_empty(), "{:?}", d);
        // Fenêtre gelée avec position : le TP reste géré (on sort, on n'ajoute pas).
        let d2 = mk().decide_token(
            &snap,
            &e,
            true,
            MakerContext {
                inv: Inventory {
                    position: 50.0,
                    avg_entry: 0.52,
                },
                window_frozen: true,
                ..Default::default()
            },
        );
        assert!(d2
            .actions
            .iter()
            .any(|a| matches!(a, QuoteAction::Ask { .. })));
        assert!(!d2
            .actions
            .iter()
            .any(|a| matches!(a, QuoteAction::Bid { .. })));
    }

    #[test]
    fn high_z_hands_over_to_taker() {
        // Incohérence violente : le maker se retire (pas de quote).
        let mut snap = snapshot(80_250.0, 80_000.0, 2e-5, 90.0);
        snap.book_up = book(0.78, 500.0, 0.80, 400.0);
        let e = est_for(&snap);
        assert!(e.z > 2.5);
        let d = mk().decide_token(&snap, &e, true, MakerContext::default());
        assert!(!d
            .actions
            .iter()
            .any(|a| matches!(a, QuoteAction::Bid { .. })));
    }
}
