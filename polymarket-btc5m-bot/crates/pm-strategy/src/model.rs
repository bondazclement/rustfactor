//! Modèle probabiliste : P(résolution = Up) à partir de l'état reconstruit.
//!
//! Hypothèse de base : sur 0–5 min, le log-prix suit une diffusion avec
//! drift lent — S_T = S·exp((μ−σ²/2)τ + σ√τ·Z). D'où
//!
//!   P(S_T > K) = Φ( (ln(S/K) + (μ − σ²/2)·τ) / (σ·√τ) )
//!
//! avec σ (par √s) et μ (par s) reconstruits EXCLUSIVEMENT depuis le flux de
//! résolution Chainlink (pm-core::vol). Le z-score associé mesure
//! « l'incohérence » exploitée par le taker : un |z| élevé proche de la
//! résolution avec un prix de marché encore loin de 0/1 est l'opportunité
//! décrite dans le cahier des charges (ex. +250 $ à 20 s de la fin quand la
//! vol des 30 dernières minutes est ±30 $).

use pm_core::book::OrderBook;
use pm_core::math::{norm_cdf, student_t_cdf};
use pm_core::strike::StrikeComputation;
use serde::{Deserialize, Serialize};

/// Loi utilisée pour transformer z en probabilité.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Dist {
    /// Gaussienne — historique, massivement surconfiante sur ce flux
    /// (docs/ETUDE_MODELE.md) ; conservée pour comparaison au backtest.
    Gauss,
    /// Student-t à `student_nu` degrés de liberté — queues épaisses,
    /// cohérente avec la kurtosis mesurée (~238 sur r_1s).
    Student,
}

/// Instantané complet transmis aux stratégies. Construit par l'orchestrateur
/// (live) ou par le replayer (backtest) — même structure, même code décision.
#[derive(Debug, Clone)]
pub struct MarketSnapshot {
    pub now_ms: u64,
    /// Frontières de la fenêtre courante.
    pub t0_ms: u64,
    pub t_end_ms: u64,
    /// Strike reconstruit (politique LastAtOrBefore) + preuve.
    pub strike: StrikeComputation,
    /// Dernier prix de résolution (Chainlink) et son horodatage source.
    pub spot: f64,
    pub spot_source_ts_ms: u64,
    /// σ par √seconde (EWMA) et μ par seconde, depuis pm-core::vol.
    pub sigma_per_sqrt_s: Option<f64>,
    pub drift_per_s: Option<f64>,
    /// Carnets des deux tokens.
    pub book_up: OrderBook,
    pub book_down: OrderBook,
    /// Vrai si un des flux critiques est stale (watchdog) — coupe le risque.
    pub any_feed_stale: bool,
}

impl MarketSnapshot {
    /// Temps restant avant résolution (secondes).
    pub fn tau_s(&self) -> f64 {
        (self.t_end_ms.saturating_sub(self.now_ms)) as f64 / 1000.0
    }

    /// Âge du dernier tick de résolution (ms).
    pub fn spot_age_ms(&self) -> u64 {
        self.now_ms.saturating_sub(self.spot_source_ts_ms)
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ProbConfig {
    /// Plancher de σ (par √s) : évite les z infinis quand la vol mesurée est
    /// quasi nulle. ~0.5 bp/√s ≈ 4 $/√s sur BTC à 80 k$.
    pub sigma_floor_per_sqrt_s: f64,
    /// τ plancher (s) : en dessous, la diffusion n'a plus de sens ; on passe
    /// en mode quasi-déterministe (le spot a « gagné » sauf retournement).
    pub tau_floor_s: f64,
    /// Ignorer le drift si |μ|·τ < cette fraction de σ√τ (bruit).
    pub drift_snr_min: f64,
    /// Plafond de la contribution du drift au z (en unités de σ√τ).
    /// Leçon du 2026-07-04 23:15 : extrapoler un crash de 120 s sur toute la
    /// fenêtre a fabriqué z=−5,6 sur un écart au strike de 12 $ → perte de
    /// 258 $ au rebond. Le drift informe, il ne doit jamais dominer.
    pub max_drift_z: f64,
    /// Loi z → probabilité (`student` par défaut, `gauss` pour comparaison).
    pub dist: Dist,
    /// Degrés de liberté de la Student-t. ν≈2 reproduit les probabilités de
    /// tenue mesurées empiriquement (P(tenir z=3) ≈ 0,95 vs 0,999 gaussien).
    pub student_nu: f64,
    /// Active la correction par la table de calibration auto-apprise
    /// (pm_strategy::calib) quand l'orchestrateur en fournit une.
    pub calibration: bool,
}

impl Default for ProbConfig {
    fn default() -> Self {
        Self {
            sigma_floor_per_sqrt_s: 5e-5,
            tau_floor_s: 1.0,
            drift_snr_min: 0.25,
            max_drift_z: 2.0,
            dist: Dist::Student,
            student_nu: 2.0,
            calibration: true,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ProbEstimate {
    /// P(Up) ∈ (0,1).
    pub p_up: f64,
    /// z = distance signée au strike en unités de σ√τ (>0 ⇒ Up favorisé).
    pub z: f64,
    /// spot − strike EN DOLLARS (signé). La variable robuste du trader
    /// humain : insensible au bruit d'estimation de σ (étude 4 du 06/07).
    pub dist_usd: f64,
    pub sigma_used: f64,
    pub tau_s: f64,
    /// Faux si un ingrédient manquait (σ absent, strike non résolu…).
    pub reliable: bool,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct ProbModel {
    pub cfg: ProbConfig,
}

impl ProbModel {
    pub fn new(cfg: ProbConfig) -> Self {
        Self { cfg }
    }

    pub fn estimate(&self, snap: &MarketSnapshot) -> ProbEstimate {
        let tau = snap.tau_s().max(self.cfg.tau_floor_s);
        let strike = snap.strike.value;
        let sigma_raw = snap.sigma_per_sqrt_s;
        let (Some(k), Some(sigma)) = (strike, sigma_raw) else {
            // Ingrédient manquant : estimation neutre, non fiable.
            return ProbEstimate {
                p_up: 0.5,
                z: 0.0,
                dist_usd: 0.0,
                sigma_used: 0.0,
                tau_s: tau,
                reliable: false,
            };
        };
        if snap.spot <= 0.0 || k <= 0.0 {
            return ProbEstimate {
                p_up: 0.5,
                z: 0.0,
                dist_usd: 0.0,
                sigma_used: 0.0,
                tau_s: tau,
                reliable: false,
            };
        }
        let sigma = sigma.max(self.cfg.sigma_floor_per_sqrt_s);
        let x = (snap.spot / k).ln();
        // Drift : uniquement s'il domine le bruit (sinon il ajoute de la variance
        // d'estimation sans information).
        let mu = snap.drift_per_s.unwrap_or(0.0);
        let denom = sigma * tau.sqrt();
        let drift_term = (mu - sigma * sigma / 2.0) * tau;
        let mut effective_drift = if drift_term.abs() >= self.cfg.drift_snr_min * denom {
            drift_term
        } else {
            0.0
        };
        // Plafonnement : le drift ne peut pas contribuer plus de max_drift_z
        // unités de z (sinon un choc récent extrapolé fabrique une certitude).
        let cap = self.cfg.max_drift_z * denom;
        effective_drift = effective_drift.clamp(-cap, cap);
        let z = (x + effective_drift) / denom;
        let p_up = match self.cfg.dist {
            Dist::Gauss => norm_cdf(z),
            Dist::Student => student_t_cdf(z, self.cfg.student_nu),
        };
        ProbEstimate {
            p_up,
            z,
            dist_usd: snap.spot - k,
            sigma_used: sigma,
            tau_s: tau,
            reliable: snap.strike.confidence > 0.0 && !snap.any_feed_stale,
        }
    }

    /// Applique la correction empirique de la table de calibration :
    /// remplace p_up par le postérieur bayésien du bac (z, τ) visité,
    /// avec la probabilité paramétrique comme prior.
    pub fn calibrer(&self, est: &mut ProbEstimate, table: &crate::calib::CalibTable) {
        if !self.cfg.calibration || !est.reliable {
            return;
        }
        let p_prior = est.p_up.max(1.0 - est.p_up);
        let p_cal = table.p_win(est.dist_usd.abs(), est.tau_s, p_prior);
        est.p_up = if est.dist_usd >= 0.0 { p_cal } else { 1.0 - p_cal };
    }
}

#[cfg(test)]
pub(crate) mod test_support {
    use super::*;
    use pm_core::strike::{StrikeComputation, StrikePolicy, StrikeStatus};
    use pm_core::Level;

    pub fn strike(value: f64, confidence: f64) -> StrikeComputation {
        StrikeComputation {
            value: Some(value),
            status: StrikeStatus::Exact,
            policy: StrikePolicy::LastAtOrBefore,
            confidence,
            before: None,
            after: None,
            used_gap_ms: Some(0),
        }
    }

    pub fn book(bid: f64, bid_sz: f64, ask: f64, ask_sz: f64) -> OrderBook {
        let mut ob = OrderBook::new();
        ob.apply_snapshot(
            &[Level {
                price: bid,
                size: bid_sz,
            }],
            &[Level {
                price: ask,
                size: ask_sz,
            }],
            0,
            0,
        );
        ob
    }

    /// Fenêtre de 5 min, T0 = 1_778_341_500_000, spot frais, flux sains.
    pub fn snapshot(spot: f64, strike_v: f64, sigma: f64, seconds_left: f64) -> MarketSnapshot {
        let t0: u64 = 1_778_341_500_000;
        let t_end = t0 + 300_000;
        let now = t_end - (seconds_left * 1000.0) as u64;
        MarketSnapshot {
            now_ms: now,
            t0_ms: t0,
            t_end_ms: t_end,
            strike: strike(strike_v, 1.0),
            spot,
            spot_source_ts_ms: now.saturating_sub(300),
            sigma_per_sqrt_s: Some(sigma),
            drift_per_s: None,
            book_up: book(0.48, 100.0, 0.52, 100.0),
            book_down: book(0.46, 100.0, 0.50, 100.0),
            any_feed_stale: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::snapshot;
    use super::*;

    #[test]
    fn at_the_money_is_half() {
        let m = ProbModel::default();
        let e = m.estimate(&snapshot(80_000.0, 80_000.0, 1e-4, 150.0));
        assert!((e.p_up - 0.5).abs() < 0.02, "p_up={}", e.p_up);
        assert!(e.reliable);
    }

    #[test]
    fn scenario_cahier_des_charges() {
        // « à 20 s de la résolution, le BTC augmente de 250 $, alors que la
        //   vol moyenne était de ±30 $ » (sur 30 min).
        // ±30 $ sur 80 k$ ≈ 37 bps par fenêtre de 30 min ; σ/√s ≈ 37bps/√1800
        //  ≈ 0.88 bp/√s → 8.8e-6... prenons σ réaliste 2e-5 /√s.
        let spot = 80_250.0;
        let strike = 80_000.0;
        let e = ProbModel::default().estimate(&snapshot(spot, strike, 2e-5, 20.0));
        assert!(e.z > 10.0, "z={} devrait être énorme", e.z);
        // Student-t : « très probable » mais jamais la fausse certitude
        // gaussienne (>0,999) qui a coûté cher (docs/ETUDE_MODELE.md).
        assert!(e.p_up > 0.99, "p_up={} quasi certain", e.p_up);
        assert!(e.p_up < 0.9999, "p_up={} pas une certitude", e.p_up);
    }

    #[test]
    fn student_less_confident_than_gauss() {
        let snap = snapshot(80_100.0, 80_000.0, 1e-4, 60.0);
        let student = ProbModel::default().estimate(&snap);
        let gauss = ProbModel::new(ProbConfig {
            dist: Dist::Gauss,
            ..Default::default()
        })
        .estimate(&snap);
        assert!(student.z == gauss.z, "même z, seule la loi change");
        assert!(
            student.p_up < gauss.p_up,
            "student {} < gauss {}",
            student.p_up,
            gauss.p_up
        );
    }

    #[test]
    fn calibration_table_overrides_parametric_p() {
        use crate::calib::{CalibTable, FenetrePending};
        let m = ProbModel::default();
        let snap = snapshot(80_150.0, 80_000.0, 1e-4, 60.0);
        let mut est = m.estimate(&snap);
        assert!(est.z > 2.0 && est.z < 6.0, "z={}", est.z);
        let p_avant = est.p_up;
        // Table nourrie : ce bac ne gagne que ~55 % du temps.
        let mut t = CalibTable::default();
        for i in 0..100 {
            let mut p = FenetrePending::default();
            p.observer(est.dist_usd, est.tau_s);
            t.regler_fenetre(&p, i % 20 < 11);
        }
        m.calibrer(&mut est, &t);
        assert!(est.p_up < p_avant, "calibré {} < brut {}", est.p_up, p_avant);
        assert!(est.p_up > 0.5, "reste du côté favori");
        // Estimation non fiable : jamais recalibrée.
        let mut bad = m.estimate(&snap);
        bad.reliable = false;
        let p0 = bad.p_up;
        m.calibrer(&mut bad, &t);
        assert_eq!(bad.p_up, p0);
    }

    #[test]
    fn more_time_means_less_certainty() {
        let m = ProbModel::default();
        let near = m.estimate(&snapshot(80_100.0, 80_000.0, 1e-4, 10.0));
        let far = m.estimate(&snapshot(80_100.0, 80_000.0, 1e-4, 290.0));
        assert!(near.p_up > far.p_up, "près de la fin, l'avance compte plus");
        assert!(far.p_up > 0.5);
    }

    #[test]
    fn below_strike_favors_down() {
        let e = ProbModel::default().estimate(&snapshot(79_900.0, 80_000.0, 1e-4, 60.0));
        assert!(e.p_up < 0.5);
        assert!(e.z < 0.0);
    }

    #[test]
    fn missing_sigma_is_unreliable_neutral() {
        let mut s = snapshot(80_100.0, 80_000.0, 1e-4, 60.0);
        s.sigma_per_sqrt_s = None;
        let e = ProbModel::default().estimate(&s);
        assert_eq!(e.p_up, 0.5);
        assert!(!e.reliable);
    }

    #[test]
    fn stale_feed_is_unreliable() {
        let mut s = snapshot(80_100.0, 80_000.0, 1e-4, 60.0);
        s.any_feed_stale = true;
        let e = ProbModel::default().estimate(&s);
        assert!(!e.reliable);
    }

    #[test]
    fn sigma_floor_prevents_infinite_z() {
        let e = ProbModel::default().estimate(&snapshot(80_001.0, 80_000.0, 1e-12, 100.0));
        assert!(e.z.is_finite());
        assert!(e.p_up < 1.0);
    }
}
