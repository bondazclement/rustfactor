//! Connexion WebSocket avec prise en charge du proxy HTTP CONNECT.
//!
//! Certains environnements (dont Claude Code Remote) n'autorisent la sortie
//! que via un proxy déclaré dans `HTTPS_PROXY`. `tokio-tungstenite` ne lit
//! pas cette variable : ce module établit le tunnel CONNECT à la main puis
//! laisse `client_async_tls` faire TLS + upgrade WebSocket par-dessus.
//! Sans variable de proxy, connexion TCP directe classique.

use anyhow::{bail, Context, Result};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio_tungstenite::{client_async_tls, MaybeTlsStream, WebSocketStream};

pub type WsStream = WebSocketStream<MaybeTlsStream<TcpStream>>;

fn proxy_from_env() -> Option<String> {
    for var in ["HTTPS_PROXY", "https_proxy", "ALL_PROXY", "all_proxy"] {
        if let Ok(v) = std::env::var(var) {
            if !v.trim().is_empty() {
                return Some(v);
            }
        }
    }
    None
}

fn host_port(url: &str, default_port: u16) -> Result<(String, u16)> {
    let stripped = url
        .strip_prefix("wss://")
        .or_else(|| url.strip_prefix("https://"))
        .or_else(|| url.strip_prefix("http://"))
        .or_else(|| url.strip_prefix("ws://"))
        .unwrap_or(url);
    let authority = stripped.split(['/', '?']).next().unwrap_or(stripped);
    match authority.rsplit_once(':') {
        Some((h, p)) => Ok((h.to_string(), p.parse().context("port proxy invalide")?)),
        None => Ok((authority.to_string(), default_port)),
    }
}

/// Ouvre un WebSocket TLS vers `url`, via le proxy CONNECT si configuré.
pub async fn connect_ws(url: &str) -> Result<WsStream> {
    let (target_host, target_port) = host_port(url, 443)?;
    let tcp = match proxy_from_env() {
        Some(proxy_url) => {
            let (proxy_host, proxy_port) = host_port(&proxy_url, 80)?;
            let mut s = TcpStream::connect((proxy_host.as_str(), proxy_port))
                .await
                .with_context(|| format!("connexion au proxy {proxy_host}:{proxy_port}"))?;
            s.set_nodelay(true).ok();
            let req = format!(
                "CONNECT {target_host}:{target_port} HTTP/1.1\r\nHost: {target_host}:{target_port}\r\nProxy-Connection: keep-alive\r\n\r\n"
            );
            s.write_all(req.as_bytes()).await?;
            // Lit la réponse jusqu'à la fin des en-têtes.
            let mut buf = Vec::with_capacity(256);
            let mut byte = [0u8; 1];
            while !buf.ends_with(b"\r\n\r\n") {
                let n = s.read(&mut byte).await?;
                if n == 0 {
                    bail!("proxy: connexion fermée pendant CONNECT");
                }
                buf.push(byte[0]);
                if buf.len() > 16_384 {
                    bail!("proxy: réponse CONNECT trop longue");
                }
            }
            let head = String::from_utf8_lossy(&buf);
            let status = head.lines().next().unwrap_or_default();
            if !status.contains(" 200") {
                bail!("proxy CONNECT refusé: {status}");
            }
            s
        }
        None => {
            let s = TcpStream::connect((target_host.as_str(), target_port))
                .await
                .with_context(|| format!("connexion directe {target_host}:{target_port}"))?;
            s.set_nodelay(true).ok();
            s
        }
    };
    let (ws, _) = client_async_tls(url, tcp)
        .await
        .context("TLS/upgrade WebSocket")?;
    Ok(ws)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_port_parsing() {
        assert_eq!(
            host_port("wss://ws-live-data.polymarket.com", 443).unwrap(),
            ("ws-live-data.polymarket.com".into(), 443)
        );
        assert_eq!(
            host_port("wss://example.com/ws/market", 443).unwrap(),
            ("example.com".into(), 443)
        );
        assert_eq!(
            host_port("http://127.0.0.1:34721", 80).unwrap(),
            ("127.0.0.1".into(), 34721)
        );
        assert_eq!(
            host_port("http://proxy.local", 80).unwrap(),
            ("proxy.local".into(), 80)
        );
    }
}
