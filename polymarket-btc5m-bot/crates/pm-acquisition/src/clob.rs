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
use tokio_tungstenite::tungstenite::Message;

/// Stream les deux tokens d'une fenêtre jusqu'à `market_resolved` ou
/// jusqu'à `deadline_ms` (garde-fou si la résolution n'est jamais émise).
/// L'orchestrateur laisse cette tâche vivre au-delà de la rotation de
/// fenêtre : c'est ce chevauchement qui garantit la capture de la
/// résolution officielle (vérité terrain Up/Down).
pub async fn run_for_tokens(
    bus: Bus,
    recorder: Recorder,
    token_up: String,
    token_down: String,
    deadline_ms: u64,
) {
    let mut backoff_s = 1u64;
    loop {
        match connect_and_stream(&bus, &recorder, &token_up, &token_down).await {
            Ok(()) => {
                tracing::info!("CLOB flux terminé (marché résolu/fermé)");
                return;
            }
            Err(e) => tracing::warn!("CLOB erreur: {e:#}"),
        }
        if now_ms() >= deadline_ms {
            tracing::warn!("CLOB deadline atteinte sans market_resolved");
            return;
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
    let ws = crate::net::connect_ws(WS_CLOB_MARKET_URL).await?;
    tracing::info!(
        "CLOB connecté (up={}…, down={}…)",
        &token_up[..8.min(token_up.len())],
        &token_down[..8.min(token_down.len())]
    );
    let (mut write, mut read) = ws.split();
    let sub = json!({
        "assets_ids": [token_up, token_down],
        "type": "market",
        "initial_dump": true,
        "custom_feature_enabled": true
    });
    write.send(Message::Text(sub.to_string())).await?;

    let mut ping = time::interval(Duration::from_secs(10));
    let mut check = time::interval(Duration::from_secs(1));
    let mut last_data_ms = now_ms();
    loop {
        tokio::select! {
            _ = check.tick() => {
                // Connexion à moitié morte (cf. incident RTDS 2026-07-04) :
                // un carnet actif émet en continu ; 30 s de silence = mort.
                let silent = now_ms().saturating_sub(last_data_ms);
                if silent >= 30_000 {
                    anyhow::bail!("CLOB silencieux {silent} ms → reconnexion forcée");
                }
            }
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
                    // Archive verbatim avant parsing. Seules les vraies trames
                    // comptent pour le détecteur de silence : un PONG ne prouve
                    // pas que le carnet vit (cf. incident RTDS du 06/07).
                    last_data_ms = recv_ms;
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
