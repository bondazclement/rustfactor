//! Lecture des journaux NDJSON v2 (recorder de pm-acquisition).
//!
//! Chaque ligne est `{"v":2,"recv_ms":..,"mono_ns":..,"stream":..,"raw":..}` ;
//! la trame `raw` est reparsée avec `pm_core::parse` — exactement le code du
//! chemin live.

use anyhow::{Context, Result};
use pm_core::parse::{parse_clob_frame, parse_rtds_frame, RtdsParsed};
use pm_core::{BusEvent, ClobEvent, FastTick, MarketWindow, ResolutionTick};
use serde::Deserialize;
use std::io::BufRead;
use std::path::Path;

#[derive(Debug, Deserialize)]
struct FrameLine {
    v: u8,
    recv_ms: u64,
    #[serde(default)]
    #[allow(dead_code)]
    mono_ns: u64,
    stream: String,
    raw: String,
}

#[derive(Debug, Default)]
pub struct JournalData {
    pub resolution_ticks: Vec<ResolutionTick>,
    pub fast_ticks: Vec<FastTick>,
    pub clob_events: Vec<ClobEvent>,
    pub frames: usize,
    pub unparsed_lines: usize,
}

/// Charge un ou plusieurs segments de journal v2 (ordre chronologique de la
/// liste à la charge de l'appelant — les segments sont nommés par heure UTC).
pub fn load_journal(paths: &[&Path]) -> Result<JournalData> {
    let mut data = JournalData::default();
    for path in paths {
        let file =
            std::fs::File::open(path).with_context(|| format!("ouverture {}", path.display()))?;
        for line in std::io::BufReader::new(file).lines() {
            let line = line?;
            if line.trim().is_empty() {
                continue;
            }
            let Ok(frame) = serde_json::from_str::<FrameLine>(&line) else {
                data.unparsed_lines += 1;
                continue;
            };
            if frame.v != 2 {
                data.unparsed_lines += 1;
                continue;
            }
            data.frames += 1;
            match frame.stream.as_str() {
                "rtds" => match parse_rtds_frame(&frame.raw, frame.recv_ms) {
                    RtdsParsed::Resolution(t) => data.resolution_ticks.push(t),
                    RtdsParsed::Fast(t) => data.fast_ticks.push(t),
                    RtdsParsed::Ignored => {}
                },
                "clob" => data.clob_events.extend(parse_clob_frame(&frame.raw)),
                _ => {}
            }
        }
    }
    Ok(data)
}

/// Rejoue un ou plusieurs segments comme un flux ordonné d'événements bus
/// horodatés par la réception locale — la même séquence que le moteur live.
pub fn load_bus_events(paths: &[&Path]) -> Result<Vec<(u64, BusEvent)>> {
    let mut out: Vec<(u64, BusEvent)> = Vec::new();
    for path in paths {
        let file =
            std::fs::File::open(path).with_context(|| format!("ouverture {}", path.display()))?;
        for line in std::io::BufReader::new(file).lines() {
            let line = line?;
            if line.trim().is_empty() {
                continue;
            }
            let Ok(frame) = serde_json::from_str::<FrameLine>(&line) else {
                continue;
            };
            if frame.v != 2 {
                continue;
            }
            match frame.stream.as_str() {
                "rtds" => match parse_rtds_frame(&frame.raw, frame.recv_ms) {
                    RtdsParsed::Resolution(t) => out.push((frame.recv_ms, BusEvent::Resolution(t))),
                    RtdsParsed::Fast(t) => out.push((frame.recv_ms, BusEvent::Fast(t))),
                    RtdsParsed::Ignored => {}
                },
                "clob" => {
                    for ev in parse_clob_frame(&frame.raw) {
                        out.push((frame.recv_ms, BusEvent::Clob(ev)));
                    }
                }
                "gamma" => {
                    if let Ok(w) = serde_json::from_str::<MarketWindow>(&frame.raw) {
                        out.push((frame.recv_ms, BusEvent::WindowChanged(w)));
                    }
                }
                _ => {}
            }
        }
    }
    out.sort_by_key(|(ts, _)| *ts);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn roundtrip_recorder_format() {
        let dir = std::env::temp_dir().join(format!("pm_v2_test_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("journal_20260508T12.ndjson");
        let mut f = std::fs::File::create(&path).unwrap();
        let rtds_raw = r#"{\"topic\":\"crypto_prices_chainlink\",\"type\":\"update\",\"timestamp\":1778341499450,\"payload\":{\"symbol\":\"btc/usd\",\"timestamp\":1778341499400,\"value\":80466.61}}"#;
        writeln!(
            f,
            r#"{{"v":2,"recv_ms":1778341499500,"mono_ns":1,"stream":"rtds","raw":"{rtds_raw}"}}"#
        )
        .unwrap();
        writeln!(f, r#"{{"v":2,"recv_ms":1778341500600,"mono_ns":2,"stream":"clob","raw":"{{\"event_type\":\"best_bid_ask\",\"asset_id\":\"853\",\"best_bid\":\"0.73\",\"best_ask\":\"0.77\",\"timestamp\":\"1778341500590\"}}"}}"#).unwrap();
        drop(f);

        let data = load_journal(&[&path]).unwrap();
        assert_eq!(data.frames, 2);
        assert_eq!(data.resolution_ticks.len(), 1);
        let t = data.resolution_ticks[0];
        assert_eq!(t.price, 80_466.61);
        assert_eq!(t.source_ts_ms, 1_778_341_499_400);
        assert_eq!(t.recv_ms, 1_778_341_499_500);
        assert_eq!(data.clob_events.len(), 1);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
