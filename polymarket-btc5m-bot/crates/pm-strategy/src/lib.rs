//! pm-strategy — modules 2 (taker) et 3 (market maker).
//!
//! Les décisions sont des **fonctions pures** sur un instantané de marché
//! (`MarketSnapshot`) : mêmes entrées ⇒ même décision, donc backtestables au
//! tick près via pm-replay et unitaires sans réseau.
//!
//! ⚠️ Les seuils par défaut sont des points de départ raisonnés, PAS des
//! valeurs calibrées : la calibration exige les archives réelles
//! (data_low_latency) — voir docs/PHASE1_FINDINGS.md. Les garde-fous
//! (fraîcheur des flux, confiance du strike, liquidité) sont en revanche
//! fermes et ne doivent jamais être assouplis.

pub mod calib;
pub mod config;
pub mod maker;
pub mod model;
pub mod paper;
pub mod taker;

pub use model::{MarketSnapshot, ProbModel};
