mod gamma;
mod models;
mod record;
mod ui;
mod ws_clob;
mod ws_rtds;

use anyhow::Result;
use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use models::AppState;
use ratatui::{backend::CrosstermBackend, Terminal};
use record::RecordEvent;
use std::env;
use std::io;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio::time;

#[derive(Debug, Clone)]
struct Args {
    out_dir: PathBuf,
    no_record: bool,
    duration_sec: Option<u64>,
}

fn parse_args() -> Result<Args> {
    let mut out_dir = PathBuf::from("../data_low_latency");
    let mut no_record = false;
    let mut duration_sec = None;
    let args: Vec<String> = env::args().collect();
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--out" => {
                i += 1;
                let v = args.get(i).ok_or_else(|| anyhow::anyhow!("missing value for --out"))?;
                out_dir = PathBuf::from(v);
            }
            "--no-record" => no_record = true,
            "--duration-sec" => {
                i += 1;
                let v = args
                    .get(i)
                    .ok_or_else(|| anyhow::anyhow!("missing value for --duration-sec"))?;
                duration_sec = Some(v.parse::<u64>()?);
            }
            "--help" | "-h" => {
                println!("Rustector_btc_5mn [--out DIR] [--no-record] [--duration-sec N]");
                std::process::exit(0);
            }
            unknown => return Err(anyhow::anyhow!("unknown argument: {}", unknown)),
        }
        i += 1;
    }
    Ok(Args {
        out_dir,
        no_record,
        duration_sec,
    })
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = parse_args()?;
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let result = run(&mut terminal, args).await;

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    if let Err(e) = result {
        eprintln!("Error: {}", e);
    }
    Ok(())
}

async fn run(
    terminal: &mut ratatui::Terminal<CrosstermBackend<io::Stdout>>,
    args: Args,
) -> Result<()> {
    let state = Arc::new(Mutex::new(AppState::default()));
    if let Ok(mut st) = state.lock() {
        st.recording_enabled = !args.no_record;
    }
    let http_client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .user_agent("Mozilla/5.0 (Rustector_btc_5mn)")
        .build()?;

    let recorder_tx = if args.no_record {
        None
    } else {
        let (tx, rx) = mpsc::unbounded_channel::<RecordEvent>();
        let state_writer = state.clone();
        let out = args.out_dir.clone();
        tokio::spawn(async move {
            if let Err(e) = record::start_writer(out, state_writer, rx).await {
                eprintln!("[REC] writer error: {}", e);
            }
        });
        Some(tx)
    };

    // Market discovery + window rotation
    let state_gamma = state.clone();
    let http_gamma = http_client.clone();
    let rec_gamma = recorder_tx.clone();
    tokio::spawn(async move {
        loop {
            match gamma::fetch_active_market(&http_gamma).await {
                Ok(market_info) => {
                    let mut changed = false;
                    {
                        let mut st = state_gamma.lock().unwrap();
                        if st.market.as_ref().map(|m| &m.slug) != Some(&market_info.slug) {
                            st.reset_for_new_market();
                            st.market = Some(market_info.clone());
                            changed = true;
                        } else {
                            st.market = Some(market_info.clone());
                        }
                    }
                    if changed {
                        if let Some(tx) = rec_gamma.clone() {
                            let _ = tx.send(RecordEvent::WindowChanged {
                                ts_ms: now_ms(),
                                window_slug: market_info.slug.clone(),
                                window_epoch: parse_epoch(&market_info.slug).unwrap_or(0),
                                token_up: market_info.token_up.clone(),
                                token_down: market_info.token_down.clone(),
                            });
                        }
                    }
                    let now_ms = now_ms();
                    let wait_ms = if market_info.end_date_ms > now_ms {
                        market_info.end_date_ms - now_ms + 2000
                    } else {
                        5000
                    };
                    time::sleep(Duration::from_millis(wait_ms.min(310_000))).await;
                }
                Err(e) => {
                    eprintln!("[Gamma] {}", e);
                    time::sleep(Duration::from_secs(10)).await;
                }
            }
        }
    });

    // RTDS collector
    let state_rtds = state.clone();
    let rec_rtds = recorder_tx.clone();
    tokio::spawn(async move {
        ws_rtds::run_rtds_ws(state_rtds, rec_rtds).await;
    });

    // CLOB collector with switch on slug
    let state_clob = state.clone();
    let rec_clob = recorder_tx.clone();
    tokio::spawn(async move {
        let mut current_slug = String::new();
        let mut current_handle: Option<JoinHandle<()>> = None;
        loop {
            let market = { state_clob.lock().unwrap().market.clone() };
            if let Some(m) = market {
                if m.slug != current_slug {
                    current_slug = m.slug.clone();
                    if let Some(h) = current_handle.take() {
                        h.abort();
                    }
                    let st = state_clob.clone();
                    let rec = rec_clob.clone();
                    current_handle = Some(tokio::spawn(async move {
                        let _ = ws_clob::run_clob_ws(st, m.token_up, m.token_down, rec).await;
                    }));
                }
            }
            time::sleep(Duration::from_secs(1)).await;
        }
    });

    let tick_rate = Duration::from_millis(100);
    let started = time::Instant::now();
    loop {
        terminal.draw(|f| ui::draw(f, &state))?;
        if let Some(max_sec) = args.duration_sec {
            if started.elapsed().as_secs() >= max_sec {
                break;
            }
        }
        if event::poll(tick_rate)? {
            if let Event::Key(key) = event::read()? {
                if key.kind == KeyEventKind::Press {
                    if matches!(key.code, KeyCode::Char('q') | KeyCode::Char('Q') | KeyCode::Esc) {
                        break;
                    }
                }
            }
        }
    }
    Ok(())
}

fn parse_epoch(slug: &str) -> Option<u64> {
    slug.rsplit('-').next()?.parse::<u64>().ok()
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
