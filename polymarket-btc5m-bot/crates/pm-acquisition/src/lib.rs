//! pm-acquisition — module 1 : acquisition 100 % fidèle des flux Polymarket.
//!
//! Principes non négociables :
//! 1. **Archive avant parsing** : chaque trame WebSocket est journalisée
//!    verbatim (NDJSON v2) avant toute interprétation. Une évolution de format
//!    côté Polymarket ne peut donc jamais faire perdre de données.
//! 2. **Résolution native uniquement** : le flux de résolution est
//!    RTDS `crypto_prices_chainlink` `btc/usd`. Le flux `btcusdt` est archivé
//!    comme indicateur avancé mais n'alimente ni strike ni volatilité.
//! 3. **Abonnement continu** : pas de rotation de connexion à la frontière de
//!    fenêtre pour le flux de résolution — les ticks encadrant T0 sont
//!    toujours capturés ; le découpage par fenêtre est fait en aval.

pub mod binance;
pub mod bus;
pub mod clob;
pub mod gamma;
pub mod net;
pub mod recorder;
pub use pm_core::parse;
pub mod rtds;
pub mod watchdog;

pub use bus::Bus;
pub use recorder::{RawFrame, Recorder};

/// Endpoints officiels (docs Polymarket).
pub const WS_RTDS_URL: &str = "wss://ws-live-data.polymarket.com";
pub const WS_CLOB_MARKET_URL: &str = "wss://ws-subscriptions-clob.polymarket.com/ws/market";
pub const GAMMA_BASE_URL: &str = "https://gamma-api.polymarket.com";

pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
