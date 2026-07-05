//! Configuration centrale du bot — TOUT est configurable ici.
//!
//! Source de vérité : un fichier TOML (voir `config.exemple.toml` à la
//! racine, chaque paramètre y est documenté en français). Toute section et
//! tout champ sont optionnels : les valeurs absentes prennent les défauts
//! calibrés du code (`Default`). Les surcharges en ligne de commande
//! (`--max-entry`, `--bankroll`, …) s'appliquent APRÈS le fichier.
//!
//! Le même fichier alimente le live (`pm-bot --config`) et le backtest
//! (`pm-backtest --config`) : ce qui est testé est ce qui tourne.

use crate::maker::MakerConfig;
use crate::model::ProbConfig;
use crate::taker::TakerConfig;
use anyhow::{Context, Result};
use pm_core::vol::VolConfig;
use serde::{Deserialize, Serialize};
use std::path::Path;

/// Paramètres du moteur d'orchestration (hors stratégie).
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct MoteurConfig {
    /// Cadence d'évaluation des décisions (ms).
    pub decision_step_ms: u64,
    /// Silence d'un flux au-delà duquel le watchdog le déclare périmé (ms).
    pub watchdog_stale_ms: u64,
    /// Temps de grâce après la fin de fenêtre pour capter market_resolved (s).
    pub clob_grace_s: u64,
    /// Fenêtres Gamma sondées en avant lors de la découverte.
    pub gamma_lookahead: u64,
    /// Rétention des ticks de résolution en mémoire (s) — sert au strike ET
    /// à la fenêtre de volatilité la plus longue.
    pub retention_ticks_s: u64,
}

impl Default for MoteurConfig {
    fn default() -> Self {
        Self {
            decision_step_ms: 250,
            watchdog_stale_ms: 6_000,
            clob_grace_s: 180,
            gamma_lookahead: 5,
            retention_ticks_s: 2_400,
        }
    }
}

/// Configuration complète du bot. Chaque section correspond à une table
/// TOML : `[taker]`, `[maker]`, `[modele]`, `[volatilite]`, `[moteur]`.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct BotConfig {
    pub taker: TakerConfig,
    pub maker: MakerConfig,
    pub modele: ProbConfig,
    pub volatilite: VolConfig,
    pub moteur: MoteurConfig,
}

impl BotConfig {
    /// Charge un fichier TOML (toutes sections/clefs optionnelles).
    pub fn charger(path: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("lecture de {}", path.display()))?;
        let cfg: BotConfig = toml::from_str(&text)
            .with_context(|| format!("parse TOML de {}", path.display()))?;
        Ok(cfg)
    }

    /// Charge `path` s'il existe, sinon les défauts calibrés.
    pub fn charger_ou_defaut(path: &Path) -> Result<Self> {
        if path.exists() {
            Self::charger(path)
        } else {
            Ok(Self::default())
        }
    }

    /// Rendu TOML complet de la configuration EFFECTIVE (audit/logs).
    pub fn en_toml(&self) -> String {
        toml::to_string_pretty(self).unwrap_or_else(|e| format!("<erreur toml: {e}>"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_calibrated_values() {
        let c = BotConfig::default();
        assert_eq!(c.taker.max_entry_price, 0.85);
        assert_eq!(c.taker.bankroll, 1_000.0);
        assert_eq!(c.taker.max_notional, 250.0);
        assert_eq!(c.modele.max_drift_z, 2.0);
        assert_eq!(c.moteur.decision_step_ms, 250);
    }

    #[test]
    fn partial_toml_keeps_defaults_elsewhere() {
        let cfg: BotConfig = toml::from_str(
            r#"
            [taker]
            bankroll = 5000.0
            max_notional = 100.0

            [modele]
            max_drift_z = 1.0
            "#,
        )
        .unwrap();
        // Champs fournis…
        assert_eq!(cfg.taker.bankroll, 5_000.0);
        assert_eq!(cfg.taker.max_notional, 100.0);
        assert_eq!(cfg.modele.max_drift_z, 1.0);
        // …le reste garde les défauts calibrés.
        assert_eq!(cfg.taker.min_abs_z, 2.5);
        assert_eq!(cfg.taker.max_entry_price, 0.85);
        assert_eq!(cfg.maker.take_profit, 0.08);
        assert_eq!(cfg.volatilite.ewma_half_life_s, 60.0);
        assert_eq!(cfg.moteur.watchdog_stale_ms, 6_000);
    }

    #[test]
    fn roundtrip_toml() {
        let c = BotConfig::default();
        let s = c.en_toml();
        let back: BotConfig = toml::from_str(&s).unwrap();
        assert_eq!(back.taker.min_edge, c.taker.min_edge);
        assert_eq!(back.maker.stop_loss, c.maker.stop_loss);
    }

    #[test]
    fn unknown_key_is_rejected() {
        // Protège contre les fautes de frappe silencieuses dans config.toml.
        let r: Result<BotConfig, _> = toml::from_str("[taker]\nbankrol = 10.0\n");
        assert!(r.is_err(), "clé inconnue = erreur explicite");
    }
}
