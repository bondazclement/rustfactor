use anyhow::Result;
use futures_util::{SinkExt, StreamExt};
use serde_json::json;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::time;
use tokio_tungstenite::{connect_async, tungstenite::Message};

use crate::models::{AppState, BookEvent, BestBidAskEvent, LastTradePriceEvent, PriceChangeEvent, Trade};

const WS_CLOB: &str = "wss://ws-subscriptions-clob.polymarket.com/ws/market";
const PING_INTERVAL_SECS: u64 = 10;

pub async fn run_clob_ws(
    state: Arc<Mutex<AppState>>,
    token_up: String,
    token_down: String,
) -> Result<()> {
    loop {
        match connect_and_stream(state.clone(), &token_up, &token_down).await {
            Ok(_) => {
                eprintln!("[CLOB WS] déconnecté, reconnexion dans 2s...");
            }
            Err(e) => {
                eprintln!("[CLOB WS] erreur: {}, reconnexion dans 2s...", e);
            }
        }
        time::sleep(Duration::from_secs(2)).await;
    }
}

async fn connect_and_stream(
    state: Arc<Mutex<AppState>>,
    token_up: &str,
    token_down: &str,
) -> Result<()> {
    let (ws_stream, _) = connect_async(WS_CLOB).await?;
    let (mut write, mut read) = ws_stream.split();

    let sub_msg = json!({
        "assets_ids": [token_up, token_down],
        "type": "market",
        "initial_dump": true,
        "level": 2,
        "custom_feature_enabled": true
    });
    write.send(Message::Text(sub_msg.to_string().into())).await?;

    let mut ping_interval = time::interval(Duration::from_secs(PING_INTERVAL_SECS));
    ping_interval.set_missed_tick_behavior(time::MissedTickBehavior::Skip);
    ping_interval.tick().await;

    loop {
        tokio::select! {
            msg = read.next() => {
                match msg {
                    Some(Ok(Message::Text(text))) => {
                        let recv_ms = now_ms();
                        process_clob_message(&text, &state, recv_ms);
                    }
                    Some(Ok(Message::Close(_))) | None => break,
                    Some(Ok(Message::Ping(data))) => {
                        write.send(Message::Pong(data)).await.ok();
                    }
                    _ => {}
                }
            }
            _ = ping_interval.tick() => {
                write.send(Message::Text("PING".into())).await.ok();
            }
        }
    }
    Ok(())
}

fn process_clob_message(text: &str, state: &Arc<Mutex<AppState>>, recv_ms: u64) {
    let events: Vec<serde_json::Value> = match serde_json::from_str(text) {
        Ok(v) => v,
        Err(_) => match serde_json::from_str::<serde_json::Value>(text) {
            Ok(v) => vec![v],
            Err(_) => return,
        },
    };

    let mut st = state.lock().unwrap();
    st.last_ws_clob_ms = recv_ms;

    for event_val in events {
        let event_type = event_val.get("event_type").and_then(|v| v.as_str()).unwrap_or("");

        match event_type {
            "book" => {
                if let Ok(ev) = serde_json::from_value::<BookEvent>(event_val) {
                    let ts: u64 = ev.timestamp.parse().unwrap_or(0);
                    st.clob_latency_ms = recv_ms as i64 - ts as i64;
                    let token_up = st.market.as_ref().map(|m| m.token_up.clone()).unwrap_or_default();
                    if ev.asset_id == token_up {
                        st.book_up.apply_snapshot(&ev);
                    } else {
                        st.book_down.apply_snapshot(&ev);
                    }
                }
            }
            "price_change" => {
                if let Ok(ev) = serde_json::from_value::<PriceChangeEvent>(event_val) {
                    let ts: u64 = ev.timestamp.parse().unwrap_or(0);
                    st.clob_latency_ms = recv_ms as i64 - ts as i64;
                    let token_up = st.market.as_ref().map(|m| m.token_up.clone()).unwrap_or_default();
                    for delta in &ev.price_changes {
                        if delta.asset_id == token_up {
                            st.book_up.apply_delta(delta);
                        } else {
                            st.book_down.apply_delta(delta);
                        }
                    }
                }
            }
            "last_trade_price" => {
                if let Ok(ev) = serde_json::from_value::<LastTradePriceEvent>(event_val) {
                    let ts: u64 = ev.timestamp.parse().unwrap_or(0);
                    st.clob_latency_ms = recv_ms as i64 - ts as i64;
                    if let (Ok(price), Ok(size)) = (ev.price.parse::<f64>(), ev.size.parse::<f64>()) {
                        let trade = Trade {
                            price,
                            size,
                            side: ev.side.clone(),
                            timestamp_ms: ts,
                        };
                        if st.recent_trades.len() >= 50 {
                            st.recent_trades.pop_front();
                        }
                        st.recent_trades.push_back(trade);
                    }
                }
            }
            "best_bid_ask" => {
                if let Ok(ev) = serde_json::from_value::<BestBidAskEvent>(event_val) {
                    let ts: u64 = ev.timestamp.parse().unwrap_or(0);
                    st.clob_latency_ms = recv_ms as i64 - ts as i64;
                    let token_up = st.market.as_ref().map(|m| m.token_up.clone()).unwrap_or_default();
                    if ev.asset_id == token_up {
                        st.book_up.last_update_ms = ts;
                    } else {
                        st.book_down.last_update_ms = ts;
                    }
                }
            }
            _ => {}
        }
    }
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
