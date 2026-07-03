use anyhow::Result;
use futures_util::{SinkExt, StreamExt};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::time;
use tokio_tungstenite::{connect_async, tungstenite::Message};

use crate::models::{AppState, ChainlinkTick};

const WS_RTDS: &str = "wss://ws-live-data.polymarket.com";

// Durée max sans tick avant re-subscribe (comme en Python: 5.5s)
const RECV_TIMEOUT_SECS: f64 = 5.5;
// Intervalle PING manuel en texte (comme en Python: 5s)
const PING_INTERVAL_SECS: u64 = 5;

/// Décode un prix brut Chainlink : si > 1e15, diviser par 1e18 (format wei)
fn decode_price(value: f64) -> f64 {
    if value > 1e15 {
        value / 1e18
    } else {
        value
    }
}

pub async fn run_rtds_ws(state: Arc<Mutex<AppState>>) {
    let mut backoff = 1.0f64;
    loop {
        match connect_and_stream(state.clone()).await {
            Ok(_) => {
                eprintln!("[RTDS] déconnecté, reconnexion dans {:.1}s...", backoff);
            }
            Err(e) => {
                eprintln!("[RTDS] erreur: {}, reconnexion dans {:.1}s...", e, backoff);
            }
        }
        if backoff < 15.0 {
            time::sleep(Duration::from_secs_f64(backoff + rand_jitter())).await;
            backoff = (backoff * 2.0).min(15.0);
        } else {
            time::sleep(Duration::from_secs(15)).await;
        }
    }
}

fn rand_jitter() -> f64 {
    // Jitter 0..1 sans dépendance rand
    let t = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .subsec_nanos() as f64;
    (t % 1_000_000_000.0) / 1_000_000_000.0
}

async fn connect_and_stream(state: Arc<Mutex<AppState>>) -> Result<()> {
    let (ws_stream, _) = connect_async(WS_RTDS).await?;
    let (mut write, mut read) = ws_stream.split();

    // Filtre vide = recevoir tous les symboles Chainlink, on filtre côté client sur btc/usd.
    // Le filtre serveur par symbole est inconsistant (parfois silencieux).
    let sub_cl = r#"{"action":"subscribe","subscriptions":[{"topic":"crypto_prices_chainlink","type":"*","filters":""}]}"#;
    let sub_bin = r#"{"action":"subscribe","subscriptions":[{"topic":"crypto_prices","type":"update","filters":"btcusdt"}]}"#;

    write.send(Message::Text(sub_cl.into())).await?;
    write.send(Message::Text(sub_bin.into())).await?;

    // Intervalle PING (5s, texte applicatif, comme Python)
    let mut ping_interval = time::interval(Duration::from_secs(PING_INTERVAL_SECS));
    ping_interval.set_missed_tick_behavior(time::MissedTickBehavior::Skip);
    ping_interval.tick().await; // consommer le tick initial immédiat

    // Intervalle re-subscribe si silence prolongé (5.5s, comme Python)
    let mut refresh_interval = time::interval(Duration::from_secs_f64(RECV_TIMEOUT_SECS));
    refresh_interval.set_missed_tick_behavior(time::MissedTickBehavior::Skip);
    refresh_interval.tick().await;
    let mut last_data_ms = now_ms();

    loop {
        tokio::select! {
            biased; // priorité aux messages reçus

            msg = read.next() => {
                match msg {
                    Some(Ok(Message::Text(text))) => {
                        if text.is_empty() || text.trim().is_empty() {
                            continue;
                        }
                        if text.trim() == "PONG" {
                            continue;
                        }
                        last_data_ms = now_ms();
                        let recv_ms = last_data_ms;
                        process_rtds_message(&text, &state, recv_ms);
                    }
                    Some(Ok(Message::Binary(bytes))) => {
                        let text = String::from_utf8_lossy(&bytes).to_string();
                        if text.trim() == "PONG" || text.is_empty() { continue; }
                        last_data_ms = now_ms();
                        process_rtds_message(&text, &state, last_data_ms);
                    }
                    Some(Ok(Message::Close(_))) | None => {
                        return Err(anyhow::anyhow!("connexion fermée"));
                    }
                    Some(Ok(Message::Ping(data))) => {
                        write.send(Message::Pong(data)).await.ok();
                    }
                    _ => {}
                }
            }
            _ = ping_interval.tick() => {
                // PING texte applicatif (comme Python)
                write.send(Message::Text("PING".into())).await.ok();
            }
            _ = refresh_interval.tick() => {
                // Si silence > 5.5s → re-subscribe (comme Python)
                if now_ms().saturating_sub(last_data_ms) >= (RECV_TIMEOUT_SECS * 1000.0) as u64 {
                    write.send(Message::Text(sub_cl.into())).await.ok();
                    write.send(Message::Text(sub_bin.into())).await.ok();
                }
            }
        }
    }
}

fn process_rtds_message(text: &str, state: &Arc<Mutex<AppState>>, recv_ms: u64) {
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
            if symbol.to_lowercase() != "btc/usd" {
                return;
            }
            let ts_cl = payload.get("timestamp").and_then(|v| v.as_u64()).unwrap_or(0);
            let raw_val = payload.get("value").and_then(|v| v.as_f64()).unwrap_or(0.0);
            let price = decode_price(raw_val);
            if price <= 0.0 { return; }

            let mut st = state.lock().unwrap();
            st.btc_price_chainlink = Some(price);
            st.chainlink_timestamp_ms = ts_cl;
            st.last_ws_rtds_ms = recv_ms;
            st.rtds_latency_ms = recv_ms as i64 - ts_rtds as i64;

            // Ajouter le tick au buffer Chainlink pour la bougie
            let tick = ChainlinkTick { ts_ms: ts_cl, price };
            st.chainlink_ticks.push_back(tick);
            // Garder max 512 ticks (~8 min à 1 tick/s)
            while st.chainlink_ticks.len() > 512 {
                st.chainlink_ticks.pop_front();
            }

            // Mettre à jour la bougie Chainlink pour la fenêtre courante
            if let Some(ref m) = st.market.clone() {
                let window_start_ms = m.end_date_ms.saturating_sub(300_000); // 5 min avant fin
                st.update_chainlink_candle(window_start_ms, m.end_date_ms);
            }
        }
        "crypto_prices" if msg_type == "update" => {
            let symbol = payload.get("symbol").and_then(|v| v.as_str()).unwrap_or("");
            if symbol.to_lowercase() != "btcusdt" {
                return;
            }
            let ts_bin = payload.get("timestamp").and_then(|v| v.as_u64()).unwrap_or(0);
            let raw_val = payload.get("value").and_then(|v| v.as_f64()).unwrap_or(0.0);
            let price = decode_price(raw_val);
            if price <= 0.0 { return; }

            let mut st = state.lock().unwrap();
            st.btc_price_binance = Some(price);
            st.binance_timestamp_ms = ts_bin;
            st.last_ws_rtds_ms = recv_ms;
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
