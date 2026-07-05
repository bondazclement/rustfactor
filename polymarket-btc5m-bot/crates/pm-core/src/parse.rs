//! Parsing pur des trames RTDS et CLOB → événements normalisés.
//!
//! Utilisé à l'identique par l'acquisition live (pm-acquisition) et le replay
//! (pm-replay) : aucune divergence possible entre live et backtest. Testé sur
//! les fixtures exactes de la documentation Polymarket. Un échec de parsing ne
//! perd jamais de données : la trame brute est archivée avant parsing.

use crate::events::{ClobEvent, FastTick, Level, PriceChangeLevel, ResolutionTick, Side};
use serde_json::Value;

/// Certains flux historiques encodent les prix en fixed-point 1e18.
fn decode_price(v: f64) -> f64 {
    if v > 1e15 {
        v / 1e18
    } else {
        v
    }
}

fn as_f64(v: Option<&Value>) -> Option<f64> {
    match v? {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => s.parse().ok(),
        _ => None,
    }
}

fn as_u64(v: Option<&Value>) -> Option<u64> {
    match v? {
        Value::Number(n) => n.as_u64(),
        Value::String(s) => s.parse().ok(),
        _ => None,
    }
}

/// Résultat du parsing d'une trame RTDS.
#[derive(Debug, Clone, PartialEq)]
pub enum RtdsParsed {
    Resolution(ResolutionTick),
    Fast(FastTick),
    /// Trame valide mais sans intérêt (autre symbole, PONG, ack…).
    Ignored,
}

/// Parse une trame texte RTDS. `recv_ms` = horloge locale à réception.
pub fn parse_rtds_frame(text: &str, recv_ms: u64) -> RtdsParsed {
    let t = text.trim();
    if t.is_empty() || t == "PONG" {
        return RtdsParsed::Ignored;
    }
    let Ok(msg) = serde_json::from_str::<Value>(t) else {
        return RtdsParsed::Ignored;
    };
    let topic = msg.get("topic").and_then(Value::as_str).unwrap_or("");
    let mtype = msg.get("type").and_then(Value::as_str).unwrap_or("");
    let message_ts_ms = as_u64(msg.get("timestamp")).unwrap_or(0);
    let Some(payload) = msg.get("payload") else {
        return RtdsParsed::Ignored;
    };
    let symbol = payload
        .get("symbol")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_ascii_lowercase();

    match (topic, mtype) {
        ("crypto_prices_chainlink", "update") if symbol == "btc/usd" => {
            let (Some(ts), Some(raw)) = (
                as_u64(payload.get("timestamp")),
                as_f64(payload.get("value")),
            ) else {
                return RtdsParsed::Ignored;
            };
            let price = decode_price(raw);
            if price <= 0.0 {
                return RtdsParsed::Ignored;
            }
            RtdsParsed::Resolution(ResolutionTick {
                recv_ms,
                source_ts_ms: ts,
                message_ts_ms,
                price,
            })
        }
        ("crypto_prices", "update") if symbol == "btcusdt" => {
            let (Some(ts), Some(raw)) = (
                as_u64(payload.get("timestamp")),
                as_f64(payload.get("value")),
            ) else {
                return RtdsParsed::Ignored;
            };
            let price = decode_price(raw);
            if price <= 0.0 {
                return RtdsParsed::Ignored;
            }
            RtdsParsed::Fast(FastTick {
                recv_ms,
                source_ts_ms: ts,
                price,
            })
        }
        _ => RtdsParsed::Ignored,
    }
}

fn parse_levels(v: Option<&Value>) -> Vec<Level> {
    v.and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(|l| {
                    Some(Level {
                        price: as_f64(l.get("price"))?,
                        size: as_f64(l.get("size"))?,
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Parse une trame texte du canal market CLOB. Une trame peut contenir un
/// événement seul ou un tableau d'événements.
pub fn parse_clob_frame(text: &str) -> Vec<ClobEvent> {
    let t = text.trim();
    if t.is_empty() || t == "PONG" {
        return vec![];
    }
    let Ok(val) = serde_json::from_str::<Value>(t) else {
        return vec![];
    };
    let items: Vec<&Value> = match &val {
        Value::Array(a) => a.iter().collect(),
        v => vec![v],
    };
    items.into_iter().filter_map(parse_clob_event).collect()
}

fn parse_clob_event(v: &Value) -> Option<ClobEvent> {
    let ts_ms = as_u64(v.get("timestamp")).unwrap_or(0);
    match v.get("event_type").and_then(Value::as_str)? {
        "book" => Some(ClobEvent::Book {
            asset_id: v.get("asset_id")?.as_str()?.to_string(),
            ts_ms,
            bids: parse_levels(v.get("bids")),
            asks: parse_levels(v.get("asks")),
        }),
        "price_change" => {
            let changes = v
                .get("price_changes")?
                .as_array()?
                .iter()
                .filter_map(|c| {
                    Some(PriceChangeLevel {
                        asset_id: c.get("asset_id")?.as_str()?.to_string(),
                        price: as_f64(c.get("price"))?,
                        size: as_f64(c.get("size"))?,
                        side: Side::parse(c.get("side")?.as_str()?)?,
                    })
                })
                .collect();
            Some(ClobEvent::PriceChange { ts_ms, changes })
        }
        "last_trade_price" => Some(ClobEvent::LastTrade {
            asset_id: v.get("asset_id")?.as_str()?.to_string(),
            ts_ms,
            price: as_f64(v.get("price"))?,
            size: as_f64(v.get("size"))?,
            side: Side::parse(v.get("side")?.as_str()?)?,
        }),
        "best_bid_ask" => Some(ClobEvent::BestBidAsk {
            asset_id: v.get("asset_id")?.as_str()?.to_string(),
            ts_ms,
            best_bid: as_f64(v.get("best_bid")),
            best_ask: as_f64(v.get("best_ask")),
        }),
        "tick_size_change" => Some(ClobEvent::TickSizeChange {
            asset_id: v.get("asset_id")?.as_str()?.to_string(),
            ts_ms,
            new_tick_size: as_f64(v.get("new_tick_size"))?,
        }),
        "market_resolved" => Some(ClobEvent::MarketResolved {
            slug: v
                .get("slug")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            ts_ms,
            winning_asset_id: v
                .get("winning_asset_id")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            winning_outcome: v
                .get("winning_outcome")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
        }),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Fixtures verbatim de la documentation Polymarket (MCP docs, market-channel.mdx / rtds.mdx).

    #[test]
    fn rtds_chainlink_update() {
        let frame = r#"{"topic":"crypto_prices_chainlink","type":"update","timestamp":1753314088421,"payload":{"symbol":"btc/usd","timestamp":1753314088395,"value":67234.50}}"#;
        let RtdsParsed::Resolution(t) = parse_rtds_frame(frame, 1753314088460) else {
            panic!("attendu Resolution")
        };
        assert_eq!(t.source_ts_ms, 1753314088395);
        assert_eq!(t.message_ts_ms, 1753314088421);
        assert_eq!(t.recv_ms, 1753314088460);
        assert_eq!(t.price, 67234.50);
    }

    #[test]
    fn rtds_fixed_point_1e18_is_decoded() {
        let frame = r#"{"topic":"crypto_prices_chainlink","type":"update","timestamp":1,"payload":{"symbol":"btc/usd","timestamp":2,"value":80466610000000000000000.0}}"#;
        let RtdsParsed::Resolution(t) = parse_rtds_frame(frame, 3) else {
            panic!()
        };
        assert!((t.price - 80_466.61).abs() < 1e-6);
    }

    #[test]
    fn rtds_btcusdt_is_fast_not_resolution() {
        let frame = r#"{"topic":"crypto_prices","type":"update","timestamp":1753314088421,"payload":{"symbol":"btcusdt","timestamp":1753314088395,"value":67234.50}}"#;
        assert!(matches!(parse_rtds_frame(frame, 0), RtdsParsed::Fast(_)));
    }

    #[test]
    fn rtds_other_symbols_and_pong_ignored() {
        let eth = r#"{"topic":"crypto_prices_chainlink","type":"update","timestamp":1,"payload":{"symbol":"eth/usd","timestamp":2,"value":3456.78}}"#;
        assert_eq!(parse_rtds_frame(eth, 0), RtdsParsed::Ignored);
        assert_eq!(parse_rtds_frame("PONG", 0), RtdsParsed::Ignored);
        assert_eq!(parse_rtds_frame("", 0), RtdsParsed::Ignored);
        assert_eq!(parse_rtds_frame("not json", 0), RtdsParsed::Ignored);
    }

    #[test]
    fn clob_book_fixture() {
        let frame = r#"{
          "event_type": "book",
          "asset_id": "65818619657568813474341868652308942079804919287380422192892211131408793125422",
          "market": "0xbd31dc8a20211944f6b70f31557f1001557b59905b7738480ca09bd4532f84af",
          "bids": [{ "price": ".48", "size": "30" },{ "price": ".49", "size": "20" },{ "price": ".50", "size": "15" }],
          "asks": [{ "price": ".52", "size": "25" },{ "price": ".53", "size": "60" },{ "price": ".54", "size": "10" }],
          "timestamp": "123456789000",
          "hash": "0x0...."
        }"#;
        let evs = parse_clob_frame(frame);
        assert_eq!(evs.len(), 1);
        let ClobEvent::Book {
            bids, asks, ts_ms, ..
        } = &evs[0]
        else {
            panic!()
        };
        assert_eq!(*ts_ms, 123_456_789_000);
        assert_eq!(bids.len(), 3);
        assert_eq!(asks.len(), 3);
        assert_eq!(bids[0].price, 0.48); // ".48" doit parser
        assert_eq!(asks[2].size, 10.0);
    }

    #[test]
    fn clob_price_change_fixture() {
        let frame = r#"{
          "market": "0x5f65...",
          "price_changes": [
            {"asset_id": "71321045679252212594626385532706912750332728571942532289631379312455583992563","price": "0.5","size": "200","side": "BUY","best_bid": "0.5","best_ask": "1"},
            {"asset_id": "52114319501245915516055106046884209969926127482827954674443846427813813222426","price": "0.5","size": "0","side": "SELL","best_bid": "0","best_ask": "0.5"}
          ],
          "timestamp": "1757908892351",
          "event_type": "price_change"
        }"#;
        let evs = parse_clob_frame(frame);
        let ClobEvent::PriceChange { ts_ms, changes } = &evs[0] else {
            panic!()
        };
        assert_eq!(*ts_ms, 1_757_908_892_351);
        assert_eq!(changes.len(), 2);
        assert_eq!(changes[0].side, Side::Buy);
        assert_eq!(changes[1].size, 0.0); // suppression de niveau
    }

    #[test]
    fn clob_array_frame_and_unknown_types() {
        let frame = r#"[
          {"event_type":"last_trade_price","asset_id":"114","fee_rate_bps":"0","market":"0x6a","price":"0.456","side":"BUY","size":"219.217767","timestamp":"1750428146322"},
          {"event_type":"best_bid_ask","market":"0x00","asset_id":"853","best_bid":"0.73","best_ask":"0.77","spread":"0.04","timestamp":"1766789469958"},
          {"event_type":"unknown_future_event","foo":1}
        ]"#;
        let evs = parse_clob_frame(frame);
        assert_eq!(
            evs.len(),
            2,
            "l'événement inconnu est ignoré au parsing (mais archivé en brut)"
        );
        assert!(matches!(&evs[0], ClobEvent::LastTrade { price, .. } if *price == 0.456));
        assert!(
            matches!(&evs[1], ClobEvent::BestBidAsk { best_bid: Some(b), best_ask: Some(a), .. } if *b == 0.73 && *a == 0.77)
        );
    }

    #[test]
    fn clob_market_resolved_fixture() {
        let frame = r#"{
          "id":"1031769","question":"q","market":"0x311d","slug":"btc-updown-5m-1778341500",
          "assets_ids":["76","31"],"outcomes":["Up","Down"],
          "winning_asset_id":"76","winning_outcome":"Up",
          "timestamp":"1766790415550","event_type":"market_resolved"
        }"#;
        let evs = parse_clob_frame(frame);
        let ClobEvent::MarketResolved {
            slug,
            winning_outcome,
            ..
        } = &evs[0]
        else {
            panic!()
        };
        assert_eq!(slug, "btc-updown-5m-1778341500");
        assert_eq!(winning_outcome, "Up");
    }
}
