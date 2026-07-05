//! Journal NDJSON v2 : archive brute, garantie de fidélité du projet.
//!
//! Chaque trame réseau est écrite **verbatim** avec ses horodatages de
//! réception, *avant* tout parsing. Format (une ligne JSON par trame) :
//!
//! ```jsonc
//! {"v":2,"recv_ms":1778341500123,"mono_ns":123456789,"stream":"rtds","raw":"{...trame...}"}
//! ```
//!
//! - `recv_ms`  : horloge murale locale (ms) à la réception,
//! - `mono_ns`  : horloge monotone (ns) depuis le démarrage du process —
//!   insensible aux sauts NTP, sert aux mesures de latence fines,
//! - `stream`   : `rtds` | `clob` | `gamma` | `meta`,
//! - `raw`      : la trame texte exacte reçue (ou document JSON pour gamma/meta).
//!
//! Les fichiers sont segmentés par heure UTC (`journal_YYYYMMDDTHH.ndjson`)
//! dans `out_dir`, en append : un redémarrage ne détruit jamais rien. Le
//! découpage par fenêtre de 5 min est fait au post-traitement (pm-replay),
//! jamais à l'acquisition — c'est ce qui garantit de ne pas perdre les ticks
//! qui encadrent la frontière T0.

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::Instant;
use tokio::fs::{create_dir_all, File, OpenOptions};
use tokio::io::{AsyncWriteExt, BufWriter};
use tokio::sync::mpsc;
use tokio::time::{self, Duration};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RawFrame {
    pub v: u8,
    pub recv_ms: u64,
    pub mono_ns: u64,
    pub stream: String,
    pub raw: String,
}

impl RawFrame {
    pub fn new(stream: &str, raw: impl Into<String>, recv_ms: u64, mono_ns: u64) -> Self {
        Self {
            v: 2,
            recv_ms,
            mono_ns,
            stream: stream.to_string(),
            raw: raw.into(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Recorder {
    tx: mpsc::UnboundedSender<RawFrame>,
    epoch: Instant,
}

#[derive(Debug, Default, Clone, Copy)]
pub struct RecorderStats {
    pub lines: u64,
    pub bytes: u64,
}

impl Recorder {
    /// Démarre la tâche d'écriture ; renvoie le handle d'envoi.
    /// L'écriture est hors du chemin chaud : canal non borné + flush 250 ms.
    pub fn spawn(out_dir: PathBuf) -> Self {
        let (tx, rx) = mpsc::unbounded_channel::<RawFrame>();
        tokio::spawn(async move {
            if let Err(e) = writer_loop(out_dir, rx).await {
                tracing::error!("recorder writer error: {e:#}");
            }
        });
        Self {
            tx,
            epoch: Instant::now(),
        }
    }

    pub fn mono_ns(&self) -> u64 {
        self.epoch.elapsed().as_nanos() as u64
    }

    /// Journalise une trame brute. Ne bloque jamais.
    pub fn record(&self, stream: &str, raw: impl Into<String>, recv_ms: u64) {
        let frame = RawFrame::new(stream, raw, recv_ms, self.mono_ns());
        if self.tx.send(frame).is_err() {
            tracing::error!("recorder channel closed — trame perdue");
        }
    }
}

fn segment_name(recv_ms: u64) -> String {
    // Segmentation par heure UTC sans dépendance chrono :
    // jours civils depuis l'époque + heure.
    let secs = recv_ms / 1000;
    let days = secs / 86_400;
    let (y, m, d) = civil_from_days(days as i64);
    let hh = (secs % 86_400) / 3600;
    format!("journal_{y:04}{m:02}{d:02}T{hh:02}.ndjson")
}

/// Algorithme de Howard Hinnant (domaine public) : jours → date civile.
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

async fn open_segment(dir: &Path, name: &str) -> Result<BufWriter<File>> {
    create_dir_all(dir).await?;
    let file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(dir.join(name))
        .await?;
    Ok(BufWriter::new(file))
}

async fn writer_loop(out_dir: PathBuf, mut rx: mpsc::UnboundedReceiver<RawFrame>) -> Result<()> {
    let mut current_name = String::new();
    let mut writer: Option<BufWriter<File>> = None;
    let mut flush = time::interval(Duration::from_millis(250));
    flush.set_missed_tick_behavior(time::MissedTickBehavior::Delay);
    loop {
        tokio::select! {
            _ = flush.tick() => {
                if let Some(w) = writer.as_mut() { let _ = w.flush().await; }
            }
            maybe = rx.recv() => {
                let Some(frame) = maybe else { break };
                let name = segment_name(frame.recv_ms);
                if name != current_name {
                    if let Some(w) = writer.as_mut() { let _ = w.flush().await; }
                    writer = Some(open_segment(&out_dir, &name).await?);
                    current_name = name;
                }
                if let Some(w) = writer.as_mut() {
                    let line = serde_json::to_vec(&frame)?;
                    w.write_all(&line).await?;
                    w.write_all(b"\n").await?;
                }
            }
        }
    }
    if let Some(w) = writer.as_mut() {
        let _ = w.flush().await;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn segment_name_is_utc_hour() {
        // 2026-05-08 12:25:00 UTC = 1778243100 (multiple de 300)
        assert_eq!(
            segment_name(1_778_243_100_000),
            "journal_20260508T12.ndjson"
        );
        // Frontière d'heure.
        assert_eq!(
            segment_name(1_778_245_199_999),
            "journal_20260508T12.ndjson"
        );
        assert_eq!(
            segment_name(1_778_245_200_000),
            "journal_20260508T13.ndjson"
        );
    }

    #[test]
    fn civil_epoch() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(19_723), (2024, 1, 1));
    }

    #[tokio::test]
    async fn frames_are_written_verbatim() {
        let dir = std::env::temp_dir().join(format!("pm_rec_test_{}", std::process::id()));
        let _ = tokio::fs::remove_dir_all(&dir).await;
        let rec = Recorder::spawn(dir.clone());
        let raw = r#"{"topic":"crypto_prices_chainlink","type":"update","payload":{"symbol":"btc/usd","timestamp":1778341500000,"value":80466.61}}"#;
        rec.record("rtds", raw, 1_778_341_500_123);
        // Laisse le writer flusher.
        tokio::time::sleep(Duration::from_millis(400)).await;
        let seg = dir.join(segment_name(1_778_341_500_123));
        let content = tokio::fs::read_to_string(&seg).await.unwrap();
        let parsed: RawFrame = serde_json::from_str(content.lines().next().unwrap()).unwrap();
        assert_eq!(parsed.raw, raw, "la trame doit être conservée verbatim");
        assert_eq!(parsed.stream, "rtds");
        assert_eq!(parsed.v, 2);
        let _ = tokio::fs::remove_dir_all(&dir).await;
    }
}
