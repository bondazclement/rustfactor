use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::Result;
use serde::Serialize;
use tokio::fs::{create_dir_all, File, OpenOptions};
use tokio::io::{AsyncWriteExt, BufWriter};
use tokio::sync::mpsc;
use tokio::time::{self, Duration};

use crate::models::AppState;

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RecordEvent {
    WindowChanged {
        ts_ms: u64,
        window_slug: String,
        window_epoch: u64,
        token_up: String,
        token_down: String,
    },
    RtdsTick {
        ts_ms: u64,
        source_ts_ms: u64,
        message_ts_ms: u64,
        symbol: String,
        price: f64,
        raw_value: f64,
    },
    ClobEvent {
        ts_ms: u64,
        event_type: String,
        event_ts_ms: u64,
        payload: serde_json::Value,
    },
}

pub async fn start_writer(
    out_dir: PathBuf,
    state: Arc<Mutex<AppState>>,
    mut rx: mpsc::UnboundedReceiver<RecordEvent>,
) -> Result<()> {
    create_dir_all(&out_dir).await?;
    let mut current_epoch: Option<u64> = None;
    let mut writer: Option<BufWriter<File>> = None;
    let mut flush = time::interval(Duration::from_millis(250));
    loop {
        tokio::select! {
            _ = flush.tick() => {
                if let Some(w) = writer.as_mut() { let _ = w.flush().await; }
            }
            maybe = rx.recv() => {
                let Some(ev) = maybe else { break; };
                if let RecordEvent::WindowChanged { window_epoch, .. } = &ev {
                    if current_epoch != Some(*window_epoch) {
                        current_epoch = Some(*window_epoch);
                        writer = Some(open_window_writer(&out_dir, *window_epoch).await?);
                        if let Ok(mut st) = state.lock() {
                            st.recording_file = window_file_path(&out_dir, *window_epoch).display().to_string();
                            st.recording_lines = 0;
                            st.recording_bytes = 0;
                        }
                    }
                }
                if let Some(w) = writer.as_mut() {
                    let line = serde_json::to_vec(&ev)?;
                    w.write_all(&line).await?;
                    w.write_all(b"\n").await?;
                    if let Ok(mut st) = state.lock() {
                        st.recording_lines += 1;
                        st.recording_bytes += (line.len() + 1) as u64;
                        st.last_record_write_ms = now_ms();
                    }
                }
            }
        }
    }
    if let Some(w) = writer.as_mut() {
        let _ = w.flush().await;
    }
    Ok(())
}

async fn open_window_writer(out_dir: &Path, epoch: u64) -> Result<BufWriter<File>> {
    let window_dir = out_dir.join(format!("window_{}", epoch));
    create_dir_all(&window_dir).await?;
    let file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(window_dir.join("raw.ndjson"))
        .await?;
    Ok(BufWriter::new(file))
}

fn window_file_path(out_dir: &Path, epoch: u64) -> PathBuf {
    out_dir.join(format!("window_{}/raw.ndjson", epoch))
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
