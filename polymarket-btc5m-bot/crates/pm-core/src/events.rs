//! Événements normalisés partagés par l'acquisition, le replay et les stratégies.
//!
//! Règle de fidélité : ces types sont *dérivés* des trames brutes archivées ;
//! l'archive NDJSON v2 conserve toujours la trame verbatim (voir pm-acquisition).

use serde::{Deserialize, Serialize};

/// Fenêtre de marché active (découverte via Gamma).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MarketWindow {
    pub slug: String,
    pub epoch_s: u64,
    pub start_ms: u64,
    pub end_ms: u64,
    pub condition_id: String,
    pub token_up: String,
    pub token_down: String,
    pub neg_risk: bool,
    pub tick_size: f64,
}

/// Tick du flux de résolution (RTDS `crypto_prices_chainlink` btc/usd).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ResolutionTick {
    /// Horloge locale à la réception (ms).
    pub recv_ms: u64,
    /// `payload.timestamp` : horodatage du report Chainlink (ms). C'est LA
    /// référence temporelle pour le strike et la volatilité.
    pub source_ts_ms: u64,
    /// `timestamp` du message RTDS (ms) — sert au diagnostic de latence.
    pub message_ts_ms: u64,
    pub price: f64,
}

/// Tick indicateur rapide (RTDS `crypto_prices` btcusdt). Jamais utilisé pour
/// la résolution — uniquement comme indicateur avancé éventuel.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct FastTick {
    pub recv_ms: u64,
    pub source_ts_ms: u64,
    pub price: f64,
}

/// Côté d'un ordre / d'un niveau.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Side {
    Buy,
    Sell,
}

impl Side {
    pub fn parse(s: &str) -> Option<Side> {
        match s {
            "BUY" | "buy" => Some(Side::Buy),
            "SELL" | "sell" => Some(Side::Sell),
            _ => None,
        }
    }
}

/// Un niveau de prix L2.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Level {
    pub price: f64,
    pub size: f64,
}

/// Événements CLOB normalisés (canal market).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ClobEvent {
    /// Snapshot complet du carnet pour un token.
    Book {
        asset_id: String,
        ts_ms: u64,
        bids: Vec<Level>,
        asks: Vec<Level>,
    },
    /// Delta de niveaux (size 0 = suppression du niveau).
    PriceChange {
        ts_ms: u64,
        changes: Vec<PriceChangeLevel>,
    },
    /// Trade exécuté.
    LastTrade {
        asset_id: String,
        ts_ms: u64,
        price: f64,
        size: f64,
        side: Side,
    },
    /// Changement de meilleur bid/ask (custom_feature_enabled).
    BestBidAsk {
        asset_id: String,
        ts_ms: u64,
        best_bid: Option<f64>,
        best_ask: Option<f64>,
    },
    /// Changement de tick size (prix aux extrêmes).
    TickSizeChange {
        asset_id: String,
        ts_ms: u64,
        new_tick_size: f64,
    },
    /// Résolution officielle du marché — vérité terrain Up/Down.
    MarketResolved {
        slug: String,
        ts_ms: u64,
        winning_asset_id: String,
        winning_outcome: String,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PriceChangeLevel {
    pub asset_id: String,
    pub price: f64,
    pub size: f64,
    pub side: Side,
}

/// Événement unifié transitant sur le bus interne du bot.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum BusEvent {
    WindowChanged(MarketWindow),
    Resolution(ResolutionTick),
    Fast(FastTick),
    Clob(ClobEvent),
    /// Un flux est considéré silencieux/dégradé (watchdog).
    FeedStale {
        stream: String,
        silent_ms: u64,
    },
}
