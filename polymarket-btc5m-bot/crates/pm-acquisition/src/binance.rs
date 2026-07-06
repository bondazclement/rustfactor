//! Capture DIRECTE du flux Binance (btcusdt@trade) — enregistrement pur.
//!
//! Constat (étude 5 du 06/07) : le relais spot de Polymarket (RTDS
//! `crypto_prices`) est EN RETARD de ~5 s sur l'oracle Chainlink — il ne
//! peut donc pas servir de source en avance. La seule vraie source
//! d'information anticipée est la bourse elle-même : ce module s'y
//! connecte en direct et archive verbatim (stream "binance") pour l'étude
//! lead-lag. AUCUNE décision de trading n'est branchée dessus tant que
//! l'avance n'est pas mesurée et validée.

use crate::{now_ms, Recorder};
use futures_util::{SinkExt, StreamExt};
use std::time::Duration;
use tokio::time;
use tokio_tungstenite::tungstenite::Message;

const WS_BINANCE_URL: &str = "wss://stream.binance.com:9443/ws/btcusdt@trade";

pub async fn run(recorder: Recorder) {
    let mut backoff_s = 1u64;
    loop {
        match connect_and_stream(&recorder).await {
            Ok(()) => return,
            Err(e) => tracing::warn!("binance: {e:#} — reconnexion dans {backoff_s}s"),
        }
        time::sleep(Duration::from_secs(backoff_s)).await;
        backoff_s = (backoff_s * 2).min(30);
    }
}

async fn connect_and_stream(recorder: &Recorder) -> anyhow::Result<()> {
    let ws = crate::net::connect_ws(WS_BINANCE_URL).await?;
    tracing::info!("Binance connecté (btcusdt@trade, capture seule)");
    let (mut write, mut read) = ws.split();
    let mut check = time::interval(Duration::from_secs(5));
    let mut last_data_ms = now_ms();
    loop {
        tokio::select! {
            msg = read.next() => {
                match msg {
                    Some(Ok(Message::Text(t))) => {
                        let recv_ms = now_ms();
                        last_data_ms = recv_ms;
                        recorder.record("binance", t.as_str(), recv_ms);
                    }
                    Some(Ok(Message::Ping(d))) => { write.send(Message::Pong(d)).await.ok(); }
                    Some(Ok(Message::Close(_))) | None => anyhow::bail!("fermé par le serveur"),
                    Some(Err(e)) => return Err(e.into()),
                    _ => {}
                }
            }
            _ = check.tick() => {
                // btcusdt@trade émet en continu : 15 s de silence = mort.
                let silent = now_ms().saturating_sub(last_data_ms);
                if silent >= 15_000 {
                    anyhow::bail!("silencieux {silent} ms");
                }
            }
        }
    }
}
