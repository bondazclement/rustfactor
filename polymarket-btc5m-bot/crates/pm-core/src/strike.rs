//! Reconstruction du « price to beat » (strike) d'une fenêtre.
//!
//! Constat Phase 1 (docs/PHASE1_FINDINGS.md) :
//! - le strike affiché par Polymarket peut différer du premier tick reçu dans
//!   la fenêtre (fenêtre 1778341500 : $80,466.61 ≠ open) ;
//! - l'interpolation linéaire de l'ancienne version python introduisait un
//!   biais (+$0.06 sur 1778343900 : 80,714.93 enregistré vs 80,714.87 réel).
//!
//! Hypothèse par défaut : **strike = dernier update Chainlink dont
//! `source_ts_ms ≤ T0`** (le prix « courant » au moment de l'ouverture).
//! Les politiques alternatives restent disponibles pour le banc de validation
//! (`pm-replay strike-validate`) qui tranchera sur données réelles.

use crate::events::ResolutionTick;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum StrikePolicy {
    /// Dernier tick avec source_ts_ms ≤ T0 (défaut, hypothèse Polymarket UI).
    LastAtOrBefore,
    /// Premier tick avec source_ts_ms ≥ T0.
    FirstAtOrAfter,
    /// Interpolation linéaire entre les deux (legacy python, pour comparaison).
    Interpolate,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum StrikeStatus {
    /// Tick exactement à T0 ou encadrement serré : valeur sûre.
    Exact,
    /// Valeur déterminée mais avec un trou de données autour de T0.
    Approx,
    /// Impossible à déterminer (pas de ticks exploitables).
    Unresolved,
}

/// Strike reconstruit + éléments de preuve (audit / validation).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StrikeComputation {
    pub value: Option<f64>,
    pub status: StrikeStatus,
    pub policy: StrikePolicy,
    /// 1.0 = certain ; décroît avec l'écart temporel au tick utilisé.
    pub confidence: f64,
    pub before: Option<ResolutionTick>,
    pub after: Option<ResolutionTick>,
    /// Écart (ms) entre T0 et le tick effectivement utilisé.
    pub used_gap_ms: Option<u64>,
}

/// Trou de données au-delà duquel la confiance devient nulle.
pub const DEFAULT_CONFIDENCE_GAP_MS: u64 = 10_000;

/// Calcule le strike pour la frontière `t0_ms` à partir des ticks de résolution.
///
/// Les ticks n'ont pas besoin d'être triés ; seuls `source_ts_ms` et `price`
/// sont utilisés (l'horloge locale ne compte pas pour la frontière).
pub fn compute_strike(
    ticks: &[ResolutionTick],
    t0_ms: u64,
    policy: StrikePolicy,
    confidence_gap_ms: u64,
) -> StrikeComputation {
    let before = ticks
        .iter()
        .filter(|t| t.source_ts_ms <= t0_ms)
        .max_by_key(|t| t.source_ts_ms)
        .copied();
    let after = ticks
        .iter()
        .filter(|t| t.source_ts_ms >= t0_ms)
        .min_by_key(|t| t.source_ts_ms)
        .copied();

    let unresolved = || StrikeComputation {
        value: None,
        status: StrikeStatus::Unresolved,
        policy,
        confidence: 0.0,
        before,
        after,
        used_gap_ms: None,
    };

    let conf = |gap_ms: u64| -> f64 {
        (1.0 - gap_ms as f64 / confidence_gap_ms.max(1) as f64).clamp(0.0, 1.0)
    };

    match policy {
        StrikePolicy::LastAtOrBefore => {
            // Le tick pile à T0 est le cas idéal (gap 0, Exact).
            let Some(b) = before else {
                // Dégradé : aucun tick avant T0 (démarrage tardif) → premier après,
                // confiance plafonnée.
                let Some(a) = after else { return unresolved() };
                let gap = a.source_ts_ms - t0_ms;
                return StrikeComputation {
                    value: Some(a.price),
                    status: StrikeStatus::Approx,
                    policy,
                    confidence: conf(gap).min(0.5),
                    before,
                    after,
                    used_gap_ms: Some(gap),
                };
            };
            let gap = t0_ms - b.source_ts_ms;
            StrikeComputation {
                value: Some(b.price),
                status: if gap == 0 {
                    StrikeStatus::Exact
                } else {
                    StrikeStatus::Approx
                },
                policy,
                confidence: conf(gap),
                before,
                after,
                used_gap_ms: Some(gap),
            }
        }
        StrikePolicy::FirstAtOrAfter => {
            let Some(a) = after else { return unresolved() };
            let gap = a.source_ts_ms - t0_ms;
            StrikeComputation {
                value: Some(a.price),
                status: if gap == 0 {
                    StrikeStatus::Exact
                } else {
                    StrikeStatus::Approx
                },
                policy,
                confidence: conf(gap),
                before,
                after,
                used_gap_ms: Some(gap),
            }
        }
        StrikePolicy::Interpolate => match (before, after) {
            (Some(b), Some(a)) if a.source_ts_ms > b.source_ts_ms => {
                let span = (a.source_ts_ms - b.source_ts_ms) as f64;
                let ratio = ((t0_ms - b.source_ts_ms) as f64 / span).clamp(0.0, 1.0);
                let value = b.price + (a.price - b.price) * ratio;
                let gap = a.source_ts_ms - b.source_ts_ms;
                StrikeComputation {
                    value: Some(value),
                    status: if gap == 0 {
                        StrikeStatus::Exact
                    } else {
                        StrikeStatus::Approx
                    },
                    policy,
                    confidence: conf(gap),
                    before,
                    after,
                    used_gap_ms: Some(gap),
                }
            }
            (Some(b), _) if b.source_ts_ms == t0_ms => StrikeComputation {
                value: Some(b.price),
                status: StrikeStatus::Exact,
                policy,
                confidence: 1.0,
                before,
                after,
                used_gap_ms: Some(0),
            },
            _ => unresolved(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tick(source_ts_ms: u64, price: f64) -> ResolutionTick {
        ResolutionTick {
            recv_ms: source_ts_ms + 40,
            source_ts_ms,
            message_ts_ms: source_ts_ms + 20,
            price,
        }
    }

    const T0: u64 = 1_778_343_900_000;

    /// Scénario reproduisant la fenêtre 1778343900 : le vrai strike affiché
    /// ($80,714.87) est le dernier tick avant T0 ; l'interpolation legacy
    /// surestimait (~80,714.93).
    #[test]
    fn last_at_or_before_matches_observed_ui_value() {
        let ticks = vec![
            tick(T0 - 3_000, 80_710.00),
            tick(T0 - 800, 80_714.87), // dernier connu à T0
            tick(T0 + 400, 80_715.05), // premier après
            tick(T0 + 1_600, 80_716.20),
        ];
        let c = compute_strike(
            &ticks,
            T0,
            StrikePolicy::LastAtOrBefore,
            DEFAULT_CONFIDENCE_GAP_MS,
        );
        assert_eq!(c.value, Some(80_714.87));
        assert_eq!(c.status, StrikeStatus::Approx);
        assert!(c.confidence > 0.9);
        assert_eq!(c.used_gap_ms, Some(800));

        // La politique legacy donne une valeur plus haute (biais observé).
        let i = compute_strike(
            &ticks,
            T0,
            StrikePolicy::Interpolate,
            DEFAULT_CONFIDENCE_GAP_MS,
        );
        assert!(i.value.unwrap() > 80_714.87 && i.value.unwrap() < 80_715.05);

        // FirstAtOrAfter diverge aussi (≠ price to beat affiché, cf. 1778341500).
        let f = compute_strike(
            &ticks,
            T0,
            StrikePolicy::FirstAtOrAfter,
            DEFAULT_CONFIDENCE_GAP_MS,
        );
        assert_eq!(f.value, Some(80_715.05));
    }

    #[test]
    fn exact_tick_at_boundary() {
        let ticks = vec![
            tick(T0 - 900, 80_000.0),
            tick(T0, 80_466.61),
            tick(T0 + 500, 80_470.0),
        ];
        let c = compute_strike(
            &ticks,
            T0,
            StrikePolicy::LastAtOrBefore,
            DEFAULT_CONFIDENCE_GAP_MS,
        );
        assert_eq!(c.value, Some(80_466.61));
        assert_eq!(c.status, StrikeStatus::Exact);
        assert_eq!(c.confidence, 1.0);
    }

    #[test]
    fn degraded_no_tick_before() {
        let ticks = vec![tick(T0 + 2_000, 80_100.0)];
        let c = compute_strike(
            &ticks,
            T0,
            StrikePolicy::LastAtOrBefore,
            DEFAULT_CONFIDENCE_GAP_MS,
        );
        assert_eq!(c.value, Some(80_100.0));
        assert_eq!(c.status, StrikeStatus::Approx);
        assert!(c.confidence <= 0.5, "confiance plafonnée en mode dégradé");
    }

    #[test]
    fn unresolved_without_ticks() {
        let c = compute_strike(
            &[],
            T0,
            StrikePolicy::LastAtOrBefore,
            DEFAULT_CONFIDENCE_GAP_MS,
        );
        assert_eq!(c.status, StrikeStatus::Unresolved);
        assert_eq!(c.value, None);
    }

    #[test]
    fn confidence_decays_with_gap() {
        let near = vec![tick(T0 - 500, 1.0)];
        let far = vec![tick(T0 - 8_000, 1.0)];
        let cn = compute_strike(
            &near,
            T0,
            StrikePolicy::LastAtOrBefore,
            DEFAULT_CONFIDENCE_GAP_MS,
        );
        let cf = compute_strike(
            &far,
            T0,
            StrikePolicy::LastAtOrBefore,
            DEFAULT_CONFIDENCE_GAP_MS,
        );
        assert!(cn.confidence > cf.confidence);
    }
}
