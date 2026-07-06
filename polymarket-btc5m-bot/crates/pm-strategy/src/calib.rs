//! Calibration empirique auto-apprise de la probabilité.
//!
//! Constat (docs/ETUDE_MODELE.md) : aucune forme paramétrique ne capture le
//! vrai processus Chainlink (kurtosis ≈ 238). La seule probabilité honnête
//! est MESURÉE : « parmi les états passés (écart au strike EN DOLLARS dans
//! ce bac, τ dans ce bac), quelle fraction du côté favori a gagné ? »
//!
//! v2 (étude 4 du 06/07) : les bacs sont indexés sur l'ÉCART EN DOLLARS,
//! pas sur z — un z élevé sur un écart de 20 $ est un artefact du bruit
//! d'estimation de σ (tous les états perdants mesurés vivaient là), alors
//! que l'écart en dollars est robuste. C'est la variable du trader humain.
//!
//! Mécanique :
//! - pendant la fenêtre, chaque état visité (bac z × bac τ, côté favori)
//!   est mémorisé UNE fois (dédupliqué : les secondes successives d'un même
//!   état sont corrélées, les compter toutes fausserait les effectifs) ;
//! - au règlement, chaque état visité devient une observation win/lose de
//!   son bac → mise à jour bayésienne Beta-binomiale ;
//! - la probabilité servie est le postérieur : (wins + k·p_prior) / (n + k),
//!   où p_prior vient de la loi paramétrique (Student-t) et k = poids du
//!   prior. Bac vide ⇒ prior pur ; bac riche ⇒ données pures.
//!
//! La table est persistée en JSON : elle survit aux redémarrages et
//! s'améliore continûment (auto-calibration en ligne). Le backtest peut la
//! reconstruire depuis les archives (`pm-backtest --rebuild-calib`).

use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::Path;

/// Bornes des bacs (bord droit exclusif, dernier bac ouvert).
/// Écart |spot − strike| en dollars.
pub const DIST_BINS: [f64; 7] = [10.0, 20.0, 35.0, 50.0, 75.0, 100.0, 150.0];
pub const TAU_BINS: [f64; 7] = [3.0, 15.0, 30.0, 60.0, 120.0, 180.0, 240.0];

/// Indices (bac écart, bac τ) d'un état — la grille du mode avancé.
pub fn indices(dist_usd: f64, tau_s: f64) -> Option<(usize, usize)> {
    Some((bin_idx(&DIST_BINS, dist_usd)?, bin_idx(&TAU_BINS, tau_s)?))
}

fn bin_idx(bounds: &[f64], v: f64) -> Option<usize> {
    if v < bounds[0] {
        return None;
    }
    Some(bounds.iter().rposition(|b| v >= *b).unwrap_or(0))
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
pub struct Cell {
    pub wins: f64,
    pub losses: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CalibTable {
    /// cells[iz][itau]
    pub cells: Vec<Vec<Cell>>,
    /// Poids du prior paramétrique (équivalent-observations).
    pub prior_strength: f64,
    /// Nombre de fenêtres réglées ayant nourri la table.
    pub windows_observed: u64,
    pub version: u32,
}

impl Default for CalibTable {
    fn default() -> Self {
        Self {
            cells: vec![vec![Cell::default(); TAU_BINS.len()]; DIST_BINS.len()],
            prior_strength: 30.0,
            windows_observed: 0,
            version: 2,
        }
    }
}

impl CalibTable {
    pub fn charger_ou_defaut(path: &Path) -> Self {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .filter(|t: &CalibTable| {
                t.version == 2
                    && t.cells.len() == DIST_BINS.len()
                    && t.cells.iter().all(|r| r.len() == TAU_BINS.len())
            })
            .unwrap_or_default()
    }

    pub fn sauver(&self, path: &Path) -> anyhow::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(path, serde_json::to_string_pretty(self)?)?;
        Ok(())
    }

    /// Probabilité calibrée que le CÔTÉ FAVORI (signe de l'écart) gagne.
    /// `dist_usd` : |spot − strike| en dollars ;
    /// `p_prior` : probabilité paramétrique du même événement (≥ 0,5).
    pub fn p_win(&self, dist_usd: f64, tau_s: f64, p_prior: f64) -> f64 {
        let (Some(iz), Some(it)) = (bin_idx(&DIST_BINS, dist_usd), bin_idx(&TAU_BINS, tau_s)) else {
            return p_prior;
        };
        let c = self.cells[iz][it];
        let n = c.wins + c.losses;
        let k = self.prior_strength;
        ((c.wins + k * p_prior) / (n + k)).clamp(0.001, 0.999)
    }

    /// Effectif du bac (pour journalisation/diagnostic).
    pub fn effectif(&self, dist_usd: f64, tau_s: f64) -> f64 {
        match (bin_idx(&DIST_BINS, dist_usd), bin_idx(&TAU_BINS, tau_s)) {
            (Some(iz), Some(it)) => {
                let c = self.cells[iz][it];
                c.wins + c.losses
            }
            _ => 0.0,
        }
    }

    /// Intègre les états visités d'une fenêtre réglée.
    pub fn regler_fenetre(&mut self, pending: &FenetrePending, up_won: bool) {
        for &(iz, it, favori_up) in &pending.states {
            let won = favori_up == up_won;
            let c = &mut self.cells[iz][it];
            if won {
                c.wins += 1.0;
            } else {
                c.losses += 1.0;
            }
        }
        if !pending.states.is_empty() {
            self.windows_observed += 1;
        }
    }

    pub fn total_observations(&self) -> f64 {
        self.cells
            .iter()
            .flatten()
            .map(|c| c.wins + c.losses)
            .sum()
    }
}

/// États (bac z, bac τ, côté favori) visités par la fenêtre en cours,
/// dédupliqués. Vidé/réglé à chaque rotation de fenêtre.
#[derive(Debug, Default)]
pub struct FenetrePending {
    states: HashSet<(usize, usize, bool)>,
}

impl FenetrePending {
    /// `dist_signee` : spot − strike en dollars (le signe donne le favori).
    pub fn observer(&mut self, dist_signee: f64, tau_s: f64) {
        if let (Some(iz), Some(it)) =
            (bin_idx(&DIST_BINS, dist_signee.abs()), bin_idx(&TAU_BINS, tau_s))
        {
            self.states.insert((iz, it, dist_signee >= 0.0));
        }
    }

    pub fn clear(&mut self) {
        self.states.clear();
    }

    pub fn len(&self) -> usize {
        self.states.len()
    }

    pub fn is_empty(&self) -> bool {
        self.states.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_table_returns_prior() {
        let t = CalibTable::default();
        assert_eq!(t.p_win(60.0, 45.0, 0.93), 0.93);
    }

    #[test]
    fn observations_pull_probability_toward_reality() {
        let mut t = CalibTable::default();
        // 60 fenêtres où le favori (z≈2,7, τ≈45 s) n'a gagné que 60 % du
        // temps alors que le prior paramétrique disait 95 %.
        for i in 0..60 {
            let mut p = FenetrePending::default();
            p.observer(60.0, 45.0);
            t.regler_fenetre(&p, i % 5 < 3); // favori up, up gagne 3/5
        }
        let p = t.p_win(60.0, 45.0, 0.95);
        assert!(p < 0.75, "p={p} doit être tiré vers 0,60");
        assert!(p > 0.60, "p={p} garde une part de prior");
        // Un bac jamais visité reste au prior.
        assert_eq!(t.p_win(500.0, 200.0, 0.99), 0.99);
    }

    #[test]
    fn pending_dedupes_correlated_seconds() {
        let mut p = FenetrePending::default();
        for _ in 0..100 {
            p.observer(60.0, 45.0); // 100 secondes du même état
        }
        p.observer(60.0, 10.0); // bac τ différent
        p.observer(-60.0, 45.0); // côté différent
        assert_eq!(p.len(), 3);
    }

    #[test]
    fn roundtrip_json() {
        let mut t = CalibTable::default();
        let mut p = FenetrePending::default();
        p.observer(80.0, 100.0);
        t.regler_fenetre(&p, true);
        let dir = std::env::temp_dir().join("pm_calib_test");
        let path = dir.join("calibration.json");
        t.sauver(&path).unwrap();
        let back = CalibTable::charger_ou_defaut(&path);
        assert_eq!(back.windows_observed, 1);
        assert_eq!(back.total_observations(), 1.0);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn corrupt_or_missing_file_falls_back_to_default() {
        let t = CalibTable::charger_ou_defaut(Path::new("/nonexistent/x.json"));
        assert_eq!(t.windows_observed, 0);
    }

    #[test]
    fn below_range_dist_returns_prior() {
        let t = CalibTable::default();
        assert_eq!(t.p_win(5.0, 45.0, 0.62), 0.62);
    }
}
