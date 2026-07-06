//! PaperBroker : simulation de portefeuille pour le mode paper.
//!
//! Rôles :
//! - empêcher la répétition des entrées taker (1 entrée par fenêtre et par
//!   côté — défaut observé au run du 2026-07-04 : 931 ordres identiques),
//! - simuler les fills maker contre les trades RÉELS du carnet (un trade
//!   imprimé à un prix ≤ notre bid remplit le bid ; ≥ notre ask remplit l'ask),
//! - régler les positions à la résolution de la fenêtre et tenir le PnL.
//!
//! Tout est pur et unitairement testé ; l'orchestrateur ne fait que router
//! les événements.

use pm_core::events::Side;
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, Default)]
pub struct PaperPosition {
    pub size: f64,
    pub avg_entry: f64,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct RestingQuote {
    pub price: f64,
    pub size: f64,
}

#[derive(Debug, Clone, Default)]
pub struct TokenBook {
    /// Position maker (gérée par TP/stop/liquidation du maker).
    pub position: PaperPosition,
    /// Position taker (portée jusqu'à la résolution : la thèse taker est un
    /// pari sur l'issue, pas un aller-retour). Séparée depuis le backtest du
    /// 2026-07-04 : le maker qui « gérait » les positions taker détruisait
    /// leur espérance (−600 $ combiné vs +257 $ taker seul).
    pub taker_position: PaperPosition,
    pub bid: Option<RestingQuote>,
    pub ask: Option<RestingQuote>,
    /// PnL réalisé (ventes - achats réglés) pour ce token.
    pub realized_pnl: f64,
    pub taker_entries: u32,
    pub maker_fills: u32,
}

#[derive(Debug, Clone, Default)]
pub struct WindowReport {
    pub slug: String,
    pub strike: Option<f64>,
    pub outcome: Option<String>,
    pub pnl_up: f64,
    pub pnl_down: f64,
    pub taker_entries: u32,
    pub maker_fills: u32,
}

#[derive(Debug, Default)]
pub struct PaperBroker {
    /// Livres par token (up et down de la fenêtre courante).
    books: HashMap<String, TokenBook>,
    /// Entrées taker déjà faites cette fenêtre, par token.
    pub reports: Vec<WindowReport>,
    /// Un stop maker a eu lieu dans la fenêtre courante (gel des entrées).
    window_frozen: bool,
}

impl PaperBroker {
    pub fn new() -> Self {
        Self::default()
    }

    fn book(&mut self, token: &str) -> &mut TokenBook {
        self.books.entry(token.to_string()).or_default()
    }

    pub fn freeze_window(&mut self) {
        self.window_frozen = true;
    }

    pub fn is_window_frozen(&self) -> bool {
        self.window_frozen
    }

    /// Position MAKER (celle que le maker gère).
    pub fn position(&self, token: &str) -> PaperPosition {
        self.books
            .get(token)
            .map(|b| b.position)
            .unwrap_or_default()
    }

    /// Position TAKER (portée à résolution, jamais gérée par le maker).
    pub fn taker_position(&self, token: &str) -> PaperPosition {
        self.books
            .get(token)
            .map(|b| b.taker_position)
            .unwrap_or_default()
    }

    /// Le taker peut-il entrer sur ce token ? (1 entrée max par fenêtre/token,
    /// et jamais si une position est déjà ouverte.)
    pub fn can_take(&self, token: &str) -> bool {
        self.books
            .get(token)
            .map(|b| b.taker_entries == 0 && b.taker_position.size <= 0.0)
            .unwrap_or(true)
    }

    /// Enregistre une entrée taker (fill immédiat supposé au prix moyen calculé
    /// par la stratégie — elle a déjà vérifié la profondeur réelle du carnet).
    pub fn fill_taker(&mut self, token: &str, price: f64, size: f64) {
        self.fill_taker_avec_frais(token, price, size, 0.0);
    }

    /// Comme `fill_taker`, en déduisant les frais taker Polymarket réels :
    /// frais = size × taux × p(1−p) (catégorie Crypto : taux = 0,07).
    pub fn fill_taker_avec_frais(&mut self, token: &str, price: f64, size: f64, fee_rate: f64) {
        let b = self.book(token);
        let cost = b.taker_position.avg_entry * b.taker_position.size + price * size;
        b.taker_position.size += size;
        b.taker_position.avg_entry = if b.taker_position.size > 0.0 {
            cost / b.taker_position.size
        } else {
            0.0
        };
        let fees = size * fee_rate * price * (1.0 - price);
        b.realized_pnl -= price * size + fees;
        b.taker_entries += 1;
    }

    /// Met à jour les quotes maker au repos (remplacement idempotent).
    /// Renvoie true seulement si quelque chose change (permet de ne
    /// journaliser que les transitions).
    pub fn set_quotes(
        &mut self,
        token: &str,
        bid: Option<RestingQuote>,
        ask: Option<RestingQuote>,
    ) -> bool {
        let b = self.book(token);
        let same = |a: Option<RestingQuote>, x: Option<RestingQuote>| match (a, x) {
            (None, None) => true,
            (Some(p), Some(q)) => {
                (p.price - q.price).abs() < 1e-9 && (p.size - q.size).abs() < 1e-9
            }
            _ => false,
        };
        let changed = !(same(b.bid, bid) && same(b.ask, ask));
        b.bid = bid;
        b.ask = ask;
        changed
    }

    /// Un trade réel s'est imprimé sur ce token : nos quotes seraient-elles
    /// remplies ? Convention prudente : il faut que le prix du trade CROISE la
    /// quote (≤ bid pour l'achat, ≥ ask pour la vente).
    /// Renvoie (achat exécuté, vente exécutée).
    pub fn on_market_trade(&mut self, token: &str, trade_price: f64, _side: Side) -> (bool, bool) {
        let b = self.book(token);
        let mut filled = (false, false);
        if let Some(bid) = b.bid {
            if trade_price <= bid.price {
                let cost = b.position.avg_entry * b.position.size + bid.price * bid.size;
                b.position.size += bid.size;
                b.position.avg_entry = cost / b.position.size;
                b.realized_pnl -= bid.price * bid.size;
                b.bid = None;
                b.maker_fills += 1;
                filled.0 = true;
            }
        }
        if let Some(ask) = b.ask {
            if trade_price >= ask.price && b.position.size > 0.0 {
                let sell = ask.size.min(b.position.size);
                b.realized_pnl += ask.price * sell;
                b.position.size -= sell;
                if b.position.size <= 1e-9 {
                    b.position = PaperPosition::default();
                }
                b.ask = None;
                b.maker_fills += 1;
                filled.1 = true;
            }
        }
        filled
    }

    /// Sortie immédiate simulée (ExitNow) au prix limite donné.
    pub fn exit_now(&mut self, token: &str, limit_price: f64) {
        let b = self.book(token);
        if b.position.size > 0.0 {
            b.realized_pnl += limit_price * b.position.size;
            b.position = PaperPosition::default();
        }
        b.bid = None;
        b.ask = None;
    }

    /// Règle la fenêtre : chaque part du token gagnant vaut 1 $, du perdant 0 $.
    /// `up_won` : issue de la résolution. Retourne le rapport de fenêtre.
    pub fn settle_window(
        &mut self,
        slug: &str,
        token_up: &str,
        token_down: &str,
        strike: Option<f64>,
        up_won: Option<bool>,
        outcome_label: Option<String>,
    ) -> WindowReport {
        let mut report = WindowReport {
            slug: slug.to_string(),
            strike,
            outcome: outcome_label,
            ..Default::default()
        };
        for (token, is_up) in [(token_up, true), (token_down, false)] {
            let b = self.books.remove(token).unwrap_or_default();
            let mut pnl = b.realized_pnl;
            let residual = b.position.size + b.taker_position.size;
            if let (Some(won), true) = (up_won, residual > 0.0) {
                let token_wins = if is_up { won } else { !won };
                if token_wins {
                    pnl += residual; // 1 $ la part
                }
                // sinon : la position vaut 0, le coût est déjà dans realized.
            }
            if is_up {
                report.pnl_up = pnl;
            } else {
                report.pnl_down = pnl;
            }
            report.taker_entries += b.taker_entries;
            report.maker_fills += b.maker_fills;
        }
        self.reports.push(report.clone());
        self.window_frozen = false;
        report
    }

    pub fn total_pnl(&self) -> f64 {
        self.reports.iter().map(|r| r.pnl_up + r.pnl_down).sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const UP: &str = "token-up";
    const DOWN: &str = "token-down";

    #[test]
    fn taker_single_entry_per_window() {
        let mut pb = PaperBroker::new();
        assert!(pb.can_take(UP));
        pb.fill_taker(UP, 0.80, 100.0);
        assert!(!pb.can_take(UP), "pas de 2e entrée sur le même token");
        assert!(pb.can_take(DOWN), "l'autre token reste libre");
        // Après règlement, la fenêtre suivante repart à zéro.
        pb.settle_window("w1", UP, DOWN, Some(63000.0), Some(true), Some("Up".into()));
        assert!(pb.can_take(UP));
    }

    #[test]
    fn taker_win_and_loss_pnl() {
        let mut pb = PaperBroker::new();
        pb.fill_taker(UP, 0.80, 100.0); // coût 80
        let r = pb.settle_window("w", UP, DOWN, None, Some(true), None);
        assert!(
            (r.pnl_up - 20.0).abs() < 1e-9,
            "gagné: 100 − 80 = +20, obtenu {}",
            r.pnl_up
        );

        let mut pb = PaperBroker::new();
        pb.fill_taker(UP, 0.80, 100.0);
        let r = pb.settle_window("w", UP, DOWN, None, Some(false), None);
        assert!(
            (r.pnl_up + 80.0).abs() < 1e-9,
            "perdu: −80, obtenu {}",
            r.pnl_up
        );
    }

    #[test]
    fn maker_bid_fill_then_tp_ask_fill() {
        let mut pb = PaperBroker::new();
        pb.set_quotes(
            UP,
            Some(RestingQuote {
                price: 0.60,
                size: 50.0,
            }),
            None,
        );
        // Trade imprimé à 0.59 → bid rempli.
        let (bought, _) = pb.on_market_trade(UP, 0.59, Side::Sell);
        assert!(bought);
        assert_eq!(pb.position(UP).size, 50.0);
        // TP posé à 0.68, trade à 0.70 → vendu.
        pb.set_quotes(
            UP,
            None,
            Some(RestingQuote {
                price: 0.68,
                size: 50.0,
            }),
        );
        let (_, sold) = pb.on_market_trade(UP, 0.70, Side::Buy);
        assert!(sold);
        assert_eq!(pb.position(UP).size, 0.0);
        let r = pb.settle_window("w", UP, DOWN, None, None, None);
        // 50 × (0.68 − 0.60) = +4.00 réalisés, sans dépendre de la résolution.
        assert!((r.pnl_up - 4.0).abs() < 1e-9, "pnl={}", r.pnl_up);
        assert_eq!(r.maker_fills, 2);
    }

    #[test]
    fn maker_quote_not_filled_without_crossing_trade() {
        let mut pb = PaperBroker::new();
        pb.set_quotes(
            UP,
            Some(RestingQuote {
                price: 0.60,
                size: 50.0,
            }),
            None,
        );
        let (bought, _) = pb.on_market_trade(UP, 0.62, Side::Buy);
        assert!(!bought, "un trade à 0.62 ne remplit pas un bid à 0.60");
        assert_eq!(pb.position(UP).size, 0.0);
    }

    #[test]
    fn exit_now_flattens_maker_position_only() {
        let mut pb = PaperBroker::new();
        // Fill maker à 0.60 (bid croisé par un trade), puis sortie forcée à 0.55.
        pb.set_quotes(
            UP,
            Some(RestingQuote {
                price: 0.60,
                size: 50.0,
            }),
            None,
        );
        pb.on_market_trade(UP, 0.59, Side::Sell);
        // Position taker séparée : elle ne doit PAS être touchée par exit_now.
        pb.fill_taker(UP, 0.80, 10.0);
        pb.exit_now(UP, 0.55);
        assert_eq!(pb.position(UP).size, 0.0, "position maker liquidée");
        assert_eq!(pb.taker_position(UP).size, 10.0, "position taker intacte");
        let r = pb.settle_window("w", UP, DOWN, None, Some(true), None);
        // Maker : 50×(0.55−0.60) = −2.5 ; taker : 10×(1−0.80) = +2.0 → −0.5.
        assert!((r.pnl_up + 0.5).abs() < 1e-9, "pnl={}", r.pnl_up);
    }

    #[test]
    fn unresolved_window_keeps_costs_only() {
        let mut pb = PaperBroker::new();
        pb.fill_taker(UP, 0.80, 10.0);
        let r = pb.settle_window("w", UP, DOWN, None, None, None);
        // Sans issue connue, la position n'est pas créditée (prudence comptable).
        assert!((r.pnl_up + 8.0).abs() < 1e-9);
    }

    #[test]
    fn total_pnl_accumulates() {
        let mut pb = PaperBroker::new();
        pb.fill_taker(UP, 0.5, 10.0);
        pb.settle_window("w1", UP, DOWN, None, Some(true), None);
        pb.fill_taker(DOWN, 0.5, 10.0);
        pb.settle_window("w2", UP, DOWN, None, Some(true), None); // down perd
        assert!((pb.total_pnl() - (5.0 - 5.0)).abs() < 1e-9);
    }
}
