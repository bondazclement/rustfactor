//! pm-core — types et logique pure du bot btc-updown-5m.
//!
//! Tout ce module est indépendant du réseau : chaque composant se teste
//! unitairement et se rejoue depuis les archives NDJSON (`pm-replay`).

pub mod book;
pub mod events;
pub mod math;
pub mod parse;
pub mod strike;
pub mod vol;
pub mod window;

pub use events::*;
