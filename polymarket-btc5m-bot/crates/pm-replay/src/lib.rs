//! pm-replay — lecture des archives et validation hors ligne.
//!
//! Deux formats supportés :
//! - **legacy** : `window_<epoch>/raw.ndjson` produit par Rustector_btc_5mn_1
//!   (lignes `{"kind":"window_changed"|"rtds_tick"|"clob_event",...}`),
//! - **v2**     : segments `journal_*.ndjson` du recorder de pm-acquisition
//!   (lignes `{"v":2,"stream":...,"raw":...}` — la trame est reparsée avec
//!   exactement le même code que le live : `pm_acquisition::parse`).
//!
//! C'est le banc d'essai des politiques de strike (`strike-validate`) et la
//! base du backtest des stratégies : le live et le replay passent par les
//! mêmes types (`pm_core::BusEvent`).

pub mod legacy;
pub mod v2;

use pm_core::strike::{compute_strike, StrikeComputation, StrikePolicy, DEFAULT_CONFIDENCE_GAP_MS};
use pm_core::ResolutionTick;

/// Compare les trois politiques de strike sur une frontière donnée.
#[derive(Debug, Clone)]
pub struct StrikeComparison {
    pub t0_ms: u64,
    pub last_at_or_before: StrikeComputation,
    pub first_at_or_after: StrikeComputation,
    pub interpolate: StrikeComputation,
}

pub fn compare_policies(ticks: &[ResolutionTick], t0_ms: u64) -> StrikeComparison {
    StrikeComparison {
        t0_ms,
        last_at_or_before: compute_strike(
            ticks,
            t0_ms,
            StrikePolicy::LastAtOrBefore,
            DEFAULT_CONFIDENCE_GAP_MS,
        ),
        first_at_or_after: compute_strike(
            ticks,
            t0_ms,
            StrikePolicy::FirstAtOrAfter,
            DEFAULT_CONFIDENCE_GAP_MS,
        ),
        interpolate: compute_strike(
            ticks,
            t0_ms,
            StrikePolicy::Interpolate,
            DEFAULT_CONFIDENCE_GAP_MS,
        ),
    }
}

/// Statistiques d'inter-arrivée des ticks de résolution (cadence réelle du
/// flux Chainlink — indispensable pour calibrer confiance et volatilité).
#[derive(Debug, Clone, Default)]
pub struct TickCadence {
    pub count: usize,
    pub min_dt_ms: u64,
    pub max_dt_ms: u64,
    pub mean_dt_ms: f64,
    pub p50_dt_ms: u64,
    pub p99_dt_ms: u64,
    /// Latence médiane réception locale - timestamp source (ms).
    pub median_recv_lag_ms: i64,
}

pub fn tick_cadence(ticks: &[ResolutionTick]) -> TickCadence {
    if ticks.len() < 2 {
        return TickCadence {
            count: ticks.len(),
            ..Default::default()
        };
    }
    let mut sorted: Vec<&ResolutionTick> = ticks.iter().collect();
    sorted.sort_by_key(|t| t.source_ts_ms);
    let mut dts: Vec<u64> = sorted
        .windows(2)
        .map(|w| w[1].source_ts_ms.saturating_sub(w[0].source_ts_ms))
        .collect();
    dts.sort_unstable();
    let mut lags: Vec<i64> = sorted
        .iter()
        .map(|t| t.recv_ms as i64 - t.source_ts_ms as i64)
        .collect();
    lags.sort_unstable();
    let sum: u64 = dts.iter().sum();
    TickCadence {
        count: ticks.len(),
        min_dt_ms: dts[0],
        max_dt_ms: *dts.last().unwrap(),
        mean_dt_ms: sum as f64 / dts.len() as f64,
        p50_dt_ms: dts[dts.len() / 2],
        p99_dt_ms: dts[(dts.len() * 99) / 100],
        median_recv_lag_ms: lags[lags.len() / 2],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tick(ts: u64, price: f64) -> ResolutionTick {
        ResolutionTick {
            recv_ms: ts + 50,
            source_ts_ms: ts,
            message_ts_ms: ts + 25,
            price,
        }
    }

    #[test]
    fn cadence_stats() {
        let ticks: Vec<ResolutionTick> = (0..100)
            .map(|i| tick(1_000_000 + i * 1_000, 80_000.0))
            .collect();
        let c = tick_cadence(&ticks);
        assert_eq!(c.count, 100);
        assert_eq!(c.min_dt_ms, 1_000);
        assert_eq!(c.p50_dt_ms, 1_000);
        assert_eq!(c.median_recv_lag_ms, 50);
    }

    #[test]
    fn policies_disagree_when_boundary_between_ticks() {
        let t0 = 1_778_341_500_000u64;
        let ticks = vec![tick(t0 - 700, 80_466.61), tick(t0 + 900, 80_470.00)];
        let cmp = compare_policies(&ticks, t0);
        assert_eq!(cmp.last_at_or_before.value, Some(80_466.61));
        assert_eq!(cmp.first_at_or_after.value, Some(80_470.00));
        let interp = cmp.interpolate.value.unwrap();
        assert!(interp > 80_466.61 && interp < 80_470.00);
    }
}
