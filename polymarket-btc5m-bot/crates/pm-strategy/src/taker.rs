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
#[serde(default, deny_unknown_fields)]
pub struct TakerConfig {
    /// Mode « valeur » (entrées classiques ≤ max_entry_price sur gros edge).
    /// DÉSACTIVÉ par défaut depuis le 06/07 au soir : sur toutes les mesures
    /// (études 2-4, corpus walk-forward), ces entrées sont −EV — corpus :
    /// −228 $ attribuables au mode valeur vs +37 $ certitude seule.
    /// Réactivable ici le jour où un edge valeur est démontré.
    pub mode_valeur: bool,
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
    /// Taux de frais taker Polymarket (catégorie Crypto : 0,07).
    /// Frais par share = taux × p × (1−p) au prix du trade
    /// (https://docs.polymarket.com/trading/fees). Les makers ne paient rien.
    pub fee_rate: f64,
    /// Fraction de Kelly (0.25 = quart de Kelly).
    pub kelly_fraction: f64,
    /// Bankroll de référence (USDC) pour le sizing.
    pub bankroll: f64,
    /// Plafond absolu par ordre (USDC).
    pub max_notional: f64,
    /// Slippage max accepté entre meilleur ask et prix moyen d'exécution.
    pub max_slippage: f64,
    /// Prix d'achat maximal. Asymétrie brutale au-dessus : à 0,90 on risque
    /// 0,90/part pour gagner 0,10 — une seule erreur de modèle efface 9
    /// trades gagnants (backtest 2026-07-04 : −240 $ sur une entrée à 0,90).
    pub max_entry_price: f64,
    /// Écart minimal |spot − strike| EN DOLLARS. Mesure du 06/07 (étude 4) :
    /// tous les états perdants vivent sous ~50 $ d'écart — un z élevé y est
    /// souvent un artefact de σ sous-estimé (bruit d'estimation), pas une
    /// vraie avance. En dollars bruts, la mesure est robuste.
    pub min_dist_usd: f64,
    /// LA FRONTIÈRE (étude 6, docs/LIGNE_EFFICIENCE.md) : la seule règle
    /// d'entrée PROUVÉE à 90 % de confiance sur 13 h de marché — écart
    /// large + fin proche + prix encore payable. Balayages exhaustifs :
    /// la version « boîte » bat la version lissée c·√τ, et le stop-loss
    /// dynamique détruit de la valeur (étude 7) → positions portées au
    /// règlement, une entrée max par fenêtre.
    ///
    /// Curseur de confiance (presets, voir config.exemple.toml) :
    ///   prudent  : dist 70 $, τ 120 s, prix 0,96 → ~1,7 trade/h, +21 $/trade
    ///   standard : dist 70 $, τ 120 s, prix 0,98 → ~2,6 trades/h, +15 $/trade
    ///   agressif : dist 40 $, τ 120 s, prix 0,98 → ~7 trades/h, +2 $/trade (non prouvé)
    /// Écart minimal |spot − strike| (dollars) de la frontière.
    pub dist_frontiere_usd: f64,
    /// Temps restant maximal (s) de la frontière.
    pub tau_frontiere_s: f64,
    /// Plafond de prix payable dans la frontière.
    pub prix_max_frontiere: f64,
    /// Marge d'EV nette exigée (p calibrée − prix − frais ≥ marge).
    pub marge_ev: f64,
    /// MODE AVANCÉ : masque de cellules de la table de calibration
    /// (bit i·7+j = bac écart i × bac τ j, grille 7×7 de calib.rs).
    /// 0 = désactivé (boîte dist/tau ci-dessus). Non nul : l'entrée n'est
    /// permise QUE si l'état courant tombe dans une cellule sélectionnée —
    /// la zone se choisit visuellement dans pm-dash (table de calibration).
    pub zones_frontiere: u64,
}

impl Default for TakerConfig {
    fn default() -> Self {
        Self {
            mode_valeur: false,
            min_edge: 0.06,
            // Calibré sur backtest 24 fenêtres du 2026-07-04 : toute la grille
            // est positive, la zone la plus robuste est z≥2.5 / prix ≤0.85.
            min_abs_z: 2.5,
            min_strike_confidence: 0.8,
            max_spot_age_ms: 3_000,
            min_elapsed_s: 10.0,
            min_tau_s: 3.0,
            cost_buffer: 0.005,
            fee_rate: 0.07,
            kelly_fraction: 0.25,
            bankroll: 1_000.0,
            max_notional: 250.0,
            max_slippage: 0.02,
            max_entry_price: 0.85,
            min_dist_usd: 50.0,
            dist_frontiere_usd: 70.0,
            tau_frontiere_s: 120.0,
            prix_max_frontiere: 0.98,
            marge_ev: 0.01,
            zones_frontiere: 0,
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
    /// Prix moyen d'exécution estimé en marchant le carnet (fill paper).
    pub avg_price: f64,
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
        // Seuil de z durci quand il reste beaucoup de temps : une incohérence
        // à 290 s de la fin a bien plus d'occasions de se retourner qu'à 20 s
        // (perte observée au run v2 du 2026-07-04 : entrée z=-2.6 à tau=290 s,
        // issue inversée). Interpolation linéaire : ×2 à pleine fenêtre, ×1
        // sous 60 s restantes.
        let z_required = c.min_abs_z * (1.0 + (snap.tau_s() - 60.0).max(0.0) / 240.0);
        if est.z.abs() < z_required {
            return None;
        }
        // Écart au strike en dollars : un z élevé sur un écart de 20 $ est un
        // artefact (σ bruité), pas une opportunité — c'est là que vivaient
        // toutes les pertes mesurées.
        if let Some(k) = snap.strike.value {
            if (snap.spot - k).abs() < c.min_dist_usd {
                return None;
            }
        }

        // --- Choix du côté : probabilité modèle vs prix demandé ---
        let (buy_up, p_side, book) = if est.z > 0.0 {
            (true, est.p_up, &snap.book_up)
        } else {
            (false, 1.0 - est.p_up, &snap.book_down)
        };
        let best_ask = book.best_ask()?;
        // Mode certitude : écart large + fin proche ⇒ plafond de prix étendu
        // et marge d'edge réduite (gain unitaire faible, probabilité haute).
        let dist = snap
            .strike
            .value
            .map(|k| (snap.spot - k).abs())
            .unwrap_or(0.0);
        let frontiere = if c.zones_frontiere != 0 {
            // Mode avancé : zone = cellules cochées de la grille écart × τ.
            crate::calib::indices(dist, snap.tau_s())
                .is_some_and(|(i, j)| c.zones_frontiere & (1u64 << (i * 7 + j)) != 0)
        } else {
            dist >= c.dist_frontiere_usd && snap.tau_s() <= c.tau_frontiere_s
        };
        if !frontiere && !c.mode_valeur {
            return None;
        }
        let (prix_max, edge_min) = if frontiere {
            (c.prix_max_frontiere, c.marge_ev)
        } else {
            (c.max_entry_price, c.min_edge)
        };
        if best_ask.price > prix_max {
            return None;
        }

        // Edge brut au meilleur ask, avant vérification de profondeur.
        // Frais taker réels : taux × p(1−p) par share, payés à l'entrée.
        let fee = |p: f64| c.fee_rate * p * (1.0 - p);
        let gross_edge = p_side - best_ask.price - fee(best_ask.price) - c.cost_buffer;
        if gross_edge < edge_min {
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
        // Le contrôle de risque et l'engagement réel portent sur le prix
        // LIMITE (avg + marge de slippage), pas sur l'ask : dimensionner sur
        // l'ask faisait refuser des ordres pour 1 centime (micro-test 06/07).
        let limite_estimee = (a + c.max_slippage + 0.01).min(0.99);
        let mut size = (notional / limite_estimee).floor();
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
        let net_edge = p_side - avg - fee(avg) - c.cost_buffer;
        if net_edge < edge_min {
            return None;
        }

        Some(TakerDecision {
            buy_up,
            // La limite protège : on accepte jusqu'à avg + un tick de marge.
            limit_price: (avg + 0.01).min(0.99),
            size,
            avg_price: avg,
            edge: net_edge,
            p_model: p_side,
            z: est.z,
            reason: format!(
                "z={:.2} p={:.3} ask={:.3} avg={:.3} tau={:.0}s dist={:.0}$ mode={}",
                est.z,
                p_side,
                best_ask.price,
                avg,
                snap.tau_s(),
                dist,
                if frontiere { "frontiere" } else { "valeur" }
            ),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::test_support::{book, snapshot};
    use crate::model::ProbModel;

    /// Mode avancé : seules les cellules cochées autorisent l'entrée.
    #[test]
    fn zones_frontiere_masque_les_cellules() {
        // snapshot : écart 250 $, τ=20 s → bac écart 6 (≥150), bac τ 1 (15-30).
        let mut snap = snapshot(80_250.0, 80_000.0, 2e-5, 20.0);
        snap.book_up = book(0.78, 500.0, 0.80, 400.0);
        let est = ProbModel::default().estimate(&snap);
        let (i, j) = crate::calib::indices(250.0, 20.0).unwrap();

        let mut cfg = TakerConfig::default();
        cfg.zones_frontiere = 1u64 << (i * 7 + j); // SA cellule cochée
        assert!(TakerStrategy::new(cfg).decide(&snap, &est).is_some(), "cellule cochée ⇒ trade");

        cfg.zones_frontiere = 1u64 << ((i - 1) * 7 + j); // une AUTRE cellule
        assert!(TakerStrategy::new(cfg).decide(&snap, &est).is_none(), "cellule non cochée ⇒ refus");

        cfg.zones_frontiere = 0; // boîte classique (dist 250 ≥ 70, τ 20 ≤ 120)
        assert!(TakerStrategy::new(cfg).decide(&snap, &est).is_some(), "masque nul ⇒ boîte");
    }

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
        assert!(
            strategy().decide(&s, &e).is_none(),
            "strike douteux ⇒ pas de trade"
        );

        // Spot trop vieux.
        let mut s = base.clone();
        s.spot_source_ts_ms = s.now_ms - 10_000;
        let e = m.estimate(&s);
        assert!(
            strategy().decide(&s, &e).is_none(),
            "spot périmé ⇒ pas de trade"
        );

        // Trop près de la résolution.
        let mut s = base.clone();
        s.now_ms = s.t_end_ms - 1_000;
        let e = m.estimate(&s);
        assert!(
            strategy().decide(&s, &e).is_none(),
            "tau < min ⇒ pas de trade"
        );
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
        assert!(
            d.size <= 20.0,
            "size={} doit tenir dans la profondeur",
            d.size
        );
    }

    /// L'entrée précoce qui a perdu au run v2 (z=2.6 à 290 s de la fin) doit
    /// désormais être bloquée, alors que le même z tard dans la fenêtre passe.
    #[test]
    fn early_window_requires_higher_z() {
        // z ≈ 2.6 avec 290 s restantes : bloqué (seuil ×~2 en début de fenêtre).
        // (σ = plancher du modèle pour un z prévisible.)
        let mut early = snapshot(80_178.0, 80_000.0, 5e-5, 290.0);
        early.book_up = book(0.52, 500.0, 0.55, 400.0);
        let e = ProbModel::default().estimate(&early);
        assert!(e.z > 2.0 && e.z < 3.5, "z={}", e.z);
        assert!(
            strategy().decide(&early, &e).is_none(),
            "z modéré + 290 s restantes ⇒ refus"
        );

        // Même écart au strike à 30 s de la fin : accepté.
        let mut late = snapshot(80_178.0, 80_000.0, 5e-5, 30.0);
        late.book_up = book(0.52, 500.0, 0.55, 400.0);
        let e2 = ProbModel::default().estimate(&late);
        assert!(
            strategy().decide(&late, &e2).is_some(),
            "même signal à 30 s ⇒ accepté (z={})",
            e2.z
        );
    }

    #[test]
    fn respects_max_notional() {
        let mut cfg = TakerConfig::default();
        cfg.bankroll = 1_000_000.0; // Kelly énorme
        let mut snap = snapshot(80_250.0, 80_000.0, 2e-5, 20.0);
        snap.book_up = book(0.78, 100_000.0, 0.80, 100_000.0);
        let est = ProbModel::default().estimate(&snap);
        let d = TakerStrategy::new(cfg).decide(&snap, &est).unwrap();
        assert!(
            d.size * 0.80 <= cfg.max_notional * 1.01,
            "notional plafonné"
        );
    }
}
