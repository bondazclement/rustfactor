use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::Result;
use futures_util::{SinkExt, StreamExt};
use serde_json::json;
use tokio::sync::mpsc;
use tokio::time;
use tokio_tungstenite::{connect_async, tungstenite::Message};

use crate::models::{
    AppState, BestBidAskEvent, BookEvent, LastTradePriceEvent, PriceChangeEvent, Trade,
};
use crate::record::RecordEvent;

const WS_CLOB: &str = "wss://ws-subscriptions-clob.polymarket.com/ws/market";

pub async fn run_clob_ws(
    state: Arc<Mutex<AppState>>,
    token_up: String,
    token_down: String,
    recorder: Option<mpsc::UnboundedSender<RecordEvent>>,
) -> Result<()> {
    loop {
        match connect_and_stream(state.clone(), &token_up, &token_down, recorder.clone()).await {
            Ok(_) => eprintln!("[CLOB] disconnected"),
            Err(e) => eprintln!("[CLOB] error: {}", e),
        }
        time::sleep(Duration::from_secs(2)).await;
    }
}

async fn connect_and_stream(
    state: Arc<Mutex<AppState>>,
    token_up: &str,
    token_down: &str,
    recorder: Option<mpsc::UnboundedSender<RecordEvent>>,
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
    write
        .send(Message::Text(sub_msg.to_string().into()))
        .await?;
    let mut ping = time::interval(Duration::from_secs(10));
    loop {
        tokio::select! {
            msg = read.next() => {
                match msg {
                    Some(Ok(Message::Text(text))) => process_clob_message(&text, &state, recorder.clone()),
                    Some(Ok(Message::Binary(bin))) => {
                        let text = String::from_utf8_lossy(&bin).to_string();
                        process_clob_message(&text, &state, recorder.clone());
                    }
                    Some(Ok(Message::Ping(data))) => { let _ = write.send(Message::Pong(data)).await; }
                    Some(Ok(Message::Close(_))) | None => return Err(anyhow::anyhow!("CLOB closed")),
                    _ => {}
                }
            }
            _ = ping.tick() => { let _ = write.send(Message::Text("PING".into())).await; }
        }
    }
}

fn process_clob_message(
    text: &str,
    state: &Arc<Mutex<AppState>>,
    recorder: Option<mpsc::UnboundedSender<RecordEvent>>,
) {
    let events: Vec<serde_json::Value> = match serde_json::from_str(text) {
        Ok(v) => v,
        Err(_) => match serde_json::from_str::<serde_json::Value>(text) {
            Ok(v) => vec![v],
            Err(_) => return,
        },
    };
    let recv_ms = now_ms();
    if let Ok(mut st) = state.lock() {
        st.last_ws_clob_ms = recv_ms;
    }
    for event_val in events {
        let event_type = event_val
            .get("event_type")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        match event_type {
            "book" => {
                if let Ok(ev) = serde_json::from_value::<BookEvent>(event_val.clone()) {
                    let ts = ev.timestamp.parse::<u64>().unwrap_or(0);
                    if let Ok(mut st) = state.lock() {
                        st.clob_latency_ms = recv_ms as i64 - ts as i64;
                        let token_up = st
                            .market
                            .as_ref()
                            .map(|m| m.token_up.clone())
                            .unwrap_or_default();
                        if ev.asset_id == token_up {
                            st.book_up.apply_snapshot(&ev);
                        } else {
                            st.book_down.apply_snapshot(&ev);
                        }
                        st.clob_event_count_window += 1;
                    }
                    if let Some(tx) = recorder.clone() {
                        let _ = tx.send(RecordEvent::ClobEvent {
                            ts_ms: recv_ms,
                            event_type: "book".to_string(),
                            event_ts_ms: ts,
                            payload: event_val,
                        });
                    }
                }
            }
            "price_change" => {
                if let Ok(ev) = serde_json::from_value::<PriceChangeEvent>(event_val.clone()) {
                    let ts = ev.timestamp.parse::<u64>().unwrap_or(0);
                    if let Ok(mut st) = state.lock() {
                        st.clob_latency_ms = recv_ms as i64 - ts as i64;
                        let token_up = st
                            .market
                            .as_ref()
                            .map(|m| m.token_up.clone())
                            .unwrap_or_default();
                        for delta in &ev.price_changes {
                            if delta.asset_id == token_up {
                                st.book_up.apply_delta(delta);
                            } else {
                                st.book_down.apply_delta(delta);
                            }
                        }
                        st.clob_event_count_window += 1;
                    }
                    if let Some(tx) = recorder.clone() {
                        let _ = tx.send(RecordEvent::ClobEvent {
                            ts_ms: recv_ms,
                            event_type: "price_change".to_string(),
                            event_ts_ms: ts,
                            payload: event_val,
                        });
                    }
                }
            }
            "last_trade_price" => {
                if let Ok(ev) = serde_json::from_value::<LastTradePriceEvent>(event_val.clone()) {
                    let ts = ev.timestamp.parse::<u64>().unwrap_or(0);
                    if let (Ok(price), Ok(size)) = (ev.price.parse::<f64>(), ev.size.parse::<f64>()) {
                        if let Ok(mut st) = state.lock() {
                            st.clob_latency_ms = recv_ms as i64 - ts as i64;
                            let trade = Trade {
                                price,
                                size,
                                side: ev.side,
                                timestamp_ms: ts,
                            };
                            if st.recent_trades.len() >= 100 {
                                st.recent_trades.pop_front();
                            }
                            st.recent_trades.push_back(trade);
                            st.clob_event_count_window += 1;
                        }
                    }
                    if let Some(tx) = recorder.clone() {
                        let _ = tx.send(RecordEvent::ClobEvent {
                            ts_ms: recv_ms,
                            event_type: "last_trade_price".to_string(),
                            event_ts_ms: ts,
                            payload: event_val,
                        });
                    }
                }
            }
            "best_bid_ask" => {
                if let Ok(ev) = serde_json::from_value::<BestBidAskEvent>(event_val.clone()) {
                    let ts = ev.timestamp.parse::<u64>().unwrap_or(0);
                    if let Ok(mut st) = state.lock() {
                        st.clob_latency_ms = recv_ms as i64 - ts as i64;
                        let token_up = st
                            .market
                            .as_ref()
                            .map(|m| m.token_up.clone())
                            .unwrap_or_default();
                        if ev.asset_id == token_up {
                            st.book_up.last_update_ms = ts;
                        } else {
                            st.book_down.last_update_ms = ts;
                        }
                        st.clob_event_count_window += 1;
                    }
                    if let Some(tx) = recorder.clone() {
                        let _ = tx.send(RecordEvent::ClobEvent {
                            ts_ms: recv_ms,
                            event_type: "best_bid_ask".to_string(),
                            event_ts_ms: ts,
                            payload: event_val,
                        });
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
