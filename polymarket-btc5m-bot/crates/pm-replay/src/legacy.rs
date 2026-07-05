//! Lecture du format legacy Rustector_btc_5mn_1 (`window_<epoch>/raw.ndjson`).
//!
//! Schéma produit par `record.rs` du legacy (serde, tag `kind`) :
//! - `window_changed { ts_ms, window_slug, window_epoch, token_up, token_down }`
//! - `rtds_tick      { ts_ms, source_ts_ms, message_ts_ms, symbol, price, raw_value }`
//! - `clob_event     { ts_ms, event_type, event_ts_ms, payload }`

use anyhow::{Context, Result};
use pm_core::{ClobEvent, ResolutionTick};
use serde::Deserialize;
use serde_json::Value;
use std::io::BufRead;
use std::path::Path;

#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum LegacyEvent {
    WindowChanged {
        ts_ms: u64,
        window_slug: String,
        window_epoch: u64,
        token_up: String,
        token_down: String,
    },
    RtdsTick {
        ts_ms: u64,
        source_ts_ms: u64,
        message_ts_ms: u64,
        symbol: String,
        price: f64,
        #[serde(default)]
        raw_value: f64,
    },
    ClobEvent {
        ts_ms: u64,
        event_type: String,
        event_ts_ms: u64,
        payload: Value,
    },
}

#[derive(Debug, Default)]
pub struct LegacyWindowData {
    pub window_epoch: Option<u64>,
    pub window_slug: Option<String>,
    pub token_up: Option<String>,
    pub token_down: Option<String>,
    pub resolution_ticks: Vec<ResolutionTick>,
    pub clob_events: Vec<ClobEvent>,
    /// Lignes illisibles (comptées, jamais silencieusement ignorées).
    pub unparsed_lines: usize,
}

/// Charge un `raw.ndjson` legacy. `max_lines` optionnel pour les gros fichiers.
pub fn load_legacy_ndjson(path: &Path, max_lines: Option<usize>) -> Result<LegacyWindowData> {
    let file =
        std::fs::File::open(path).with_context(|| format!("ouverture {}", path.display()))?;
    let reader = std::io::BufReader::new(file);
    let mut data = LegacyWindowData::default();
    for (i, line) in reader.lines().enumerate() {
        if max_lines.is_some_and(|m| i >= m) {
            break;
        }
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        match serde_json::from_str::<LegacyEvent>(&line) {
            Ok(LegacyEvent::WindowChanged {
                window_slug,
                window_epoch,
                token_up,
                token_down,
                ..
            }) => {
                data.window_epoch = Some(window_epoch);
                data.window_slug = Some(window_slug);
                data.token_up = Some(token_up);
                data.token_down = Some(token_down);
            }
            Ok(LegacyEvent::RtdsTick {
                ts_ms,
                source_ts_ms,
                message_ts_ms,
                symbol,
                price,
                ..
            }) => {
                if symbol.eq_ignore_ascii_case("btc/usd") && price > 0.0 {
                    data.resolution_ticks.push(ResolutionTick {
                        recv_ms: ts_ms,
                        source_ts_ms,
                        message_ts_ms,
                        price,
                    });
                }
            }
            Ok(LegacyEvent::ClobEvent { payload, .. }) => {
                // Reparse du payload verbatim avec le parseur commun.
                let raw = payload.to_string();
                data.clob_events
                    .extend(pm_core::parse::parse_clob_frame(&raw));
            }
            Err(_) => data.unparsed_lines += 1,
        }
    }
    Ok(data)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn loads_legacy_file() {
        let dir = std::env::temp_dir().join(format!("pm_legacy_test_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("raw.ndjson");
        let mut f = std::fs::File::create(&path).unwrap();
        writeln!(f, r#"{{"kind":"window_changed","ts_ms":1778341500100,"window_slug":"btc-updown-5m-1778341500","window_epoch":1778341500,"token_up":"111","token_down":"222"}}"#).unwrap();
        writeln!(f, r#"{{"kind":"rtds_tick","ts_ms":1778341499500,"source_ts_ms":1778341499400,"message_ts_ms":1778341499450,"symbol":"btc/usd","price":80466.61,"raw_value":80466.61}}"#).unwrap();
        writeln!(f, r#"{{"kind":"rtds_tick","ts_ms":1778341501500,"source_ts_ms":1778341501400,"message_ts_ms":1778341501450,"symbol":"btc/usd","price":80470.00,"raw_value":80470.00}}"#).unwrap();
        writeln!(f, r#"{{"kind":"clob_event","ts_ms":1778341502000,"event_type":"last_trade_price","event_ts_ms":1778341501990,"payload":{{"event_type":"last_trade_price","asset_id":"111","price":"0.55","size":"10","side":"BUY","timestamp":"1778341501990"}}}}"#).unwrap();
        writeln!(f, "ligne corrompue").unwrap();
        drop(f);

        let data = load_legacy_ndjson(&path, None).unwrap();
        assert_eq!(data.window_epoch, Some(1_778_341_500));
        assert_eq!(data.resolution_ticks.len(), 2);
        assert_eq!(data.clob_events.len(), 1);
        assert_eq!(data.unparsed_lines, 1);

        // Strike sur cette mini-archive : dernier tick ≤ T0.
        let cmp = crate::compare_policies(&data.resolution_ticks, 1_778_341_500_000);
        assert_eq!(cmp.last_at_or_before.value, Some(80_466.61));

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn max_lines_truncates() {
        let dir = std::env::temp_dir().join(format!("pm_legacy_trunc_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("raw.ndjson");
        let mut f = std::fs::File::create(&path).unwrap();
        for i in 0..50 {
            writeln!(f, r#"{{"kind":"rtds_tick","ts_ms":{0},"source_ts_ms":{0},"message_ts_ms":{0},"symbol":"btc/usd","price":80000.0,"raw_value":80000.0}}"#, 1_000_000 + i * 1000).unwrap();
        }
        drop(f);
        let data = load_legacy_ndjson(&path, Some(10)).unwrap();
        assert_eq!(data.resolution_ticks.len(), 10);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
