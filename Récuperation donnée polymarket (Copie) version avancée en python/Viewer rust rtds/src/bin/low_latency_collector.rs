use std::collections::{BTreeMap, VecDeque};
use std::env;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::fs::{create_dir_all, File, OpenOptions};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, BufWriter};
use tokio::sync::{mpsc, watch};
use tokio::time::{self, Instant};
use tokio_tungstenite::{connect_async, tungstenite::Message};

const GAMMA_BASE: &str = "https://gamma-api.polymarket.com";
const RTDS_WS: &str = "wss://ws-live-data.polymarket.com";
const CLOB_WS: &str = "wss://ws-subscriptions-clob.polymarket.com/ws/market";
const CHAINLINK_SYMBOL: &str = "btc/usd";

#[derive(Clone, Debug, Serialize, Deserialize)]
struct ActiveWindow {
    window_epoch: u64,
    slug: String,
    market_id: String,
    token_up: String,
    token_down: String,
    end_date_ms: u64,
}

#[derive(Debug, Deserialize)]
struct GammaEvent {
    slug: String,
    closed: bool,
    markets: Vec<GammaMarket>,
}

#[derive(Debug, Deserialize)]
struct GammaMarket {
    #[serde(rename = "conditionId")]
    condition_id: String,
    #[serde(rename = "clobTokenIds")]
    clob_token_ids_raw: String,
    outcomes: String,
    #[serde(rename = "endDate")]
    end_date: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct RtdsTick {
    source_ts_ms: u64,
    recv_ts_ms: u64,
    message_ts_ms: u64,
    price: f64,
    raw_value: f64,
    symbol: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct ClobEventRecord {
    event_type: String,
    recv_ts_ms: u64,
    event_ts_ms: u64,
    asset_id: Option<String>,
    payload: Value,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum RawEvent {
    WindowChanged {
        recv_ts_ms: u64,
        window: ActiveWindow,
    },
    RtdsTick {
        recv_ts_ms: u64,
        tick: RtdsTick,
    },
    ClobEvent {
        recv_ts_ms: u64,
        event: ClobEventRecord,
    },
}

#[derive(Clone, Debug)]
enum EngineEvent {
    WindowChanged(ActiveWindow),
    RtdsTick(RtdsTick),
    ClobEvent(ClobEventRecord),
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct FeatureSnapshot {
    ts_ms: u64,
    window_epoch: u64,
    window_slug: String,
    chainlink_open: Option<f64>,
    chainlink_high: Option<f64>,
    chainlink_low: Option<f64>,
    chainlink_close: Option<f64>,
    chainlink_tick_count: usize,
    chainlink_range_abs: Option<f64>,
    chainlink_range_rel: Option<f64>,
    chainlink_last_lag_ms: Option<u64>,
    up_best_bid: Option<f64>,
    up_best_ask: Option<f64>,
    down_best_bid: Option<f64>,
    down_best_ask: Option<f64>,
    up_mid: Option<f64>,
    down_mid: Option<f64>,
    clob_trade_count_10s: usize,
    clob_trade_notional_10s: f64,
    clob_last_trade_price: Option<f64>,
    up_book_levels: usize,
    down_book_levels: usize,
}

#[derive(Clone, Debug)]
struct BookState {
    bids: BTreeMap<i64, f64>,
    asks: BTreeMap<i64, f64>,
}

impl BookState {
    fn new() -> Self {
        Self {
            bids: BTreeMap::new(),
            asks: BTreeMap::new(),
        }
    }

    fn clear_and_fill(&mut self, bids: &Value, asks: &Value) {
        self.bids.clear();
        self.asks.clear();
        if let Some(arr) = bids.as_array() {
            for level in arr {
                let p = level.get("price").and_then(as_f64);
                let s = level.get("size").and_then(as_f64);
                if let (Some(price), Some(size)) = (p, s) {
                    if size > 0.0 {
                        self.bids.insert(price_to_key(price), size);
                    }
                }
            }
        }
        if let Some(arr) = asks.as_array() {
            for level in arr {
                let p = level.get("price").and_then(as_f64);
                let s = level.get("size").and_then(as_f64);
                if let (Some(price), Some(size)) = (p, s) {
                    if size > 0.0 {
                        self.asks.insert(price_to_key(price), size);
                    }
                }
            }
        }
    }

    fn apply_delta(&mut self, side: &str, price: f64, size: f64) {
        let key = price_to_key(price);
        let side_upper = side.to_ascii_uppercase();
        let target = if side_upper == "BUY" {
            &mut self.bids
        } else {
            &mut self.asks
        };
        if size <= 0.0 {
            target.remove(&key);
        } else {
            target.insert(key, size);
        }
    }

    fn best_bid(&self) -> Option<f64> {
        self.bids.keys().next_back().map(|k| key_to_price(*k))
    }

    fn best_ask(&self) -> Option<f64> {
        self.asks.keys().next().map(|k| key_to_price(*k))
    }
}

struct FeatureEngineState {
    window: ActiveWindow,
    ticks: VecDeque<RtdsTick>,
    up_book: BookState,
    down_book: BookState,
    trades_10s: VecDeque<(u64, f64)>,
    last_trade_price: Option<f64>,
}

impl FeatureEngineState {
    fn new(window: ActiveWindow) -> Self {
        Self {
            window,
            ticks: VecDeque::new(),
            up_book: BookState::new(),
            down_book: BookState::new(),
            trades_10s: VecDeque::new(),
            last_trade_price: None,
        }
    }

    fn reset_window(&mut self, window: ActiveWindow) {
        self.window = window;
        self.ticks.clear();
        self.up_book = BookState::new();
        self.down_book = BookState::new();
        self.trades_10s.clear();
        self.last_trade_price = None;
    }

    fn on_tick(&mut self, tick: RtdsTick) {
        self.ticks.push_back(tick);
        while self.ticks.len() > 5_000 {
            self.ticks.pop_front();
        }
        let floor_ms = self.window.end_date_ms.saturating_sub(300_000);
        while let Some(front) = self.ticks.front() {
            if front.source_ts_ms < floor_ms {
                self.ticks.pop_front();
            } else {
                break;
            }
        }
    }

    fn on_clob_event(&mut self, ev: &ClobEventRecord) {
        match ev.event_type.as_str() {
            "book" => {
                let asset = ev.asset_id.as_deref().unwrap_or_default();
                let book = if asset == self.window.token_up {
                    &mut self.up_book
                } else if asset == self.window.token_down {
                    &mut self.down_book
                } else {
                    return;
                };
                let bids = ev.payload.get("bids").unwrap_or(&Value::Null);
                let asks = ev.payload.get("asks").unwrap_or(&Value::Null);
                book.clear_and_fill(bids, asks);
            }
            "price_change" => {
                if let Some(changes) = ev.payload.get("price_changes").and_then(|v| v.as_array()) {
                    for c in changes {
                        let asset = c
                            .get("asset_id")
                            .and_then(|v| v.as_str())
                            .unwrap_or_default()
                            .to_owned();
                        let price = c.get("price").and_then(as_f64);
                        let size = c.get("size").and_then(as_f64);
                        let side = c.get("side").and_then(|v| v.as_str()).unwrap_or("SELL");
                        if let (Some(price), Some(size)) = (price, size) {
                            if asset == self.window.token_up {
                                self.up_book.apply_delta(side, price, size);
                            } else if asset == self.window.token_down {
                                self.down_book.apply_delta(side, price, size);
                            }
                        }
                    }
                }
            }
            "last_trade_price" => {
                if let Some(price) = ev.payload.get("price").and_then(as_f64) {
                    self.last_trade_price = Some(price);
                    let notional = ev.payload.get("size").and_then(as_f64).unwrap_or(0.0) * price;
                    self.trades_10s.push_back((ev.recv_ts_ms, notional));
                }
            }
            _ => {}
        }
        let floor = now_ms().saturating_sub(10_000);
        while let Some((ts, _)) = self.trades_10s.front() {
            if *ts < floor {
                self.trades_10s.pop_front();
            } else {
                break;
            }
        }
    }

    fn snapshot(&self) -> FeatureSnapshot {
        let mut open = None;
        let mut high = None;
        let mut low = None;
        let mut close = None;
        let mut last_lag = None;
        for t in &self.ticks {
            open.get_or_insert(t.price);
            close = Some(t.price);
            high = Some(high.map_or(t.price, |h: f64| h.max(t.price)));
            low = Some(low.map_or(t.price, |l: f64| l.min(t.price)));
            last_lag = Some(t.recv_ts_ms.saturating_sub(t.source_ts_ms));
        }
        let range_abs = match (high, low) {
            (Some(h), Some(l)) => Some(h - l),
            _ => None,
        };
        let range_rel = match (range_abs, open) {
            (Some(r), Some(o)) if o > 0.0 => Some(r / o),
            _ => None,
        };

        let up_bid = self.up_book.best_bid();
        let up_ask = self.up_book.best_ask();
        let down_bid = self.down_book.best_bid();
        let down_ask = self.down_book.best_ask();

        FeatureSnapshot {
            ts_ms: now_ms(),
            window_epoch: self.window.window_epoch,
            window_slug: self.window.slug.clone(),
            chainlink_open: open,
            chainlink_high: high,
            chainlink_low: low,
            chainlink_close: close,
            chainlink_tick_count: self.ticks.len(),
            chainlink_range_abs: range_abs,
            chainlink_range_rel: range_rel,
            chainlink_last_lag_ms: last_lag,
            up_best_bid: up_bid,
            up_best_ask: up_ask,
            down_best_bid: down_bid,
            down_best_ask: down_ask,
            up_mid: midpoint(up_bid, up_ask),
            down_mid: midpoint(down_bid, down_ask),
            clob_trade_count_10s: self.trades_10s.len(),
            clob_trade_notional_10s: self.trades_10s.iter().map(|(_, n)| *n).sum(),
            clob_last_trade_price: self.last_trade_price,
            up_book_levels: self.up_book.bids.len() + self.up_book.asks.len(),
            down_book_levels: self.down_book.bids.len() + self.down_book.asks.len(),
        }
    }
}

#[derive(Clone, Debug)]
enum WriterMsg {
    WindowChanged(ActiveWindow),
    Raw(RawEvent),
    Feature(FeatureSnapshot),
}

#[tokio::main]
async fn main() -> Result<()> {
    let args: Vec<String> = env::args().collect();
    let subcommand = args.get(1).map(|s| s.as_str()).unwrap_or("collect");
    match subcommand {
        "collect" => run_collect(parse_collect_cfg(&args)?).await,
        "validate" => run_validate(parse_validate_cfg(&args)?).await,
        _ => {
            eprintln!("Usage:");
            eprintln!("  low_latency_collector collect [--out DIR] [--duration-sec N]");
            eprintln!("  low_latency_collector validate [--out DIR] --window-epoch N");
            Ok(())
        }
    }
}

#[derive(Clone, Debug)]
struct CollectCfg {
    out_dir: PathBuf,
    duration_sec: Option<u64>,
}

#[derive(Clone, Debug)]
struct ValidateCfg {
    out_dir: PathBuf,
    window_epoch: u64,
}

fn parse_collect_cfg(args: &[String]) -> Result<CollectCfg> {
    let mut out_dir = PathBuf::from("data_low_latency");
    let mut duration_sec = None;
    let mut i = 2;
    while i < args.len() {
        match args[i].as_str() {
            "--out" => {
                i += 1;
                let val = args.get(i).context("missing value for --out")?;
                out_dir = PathBuf::from(val);
            }
            "--duration-sec" => {
                i += 1;
                let val = args.get(i).context("missing value for --duration-sec")?;
                duration_sec = Some(val.parse::<u64>().context("invalid --duration-sec")?);
            }
            other => anyhow::bail!("unknown option: {}", other),
        }
        i += 1;
    }
    Ok(CollectCfg {
        out_dir,
        duration_sec,
    })
}

fn parse_validate_cfg(args: &[String]) -> Result<ValidateCfg> {
    let mut out_dir = PathBuf::from("data_low_latency");
    let mut window_epoch = None;
    let mut i = 2;
    while i < args.len() {
        match args[i].as_str() {
            "--out" => {
                i += 1;
                let val = args.get(i).context("missing value for --out")?;
                out_dir = PathBuf::from(val);
            }
            "--window-epoch" => {
                i += 1;
                let val = args.get(i).context("missing value for --window-epoch")?;
                window_epoch = Some(val.parse::<u64>().context("invalid --window-epoch")?);
            }
            other => anyhow::bail!("unknown option: {}", other),
        }
        i += 1;
    }
    Ok(ValidateCfg {
        out_dir,
        window_epoch: window_epoch.context("--window-epoch is required")?,
    })
}

async fn run_collect(cfg: CollectCfg) -> Result<()> {
    create_dir_all(&cfg.out_dir).await?;
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .user_agent("Mozilla/5.0 (low-latency-btc-collector)")
        .build()?;

    let initial = fetch_active_window(&client).await?;
    eprintln!(
        "[collector] start window={} up={} down={}",
        initial.slug, initial.token_up, initial.token_down
    );

    let (window_tx, window_rx) = watch::channel(initial.clone());
    let (engine_tx, engine_rx) = mpsc::channel::<EngineEvent>(40_000);
    let (writer_tx, writer_rx) = mpsc::channel::<WriterMsg>(40_000);

    writer_tx
        .send(WriterMsg::WindowChanged(initial.clone()))
        .await
        .ok();
    writer_tx
        .send(WriterMsg::Raw(RawEvent::WindowChanged {
            recv_ts_ms: now_ms(),
            window: initial.clone(),
        }))
        .await
        .ok();
    engine_tx
        .send(EngineEvent::WindowChanged(initial.clone()))
        .await
        .ok();

    let writer_task = tokio::spawn(writer_loop(cfg.out_dir.clone(), writer_rx));
    let engine_task = tokio::spawn(feature_engine_loop(initial.clone(), engine_rx, writer_tx.clone()));
    let gamma_task = tokio::spawn(gamma_window_loop(client, window_tx.clone(), engine_tx.clone(), writer_tx.clone()));
    let rtds_task = tokio::spawn(rtds_loop(window_rx.clone(), engine_tx.clone(), writer_tx.clone()));
    let clob_task = tokio::spawn(clob_loop(window_rx, engine_tx, writer_tx.clone()));

    if let Some(sec) = cfg.duration_sec {
        let deadline = Instant::now() + Duration::from_secs(sec);
        time::sleep_until(deadline).await;
        eprintln!("[collector] duration reached, stopping");
    } else {
        eprintln!("[collector] running forever, Ctrl+C to stop");
        tokio::signal::ctrl_c().await?;
    }

    gamma_task.abort();
    rtds_task.abort();
    clob_task.abort();
    engine_task.abort();
    drop(writer_tx);
    let _ = writer_task.await?;
    Ok(())
}

async fn feature_engine_loop(
    initial: ActiveWindow,
    mut rx: mpsc::Receiver<EngineEvent>,
    tx_writer: mpsc::Sender<WriterMsg>,
) -> Result<()> {
    let mut state = FeatureEngineState::new(initial);
    let mut tick = time::interval(Duration::from_millis(200));
    tick.set_missed_tick_behavior(time::MissedTickBehavior::Skip);
    loop {
        tokio::select! {
            _ = tick.tick() => {
                let snap = state.snapshot();
                if tx_writer.send(WriterMsg::Feature(snap)).await.is_err() {
                    break;
                }
            }
            maybe = rx.recv() => {
                let Some(msg) = maybe else { break; };
                match msg {
                    EngineEvent::WindowChanged(w) => state.reset_window(w),
                    EngineEvent::RtdsTick(t) => state.on_tick(t),
                    EngineEvent::ClobEvent(ev) => state.on_clob_event(&ev),
                }
            }
        }
    }
    Ok(())
}

async fn writer_loop(base_dir: PathBuf, mut rx: mpsc::Receiver<WriterMsg>) -> Result<()> {
    let mut active_epoch: Option<u64> = None;
    let mut raw_writer: Option<BufWriter<File>> = None;
    let mut feature_writer: Option<BufWriter<File>> = None;
    let mut flush_tick = time::interval(Duration::from_millis(250));
    flush_tick.set_missed_tick_behavior(time::MissedTickBehavior::Skip);

    loop {
        tokio::select! {
            _ = flush_tick.tick() => {
                if let Some(w) = raw_writer.as_mut() {
                    w.flush().await.ok();
                }
                if let Some(w) = feature_writer.as_mut() {
                    w.flush().await.ok();
                }
            }
            maybe = rx.recv() => {
                let Some(msg) = maybe else { break; };
                match msg {
                    WriterMsg::WindowChanged(w) => {
                        if active_epoch == Some(w.window_epoch) {
                            continue;
                        }
                        let window_dir = base_dir.join(format!("window_{}", w.window_epoch));
                        create_dir_all(&window_dir).await?;
                        raw_writer = Some(open_ndjson_writer(&window_dir.join("raw.ndjson")).await?);
                        feature_writer = Some(open_ndjson_writer(&window_dir.join("features.ndjson")).await?);
                        active_epoch = Some(w.window_epoch);
                    }
                    WriterMsg::Raw(raw) => {
                        if raw_writer.is_none() {
                            continue;
                        }
                        let payload = serde_json::to_vec(&raw)?;
                        if let Some(w) = raw_writer.as_mut() {
                            w.write_all(&payload).await?;
                            w.write_all(b"\n").await?;
                        }
                    }
                    WriterMsg::Feature(feature) => {
                        if feature_writer.is_none() {
                            continue;
                        }
                        let payload = serde_json::to_vec(&feature)?;
                        if let Some(w) = feature_writer.as_mut() {
                            w.write_all(&payload).await?;
                            w.write_all(b"\n").await?;
                        }
                    }
                }
            }
        }
    }

    if let Some(w) = raw_writer.as_mut() {
        w.flush().await.ok();
    }
    if let Some(w) = feature_writer.as_mut() {
        w.flush().await.ok();
    }
    Ok(())
}

async fn open_ndjson_writer(path: &Path) -> Result<BufWriter<File>> {
    let file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .await
        .with_context(|| format!("failed to open {}", path.display()))?;
    Ok(BufWriter::new(file))
}

async fn gamma_window_loop(
    client: reqwest::Client,
    tx_window: watch::Sender<ActiveWindow>,
    tx_engine: mpsc::Sender<EngineEvent>,
    tx_writer: mpsc::Sender<WriterMsg>,
) -> Result<()> {
    let mut current = tx_window.borrow().clone();
    loop {
        let sleep = rotate_sleep_seconds();
        time::sleep(Duration::from_secs_f64(sleep)).await;
        match fetch_active_window(&client).await {
            Ok(next) => {
                if next.window_epoch != current.window_epoch {
                    eprintln!("[gamma] window rotated {} -> {}", current.slug, next.slug);
                    current = next.clone();
                    tx_window.send(next.clone()).ok();
                    tx_engine.send(EngineEvent::WindowChanged(next.clone())).await.ok();
                    tx_writer.send(WriterMsg::WindowChanged(next.clone())).await.ok();
                    tx_writer.send(WriterMsg::Raw(RawEvent::WindowChanged {
                        recv_ts_ms: now_ms(),
                        window: next,
                    })).await.ok();
                }
            }
            Err(err) => {
                eprintln!("[gamma] poll error: {}", err);
            }
        }
    }
}

fn rotate_sleep_seconds() -> f64 {
    let now = now_ms() as f64 / 1000.0;
    let next_boundary = ((now as u64 / 300) + 1) * 300;
    let until = next_boundary as f64 - now;
    if until < 12.0 {
        0.8
    } else {
        5.0
    }
}

async fn fetch_active_window(client: &reqwest::Client) -> Result<ActiveWindow> {
    let epoch = (now_ms() / 1000 / 300) * 300;
    for candidate in [epoch, epoch + 300, epoch.saturating_sub(300)] {
        let slug = format!("btc-updown-5m-{}", candidate);
        if let Some(win) = fetch_window_by_slug(client, &slug).await? {
            return Ok(win);
        }
    }
    anyhow::bail!("failed to find active btc-updown-5m window")
}

async fn fetch_window_by_slug(client: &reqwest::Client, slug: &str) -> Result<Option<ActiveWindow>> {
    let url = format!("{}/events?slug={}", GAMMA_BASE, slug);
    let events: Vec<GammaEvent> = client.get(url).send().await?.json().await?;
    let Some(event) = events.into_iter().next() else {
        return Ok(None);
    };
    if event.closed {
        return Ok(None);
    }
    let Some(mkt) = event.markets.into_iter().next() else {
        return Ok(None);
    };
    let ids: Vec<String> = serde_json::from_str(&mkt.clob_token_ids_raw).unwrap_or_default();
    let outcomes: Vec<String> = serde_json::from_str(&mkt.outcomes).unwrap_or_default();
    if ids.len() < 2 {
        return Ok(None);
    }
    let (token_up, token_down) = if outcomes.first().map(|s| s.as_str()) == Some("Up") {
        (ids[0].clone(), ids[1].clone())
    } else {
        (ids[1].clone(), ids[0].clone())
    };
    let end_date_ms = chrono::DateTime::parse_from_rfc3339(&mkt.end_date)
        .map(|dt| dt.timestamp_millis() as u64)
        .unwrap_or(0);
    Ok(Some(ActiveWindow {
        window_epoch: parse_epoch_from_slug(&event.slug).unwrap_or(0),
        slug: event.slug,
        market_id: mkt.condition_id,
        token_up,
        token_down,
        end_date_ms,
    }))
}

fn parse_epoch_from_slug(slug: &str) -> Option<u64> {
    slug.rsplit('-').next()?.parse::<u64>().ok()
}

async fn rtds_loop(
    _window_rx: watch::Receiver<ActiveWindow>,
    tx_engine: mpsc::Sender<EngineEvent>,
    tx_writer: mpsc::Sender<WriterMsg>,
) -> Result<()> {
    let mut backoff = 1.0f64;
    loop {
        match rtds_once(tx_engine.clone(), tx_writer.clone()).await {
            Ok(_) => eprintln!("[rtds] disconnected"),
            Err(err) => eprintln!("[rtds] error: {}", err),
        }
        time::sleep(Duration::from_secs_f64((backoff + jitter()).min(15.0))).await;
        backoff = (backoff * 2.0).min(15.0);
    }
}

async fn rtds_once(tx_engine: mpsc::Sender<EngineEvent>, tx_writer: mpsc::Sender<WriterMsg>) -> Result<()> {
    let (ws, _) = connect_async(RTDS_WS).await?;
    let (mut write, mut read) = ws.split();
    let sub = json!({
        "action": "subscribe",
        "subscriptions": [
            {"topic":"crypto_prices_chainlink","type":"*","filters":""}
        ]
    });
    write.send(Message::Text(sub.to_string().into())).await?;
    let mut ping = time::interval(Duration::from_secs(5));
    let mut refresh = time::interval(Duration::from_millis(5_500));
    let mut last_data = now_ms();
    loop {
        tokio::select! {
            _ = ping.tick() => {
                write.send(Message::Text("PING".into())).await.ok();
            }
            _ = refresh.tick() => {
                if now_ms().saturating_sub(last_data) >= 5_500 {
                    write.send(Message::Text(sub.to_string().into())).await.ok();
                }
            }
            msg = read.next() => {
                let Some(msg) = msg else { anyhow::bail!("RTDS stream ended"); };
                let msg = msg?;
                match msg {
                    Message::Text(text) => {
                        if text.trim().is_empty() || text.trim() == "PONG" { continue; }
                        last_data = now_ms();
                        handle_rtds_payload(&text, tx_engine.clone(), tx_writer.clone()).await?;
                    }
                    Message::Binary(bin) => {
                        let text = String::from_utf8_lossy(&bin).to_string();
                        if text.trim().is_empty() || text.trim() == "PONG" { continue; }
                        last_data = now_ms();
                        handle_rtds_payload(&text, tx_engine.clone(), tx_writer.clone()).await?;
                    }
                    Message::Ping(data) => {
                        write.send(Message::Pong(data)).await.ok();
                    }
                    Message::Close(_) => anyhow::bail!("RTDS close"),
                    _ => {}
                }
            }
        }
    }
}

async fn handle_rtds_payload(
    text: &str,
    tx_engine: mpsc::Sender<EngineEvent>,
    tx_writer: mpsc::Sender<WriterMsg>,
) -> Result<()> {
    let msg: Value = match serde_json::from_str(text) {
        Ok(v) => v,
        Err(_) => return Ok(()),
    };
    let topic = msg.get("topic").and_then(|v| v.as_str()).unwrap_or("");
    let mtype = msg.get("type").and_then(|v| v.as_str()).unwrap_or("");
    if topic != "crypto_prices_chainlink" || mtype != "update" {
        return Ok(());
    }
    let payload = msg.get("payload").cloned().unwrap_or(Value::Null);
    let symbol = payload
        .get("symbol")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    if symbol != CHAINLINK_SYMBOL {
        return Ok(());
    }
    let raw_value = payload.get("value").and_then(as_f64).unwrap_or(0.0);
    if raw_value <= 0.0 {
        return Ok(());
    }
    let source_ts_ms = payload.get("timestamp").and_then(|v| v.as_u64()).unwrap_or(0);
    let message_ts_ms = msg.get("timestamp").and_then(|v| v.as_u64()).unwrap_or(0);
    let price = decode_price(raw_value);
    let recv = now_ms();
    let tick = RtdsTick {
        source_ts_ms,
        recv_ts_ms: recv,
        message_ts_ms,
        price,
        raw_value,
        symbol,
    };
    tx_engine.send(EngineEvent::RtdsTick(tick.clone())).await.ok();
    tx_writer
        .send(WriterMsg::Raw(RawEvent::RtdsTick {
            recv_ts_ms: recv,
            tick,
        }))
        .await
        .ok();
    Ok(())
}

fn decode_price(raw: f64) -> f64 {
    if raw > 1e15 {
        raw / 1e18
    } else {
        raw
    }
}

async fn clob_loop(
    mut window_rx: watch::Receiver<ActiveWindow>,
    tx_engine: mpsc::Sender<EngineEvent>,
    tx_writer: mpsc::Sender<WriterMsg>,
) -> Result<()> {
    let mut current = window_rx.borrow().clone();
    loop {
        if let Err(err) = clob_once(&current, &mut window_rx, tx_engine.clone(), tx_writer.clone()).await {
            eprintln!("[clob] error for {}: {}", current.slug, err);
            time::sleep(Duration::from_secs(2)).await;
        }
        current = window_rx.borrow().clone();
    }
}

async fn clob_once(
    window: &ActiveWindow,
    window_rx: &mut watch::Receiver<ActiveWindow>,
    tx_engine: mpsc::Sender<EngineEvent>,
    tx_writer: mpsc::Sender<WriterMsg>,
) -> Result<()> {
    eprintln!("[clob] subscribe {}", window.slug);
    let (ws, _) = connect_async(CLOB_WS).await?;
    let (mut write, mut read) = ws.split();
    let sub = json!({
        "type":"market",
        "assets_ids":[window.token_up.clone(), window.token_down.clone()],
        "level":2,
        "initial_dump":true,
        "custom_feature_enabled":true
    });
    write.send(Message::Text(sub.to_string().into())).await?;
    let mut ping = time::interval(Duration::from_secs(10));
    loop {
        tokio::select! {
            _ = ping.tick() => {
                write.send(Message::Text("PING".into())).await.ok();
            }
            changed = window_rx.changed() => {
                if changed.is_ok() {
                    let next = window_rx.borrow().clone();
                    if next.window_epoch != window.window_epoch {
                        eprintln!("[clob] window switch {} -> {}", window.slug, next.slug);
                        return Ok(());
                    }
                }
            }
            msg = read.next() => {
                let Some(msg) = msg else { anyhow::bail!("CLOB stream ended"); };
                let msg = msg?;
                match msg {
                    Message::Text(text) => {
                        handle_clob_payload(&text, tx_engine.clone(), tx_writer.clone()).await?;
                    }
                    Message::Binary(bin) => {
                        let text = String::from_utf8_lossy(&bin).to_string();
                        handle_clob_payload(&text, tx_engine.clone(), tx_writer.clone()).await?;
                    }
                    Message::Ping(data) => {
                        write.send(Message::Pong(data)).await.ok();
                    }
                    Message::Close(_) => anyhow::bail!("CLOB close"),
                    _ => {}
                }
            }
        }
    }
}

async fn handle_clob_payload(
    text: &str,
    tx_engine: mpsc::Sender<EngineEvent>,
    tx_writer: mpsc::Sender<WriterMsg>,
) -> Result<()> {
    let recv_ts_ms = now_ms();
    let parsed: Value = match serde_json::from_str(text) {
        Ok(v) => v,
        Err(_) => return Ok(()),
    };
    let events: Vec<Value> = if let Some(arr) = parsed.as_array() {
        arr.clone()
    } else {
        vec![parsed]
    };
    for ev in events {
        let event_type = ev
            .get("event_type")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_owned();
        if event_type.is_empty() {
            continue;
        }
        let event_ts_ms = ev
            .get("timestamp")
            .and_then(|v| v.as_str())
            .and_then(|s| s.parse::<u64>().ok())
            .or_else(|| ev.get("timestamp").and_then(|v| v.as_u64()))
            .unwrap_or(0);
        let asset_id = ev.get("asset_id").and_then(|v| v.as_str()).map(|s| s.to_owned());
        let record = ClobEventRecord {
            event_type,
            recv_ts_ms,
            event_ts_ms,
            asset_id,
            payload: ev,
        };
        tx_engine.send(EngineEvent::ClobEvent(record.clone())).await.ok();
        tx_writer
            .send(WriterMsg::Raw(RawEvent::ClobEvent {
                recv_ts_ms,
                event: record,
            }))
            .await
            .ok();
    }
    Ok(())
}

async fn run_validate(cfg: ValidateCfg) -> Result<()> {
    let window_dir = cfg.out_dir.join(format!("window_{}", cfg.window_epoch));
    let raw = window_dir.join("raw.ndjson");
    let feat = window_dir.join("features.ndjson");
    eprintln!("[validate] reading {}", raw.display());
    let mut raw_reader = BufReader::new(File::open(&raw).await?);
    let mut buf = String::new();
    let mut rtds_count = 0usize;
    let mut clob_count = 0usize;
    let mut window_changes = 0usize;
    let mut lag_values: Vec<u64> = Vec::new();
    let mut prices: Vec<f64> = Vec::new();
    while raw_reader.read_line(&mut buf).await? > 0 {
        let line = buf.trim();
        if line.is_empty() {
            buf.clear();
            continue;
        }
        let val: Value = serde_json::from_str(line).unwrap_or(Value::Null);
        let kind = val.get("kind").and_then(|v| v.as_str()).unwrap_or("");
        match kind {
            "rtds_tick" => {
                rtds_count += 1;
                let recv = val.get("recv_ts_ms").and_then(|v| v.as_u64()).unwrap_or(0);
                let src = val
                    .get("tick")
                    .and_then(|t| t.get("source_ts_ms"))
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0);
                if recv > src && src > 0 {
                    lag_values.push(recv - src);
                }
                if let Some(p) = val
                    .get("tick")
                    .and_then(|t| t.get("price"))
                    .and_then(|v| v.as_f64())
                {
                    prices.push(p);
                }
            }
            "clob_event" => clob_count += 1,
            "window_changed" => window_changes += 1,
            _ => {}
        }
        buf.clear();
    }

    let mut feature_reader = BufReader::new(File::open(&feat).await?);
    let mut fbuf = String::new();
    let mut feature_count = 0usize;
    while feature_reader.read_line(&mut fbuf).await? > 0 {
        if !fbuf.trim().is_empty() {
            feature_count += 1;
        }
        fbuf.clear();
    }

    let lag_median = percentile_u64(&lag_values, 50.0);
    let lag_p95 = percentile_u64(&lag_values, 95.0);
    let price_min = prices.iter().copied().reduce(f64::min);
    let price_max = prices.iter().copied().reduce(f64::max);
    let price_open = prices.first().copied();
    let price_close = prices.last().copied();
    println!("window_epoch={}", cfg.window_epoch);
    println!("raw_rtds_ticks={}", rtds_count);
    println!("raw_clob_events={}", clob_count);
    println!("raw_window_changes={}", window_changes);
    println!("feature_snapshots={}", feature_count);
    println!(
        "rtds_lag_ms_median={} p95={}",
        lag_median.unwrap_or(0),
        lag_p95.unwrap_or(0)
    );
    println!(
        "rtds_ohlc=open:{:?} high:{:?} low:{:?} close:{:?}",
        price_open, price_max, price_min, price_close
    );
    Ok(())
}

fn percentile_u64(values: &[u64], p: f64) -> Option<u64> {
    if values.is_empty() {
        return None;
    }
    let mut data = values.to_vec();
    data.sort_unstable();
    let idx = ((p / 100.0) * ((data.len() - 1) as f64)).round() as usize;
    data.get(idx).copied()
}

fn as_f64(v: &Value) -> Option<f64> {
    if let Some(f) = v.as_f64() {
        return Some(f);
    }
    v.as_str().and_then(|s| s.parse::<f64>().ok())
}

fn midpoint(bid: Option<f64>, ask: Option<f64>) -> Option<f64> {
    match (bid, ask) {
        (Some(b), Some(a)) => Some((b + a) / 2.0),
        _ => None,
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

fn jitter() -> f64 {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .subsec_nanos();
    (nanos as f64 % 1_000_000_000.0) / 1_000_000_000.0
}

fn price_to_key(price: f64) -> i64 {
    (price * 1_000_000.0).round() as i64
}

fn key_to_price(key: i64) -> f64 {
    key as f64 / 1_000_000.0
}
