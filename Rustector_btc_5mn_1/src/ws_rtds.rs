use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::Result;
use futures_util::{SinkExt, StreamExt};
use tokio::sync::mpsc;
use tokio::time;
use tokio_tungstenite::{connect_async, tungstenite::Message};

use crate::models::{AppState, ChainlinkTick};
use crate::record::RecordEvent;

const WS_RTDS: &str = "wss://ws-live-data.polymarket.com";
const RECV_TIMEOUT_SECS: f64 = 5.5;

fn decode_price(value: f64) -> f64 {
    if value > 1e15 {
        value / 1e18
    } else {
        value
    }
}

pub async fn run_rtds_ws(
    state: Arc<Mutex<AppState>>,
    recorder: Option<mpsc::UnboundedSender<RecordEvent>>,
) {
    loop {
        match connect_and_stream(state.clone(), recorder.clone()).await {
            Ok(_) => eprintln!("[RTDS] disconnected"),
            Err(e) => eprintln!("[RTDS] error: {}", e),
        }
        time::sleep(Duration::from_secs(2)).await;
    }
}

async fn connect_and_stream(
    state: Arc<Mutex<AppState>>,
    recorder: Option<mpsc::UnboundedSender<RecordEvent>>,
) -> Result<()> {
    let (ws_stream, _) = connect_async(WS_RTDS).await?;
    let (mut write, mut read) = ws_stream.split();
    let sub_cl =
        r#"{"action":"subscribe","subscriptions":[{"topic":"crypto_prices_chainlink","type":"*","filters":""}]}"#;
    let sub_bin =
        r#"{"action":"subscribe","subscriptions":[{"topic":"crypto_prices","type":"update","filters":"btcusdt"}]}"#;
    write.send(Message::Text(sub_cl.into())).await?;
    write.send(Message::Text(sub_bin.into())).await?;

    let mut ping = time::interval(Duration::from_secs(5));
    let mut refresh = time::interval(Duration::from_secs_f64(RECV_TIMEOUT_SECS));
    let mut last_data_ms = now_ms();
    loop {
        tokio::select! {
            msg = read.next() => {
                match msg {
                    Some(Ok(Message::Text(text))) => {
                        if text.trim().is_empty() || text.trim() == "PONG" { continue; }
                        last_data_ms = now_ms();
                        process_rtds_message(&text, &state, recorder.clone());
                    }
                    Some(Ok(Message::Binary(bin))) => {
                        let text = String::from_utf8_lossy(&bin).to_string();
                        if text.trim().is_empty() || text.trim() == "PONG" { continue; }
                        last_data_ms = now_ms();
                        process_rtds_message(&text, &state, recorder.clone());
                    }
                    Some(Ok(Message::Ping(data))) => {
                        write.send(Message::Pong(data)).await.ok();
                    }
                    Some(Ok(Message::Close(_))) | None => return Err(anyhow::anyhow!("RTDS closed")),
                    _ => {}
                }
            }
            _ = ping.tick() => {
                write.send(Message::Text("PING".into())).await.ok();
            }
            _ = refresh.tick() => {
                if now_ms().saturating_sub(last_data_ms) >= 5500 {
                    write.send(Message::Text(sub_cl.into())).await.ok();
                    write.send(Message::Text(sub_bin.into())).await.ok();
                }
            }
        }
    }
}

fn process_rtds_message(
    text: &str,
    state: &Arc<Mutex<AppState>>,
    recorder: Option<mpsc::UnboundedSender<RecordEvent>>,
) {
    let msg: serde_json::Value = match serde_json::from_str(text) {
        Ok(v) => v,
        Err(_) => return,
    };
    let topic = msg.get("topic").and_then(|v| v.as_str()).unwrap_or("");
    let msg_type = msg.get("type").and_then(|v| v.as_str()).unwrap_or("");
    let ts_rtds = msg.get("timestamp").and_then(|v| v.as_u64()).unwrap_or(0);
    let payload = match msg.get("payload") {
        Some(p) => p,
        None => return,
    };

    match topic {
        "crypto_prices_chainlink" if msg_type == "update" => {
            let symbol = payload.get("symbol").and_then(|v| v.as_str()).unwrap_or("");
            if symbol.to_ascii_lowercase() != "btc/usd" {
                return;
            }
            let ts_cl = payload.get("timestamp").and_then(|v| v.as_u64()).unwrap_or(0);
            let raw_val = payload.get("value").and_then(|v| v.as_f64()).unwrap_or(0.0);
            let price = decode_price(raw_val);
            if price <= 0.0 {
                return;
            }
            let recv = now_ms();
            if let Ok(mut st) = state.lock() {
                st.btc_price_chainlink = Some(price);
                st.chainlink_timestamp_ms = ts_cl;
                st.last_ws_rtds_ms = recv;
                st.rtds_latency_ms = recv as i64 - ts_rtds as i64;
                st.chainlink_ticks.push_back(ChainlinkTick { ts_ms: ts_cl, price });
                while st.chainlink_ticks.len() > 1024 {
                    st.chainlink_ticks.pop_front();
                }
                if let Some(m) = st.market.clone() {
                    st.update_chainlink_candle(m.end_date_ms.saturating_sub(300_000), m.end_date_ms);
                }
                st.rtds_tick_count_window += 1;
            }
            if let Some(tx) = recorder {
                let _ = tx.send(RecordEvent::RtdsTick {
                    ts_ms: recv,
                    source_ts_ms: ts_cl,
                    message_ts_ms: ts_rtds,
                    symbol: "btc/usd".to_string(),
                    price,
                    raw_value: raw_val,
                });
            }
        }
        "crypto_prices" if msg_type == "update" => {
            let symbol = payload.get("symbol").and_then(|v| v.as_str()).unwrap_or("");
            if symbol.to_ascii_lowercase() != "btcusdt" {
                return;
            }
            let ts = payload.get("timestamp").and_then(|v| v.as_u64()).unwrap_or(0);
            let raw = payload.get("value").and_then(|v| v.as_f64()).unwrap_or(0.0);
            let price = decode_price(raw);
            if let Ok(mut st) = state.lock() {
                st.btc_price_binance = Some(price);
                st.last_ws_rtds_ms = now_ms();
                let _ = ts;
            }
        }
        _ => {}
    }
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
