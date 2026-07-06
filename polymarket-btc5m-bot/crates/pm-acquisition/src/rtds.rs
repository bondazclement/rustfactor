//! Client RTDS (`wss://ws-live-data.polymarket.com`).
//!
//! Abonnements :
//! - `crypto_prices_chainlink` filtre `btc/usd` → flux de RÉSOLUTION,
//! - `crypto_prices` filtre `btcusdt`          → indicateur rapide (archivé).
//!
//! Connexion continue (indépendante des fenêtres), PING applicatif 5 s,
//! réabonnement forcé si silence > 5,5 s (comportement validé par le
//! collecteur legacy), reconnexion avec backoff exponentiel borné.

use crate::{now_ms, Bus, Recorder, WS_RTDS_URL};
use futures_util::{SinkExt, StreamExt};
use pm_core::{parse, BusEvent};
use std::time::Duration;
use tokio::time;
use tokio_tungstenite::tungstenite::Message;

// Test live 2026-07-04 : le filtre JSON documenté ({"symbol":"btc/usd"}) ne
// renvoie AUCUNE donnée ; l'abonnement sans filtre (format du legacy) émet
// bien ~1 tick/s/symbole. On s'abonne donc sans filtre et on filtre côté
// client (pm_core::parse ne garde que btc/usd).
const SUB_CHAINLINK: &str = r#"{"action":"subscribe","subscriptions":[{"topic":"crypto_prices_chainlink","type":"*","filters":""}]}"#;
// Sans filtre : le filtre documenté ("btcusdt") ne renvoie AUCUNE donnée
// (constat terrain). Le flux complet est archivé verbatim — il sert à
// mesurer le lead-lag spot/oracle (docs/ETUDE_MODELE.md §5.3).
const SUB_FAST: &str = r#"{"action":"subscribe","subscriptions":[{"topic":"crypto_prices","type":"update","filters":""}]}"#;
const SILENT_RESUB_MS: u64 = 5_500;
/// Au-delà de ce silence, la connexion est considérée morte (tunnel à
/// moitié fermé : les écritures passent, rien n'arrive) → reconnexion
/// complète. Découvert au run du 2026-07-04 22:20 : 40 min de réabonnements
/// inutiles sur une connexion morte.
const SILENT_RECONNECT_MS: u64 = 12_000;

pub async fn run(bus: Bus, recorder: Recorder) {
    let mut backoff_s = 1u64;
    loop {
        match connect_and_stream(&bus, &recorder).await {
            Ok(()) => tracing::warn!("RTDS déconnecté proprement"),
            Err(e) => tracing::warn!("RTDS erreur: {e:#}"),
        }
        time::sleep(Duration::from_secs(backoff_s)).await;
        backoff_s = (backoff_s * 2).min(15);
    }
}

async fn connect_and_stream(bus: &Bus, recorder: &Recorder) -> anyhow::Result<()> {
    let ws = crate::net::connect_ws(WS_RTDS_URL).await?;
    tracing::info!("RTDS connecté");
    let (mut write, mut read) = ws.split();
    write.send(Message::Text(SUB_CHAINLINK.into())).await?;
    write.send(Message::Text(SUB_FAST.into())).await?;

    let mut ping = time::interval(Duration::from_secs(5));
    let mut check = time::interval(Duration::from_millis(1_000));
    let mut last_data_ms = now_ms();
    let mut resub_sent = false;

    loop {
        tokio::select! {
            msg = read.next() => {
                let text = match msg {
                    Some(Ok(Message::Text(t))) => t.to_string(),
                    Some(Ok(Message::Binary(b))) => String::from_utf8_lossy(&b).to_string(),
                    Some(Ok(Message::Ping(d))) => { write.send(Message::Pong(d)).await.ok(); continue; }
                    Some(Ok(Message::Close(_))) | None => anyhow::bail!("RTDS fermé par le serveur"),
                    Some(Err(e)) => return Err(e.into()),
                    _ => continue,
                };
                let recv_ms = now_ms();
                // 1. Archive verbatim AVANT parsing (règle de fidélité).
                if text.trim() != "PONG" && !text.trim().is_empty() {
                    recorder.record("rtds", text.as_str(), recv_ms);
                }
                // 2. Parsing et diffusion. Le détecteur de silence ne compte
                // QUE les ticks de résolution : les PONG et le flux spot
                // maintiennent la connexion « vivante » alors que le canal
                // chainlink peut être mort (incident du 06/07 17h : 45 min
                // de strike figé, reconnexion jamais déclenchée).
                match parse::parse_rtds_frame(&text, recv_ms) {
                    parse::RtdsParsed::Resolution(t) => {
                        last_data_ms = recv_ms;
                        resub_sent = false;
                        bus.publish(BusEvent::Resolution(t));
                    }
                    parse::RtdsParsed::Fast(t) => bus.publish(BusEvent::Fast(t)),
                    parse::RtdsParsed::Ignored => {}
                }
            }
            _ = ping.tick() => { write.send(Message::Text("PING".into())).await.ok(); }
            _ = check.tick() => {
                let silent = now_ms().saturating_sub(last_data_ms);
                if silent >= SILENT_RECONNECT_MS {
                    anyhow::bail!("RTDS sans tick de résolution depuis {silent} ms malgré réabonnement → reconnexion forcée");
                }
                if silent >= SILENT_RESUB_MS && !resub_sent {
                    tracing::warn!("RTDS silencieux {silent} ms → réabonnement");
                    write.send(Message::Text(SUB_CHAINLINK.into())).await.ok();
                    write.send(Message::Text(SUB_FAST.into())).await.ok();
                    resub_sent = true;
                }
            }
        }
    }
}
