//! Client canal market CLOB (`wss://ws-subscriptions-clob.polymarket.com/ws/market`).
//!
//! Une connexion par fenêtre active (les tokens changent toutes les 5 min).
//! Pour ne pas avoir de trou au changement de fenêtre, l'orchestrateur ouvre
//! la connexion de la fenêtre N+1 dès sa découverte (les fenêtres up/down
//! sont créées à l'avance côté Polymarket) et ne coupe l'ancienne qu'après
//! la résolution.

use crate::{now_ms, Bus, Recorder, WS_CLOB_MARKET_URL};
use futures_util::{SinkExt, StreamExt};
use pm_core::{parse, BusEvent};
use serde_json::json;
use std::time::Duration;
use tokio::time;
use tokio_tungstenite::{connect_async, tungstenite::Message};

/// Stream les deux tokens d'une fenêtre jusqu'à annulation du task ou
/// fermeture définitive côté serveur (marché résolu).
pub async fn run_for_tokens(bus: Bus, recorder: Recorder, token_up: String, token_down: String) {
    let mut backoff_s = 1u64;
    loop {
        match connect_and_stream(&bus, &recorder, &token_up, &token_down).await {
            Ok(()) => {
                tracing::info!("CLOB flux terminé (marché résolu/fermé)");
                return;
            }
            Err(e) => tracing::warn!("CLOB erreur: {e:#}"),
        }
        time::sleep(Duration::from_secs(backoff_s)).await;
        backoff_s = (backoff_s * 2).min(15);
    }
}

async fn connect_and_stream(
    bus: &Bus,
    recorder: &Recorder,
    token_up: &str,
    token_down: &str,
) -> anyhow::Result<()> {
    let (ws, _) = connect_async(WS_CLOB_MARKET_URL).await?;
    tracing::info!("CLOB connecté (up={}…, down={}…)", &token_up[..8.min(token_up.len())], &token_down[..8.min(token_down.len())]);
    let (mut write, mut read) = ws.split();
    let sub = json!({
        "assets_ids": [token_up, token_down],
        "type": "market",
        "initial_dump": true,
        "custom_feature_enabled": true
    });
    write.send(Message::Text(sub.to_string().into())).await?;

    let mut ping = time::interval(Duration::from_secs(10));
    loop {
        tokio::select! {
            msg = read.next() => {
                let text = match msg {
                    Some(Ok(Message::Text(t))) => t.to_string(),
                    Some(Ok(Message::Binary(b))) => String::from_utf8_lossy(&b).to_string(),
                    Some(Ok(Message::Ping(d))) => { write.send(Message::Pong(d)).await.ok(); continue; }
                    Some(Ok(Message::Close(_))) | None => anyhow::bail!("CLOB fermé par le serveur"),
                    Some(Err(e)) => return Err(e.into()),
                    _ => continue,
                };
                let recv_ms = now_ms();
                if text.trim() != "PONG" && !text.trim().is_empty() {
                    // Archive verbatim avant parsing.
                    recorder.record("clob", text.as_str(), recv_ms);
                }
                let mut resolved = false;
                for ev in parse::parse_clob_frame(&text) {
                    if matches!(ev, pm_core::ClobEvent::MarketResolved { .. }) {
                        resolved = true;
                    }
                    bus.publish(BusEvent::Clob(ev));
                }
                if resolved {
                    // Fin de vie naturelle de cette connexion.
                    return Ok(());
                }
            }
            _ = ping.tick() => { write.send(Message::Text("PING".into())).await.ok(); }
        }
    }
}
