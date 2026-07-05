//! Volatilité reconstruite **exclusivement** depuis le flux de résolution
//! (ticks Chainlink RTDS). Aucune source externe (règle absolue du projet).
//!
//! Les updates Chainlink arrivent à intervalles irréguliers : chaque rendement
//! log est normalisé par son intervalle de temps pour produire une variance
//! par seconde, agrégée de deux façons :
//! - fenêtres glissantes de variance réalisée (30 s / 120 s / 300 s / 1800 s),
//! - EWMA type RiskMetrics adaptée au temps irrégulier (poids λ^Δt).
//!
//! Toutes les valeurs sont en « sigma par √seconde » sur le log-prix, donc la
//! projection sur un horizon τ est σ·√τ.

use crate::events::ResolutionTick;
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct VolConfig {
    /// Demi-vie de l'EWMA en secondes.
    pub ewma_half_life_s: f64,
    /// Rétention maximale des ticks (secondes).
    pub retention_s: u64,
    /// Intervalle minimal (ms) pour éviter les divisions par ~0 sur des
    /// doublons de timestamp.
    pub min_dt_ms: u64,
}

impl Default for VolConfig {
    fn default() -> Self {
        Self {
            ewma_half_life_s: 60.0,
            retention_s: 1800,
            min_dt_ms: 10,
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct Sample {
    ts_ms: u64,
    log_price: f64,
    /// Rendement log depuis l'échantillon précédent.
    ret: f64,
    /// Δt (s) depuis l'échantillon précédent.
    dt_s: f64,
}

#[derive(Debug, Clone)]
pub struct VolEstimator {
    cfg: VolConfig,
    samples: VecDeque<Sample>,
    last: Option<(u64, f64)>, // (ts_ms, log_price)
    /// Variance par seconde, EWMA.
    ewma_var_per_s: Option<f64>,
}

impl VolEstimator {
    pub fn new(cfg: VolConfig) -> Self {
        Self {
            cfg,
            samples: VecDeque::new(),
            last: None,
            ewma_var_per_s: None,
        }
    }

    pub fn clear(&mut self) {
        self.samples.clear();
        self.last = None;
        self.ewma_var_per_s = None;
    }

    /// Ingestion d'un tick de résolution. Les ticks doivent arriver en ordre
    /// de `source_ts_ms` croissant ; les retours en arrière sont ignorés.
    pub fn push(&mut self, tick: &ResolutionTick) {
        if tick.price <= 0.0 {
            return;
        }
        let lp = tick.price.ln();
        let Some((prev_ts, prev_lp)) = self.last else {
            self.last = Some((tick.source_ts_ms, lp));
            return;
        };
        if tick.source_ts_ms <= prev_ts + self.cfg.min_dt_ms.saturating_sub(1) {
            // Doublon ou régression d'horloge : on met à jour le dernier prix
            // sans créer de rendement dégénéré.
            self.last = Some((prev_ts.max(tick.source_ts_ms), lp));
            return;
        }
        let dt_s = (tick.source_ts_ms - prev_ts) as f64 / 1000.0;
        let ret = lp - prev_lp;
        self.samples.push_back(Sample {
            ts_ms: tick.source_ts_ms,
            log_price: lp,
            ret,
            dt_s,
        });
        self.last = Some((tick.source_ts_ms, lp));

        // EWMA à temps irrégulier : λ = exp(-ln2 · Δt / half_life).
        let var_inst = ret * ret / dt_s; // variance par seconde instantanée
        let lambda = (-(std::f64::consts::LN_2) * dt_s / self.cfg.ewma_half_life_s).exp();
        self.ewma_var_per_s = Some(match self.ewma_var_per_s {
            Some(v) => lambda * v + (1.0 - lambda) * var_inst,
            None => var_inst,
        });

        // Purge de la rétention.
        let cutoff = tick
            .source_ts_ms
            .saturating_sub(self.cfg.retention_s * 1000);
        while self.samples.front().is_some_and(|s| s.ts_ms < cutoff) {
            self.samples.pop_front();
        }
    }

    /// σ par √seconde (EWMA). None tant qu'aucun rendement n'a été observé.
    pub fn ewma_sigma_per_sqrt_s(&self) -> Option<f64> {
        self.ewma_var_per_s.map(f64::sqrt)
    }

    /// σ par √seconde réalisé sur la fenêtre `window_s` se terminant à `now_ms`
    /// (temps source). None si moins de `min_returns` rendements dans la fenêtre.
    pub fn realized_sigma_per_sqrt_s(
        &self,
        window_s: u64,
        now_ms: u64,
        min_returns: usize,
    ) -> Option<f64> {
        let cutoff = now_ms.saturating_sub(window_s * 1000);
        let mut sum_sq = 0.0;
        let mut sum_dt = 0.0;
        let mut n = 0usize;
        for s in self.samples.iter().rev() {
            if s.ts_ms < cutoff {
                break;
            }
            sum_sq += s.ret * s.ret;
            sum_dt += s.dt_s;
            n += 1;
        }
        if n < min_returns || sum_dt <= 0.0 {
            return None;
        }
        Some((sum_sq / sum_dt).sqrt())
    }

    /// Drift (rendement log par seconde) réalisé sur la fenêtre.
    pub fn realized_drift_per_s(&self, window_s: u64, now_ms: u64) -> Option<f64> {
        let cutoff = now_ms.saturating_sub(window_s * 1000);
        let in_window: Vec<&Sample> = self.samples.iter().filter(|s| s.ts_ms >= cutoff).collect();
        let (first, last) = (in_window.first()?, in_window.last()?);
        let dt = (last.ts_ms.saturating_sub(first.ts_ms)) as f64 / 1000.0;
        if dt <= 0.0 {
            return None;
        }
        Some((last.log_price - first.log_price) / dt)
    }

    /// Projection : écart-type du log-rendement à horizon `tau_s`.
    pub fn projected_sigma(&self, tau_s: f64) -> Option<f64> {
        Some(self.ewma_sigma_per_sqrt_s()? * tau_s.max(0.0).sqrt())
    }

    pub fn sample_count(&self) -> usize {
        self.samples.len()
    }

    pub fn last_source_ts_ms(&self) -> Option<u64> {
        self.last.map(|(ts, _)| ts)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tick(ts_ms: u64, price: f64) -> ResolutionTick {
        ResolutionTick {
            recv_ms: ts_ms,
            source_ts_ms: ts_ms,
            message_ts_ms: ts_ms,
            price,
        }
    }

    /// Marche ±b en alternance à pas de 1 s : la variance par seconde doit
    /// converger vers b² (en log).
    #[test]
    fn realized_sigma_matches_constructed_process() {
        let mut v = VolEstimator::new(VolConfig::default());
        let p0: f64 = 80_000.0;
        let b = 0.0005; // 5 bps par pas
        let mut lp = p0.ln();
        let t0: u64 = 1_778_341_500_000;
        v.push(&tick(t0, lp.exp()));
        for i in 1..=600u64 {
            lp += if i % 2 == 0 { b } else { -b };
            v.push(&tick(t0 + i * 1000, lp.exp()));
        }
        let now = t0 + 600_000;
        let sigma = v.realized_sigma_per_sqrt_s(300, now, 30).unwrap();
        assert!((sigma - b).abs() / b < 0.05, "sigma={sigma}, attendu ~{b}");
        let ewma = v.ewma_sigma_per_sqrt_s().unwrap();
        assert!((ewma - b).abs() / b < 0.10, "ewma={ewma}, attendu ~{b}");
    }

    #[test]
    fn irregular_spacing_is_normalized() {
        // Même processus, mais un tick sur deux manque : Δt = 2 s, rendement 2b.
        // La variance par seconde doit rester ~ (2b)²/2 = 2b² → σ = b√2.
        let mut v = VolEstimator::new(VolConfig::default());
        let b = 0.0005;
        let t0: u64 = 1_778_341_500_000;
        let mut lp = 80_000.0_f64.ln();
        v.push(&tick(t0, lp.exp()));
        for i in 1..=300u64 {
            lp += if i % 2 == 0 { 2.0 * b } else { -2.0 * b };
            v.push(&tick(t0 + i * 2000, lp.exp()));
        }
        let sigma = v.realized_sigma_per_sqrt_s(600, t0 + 600_000, 30).unwrap();
        let expected = b * 2.0_f64.sqrt();
        assert!(
            (sigma - expected).abs() / expected < 0.05,
            "sigma={sigma} attendu ~{expected}"
        );
    }

    #[test]
    fn duplicate_timestamps_do_not_blow_up() {
        let mut v = VolEstimator::new(VolConfig::default());
        let t0: u64 = 1_000_000;
        v.push(&tick(t0, 100.0));
        v.push(&tick(t0, 101.0)); // doublon de timestamp
        v.push(&tick(t0 + 1000, 100.5));
        assert!(v.ewma_sigma_per_sqrt_s().unwrap().is_finite());
    }

    #[test]
    fn drift_sign() {
        let mut v = VolEstimator::new(VolConfig::default());
        let t0: u64 = 1_000_000;
        for i in 0..=60u64 {
            v.push(&tick(t0 + i * 1000, 80_000.0 + 10.0 * i as f64));
        }
        let d = v.realized_drift_per_s(60, t0 + 60_000).unwrap();
        assert!(d > 0.0);
    }

    #[test]
    fn projection_scales_with_sqrt_tau() {
        let mut v = VolEstimator::new(VolConfig::default());
        let t0: u64 = 1_000_000;
        let mut p = 80_000.0;
        for i in 1..=120u64 {
            p += if i % 2 == 0 { 40.0 } else { -40.0 };
            v.push(&tick(t0 + i * 1000, p));
        }
        let s1 = v.projected_sigma(25.0).unwrap();
        let s2 = v.projected_sigma(100.0).unwrap();
        assert!((s2 / s1 - 2.0).abs() < 1e-9);
    }
}
