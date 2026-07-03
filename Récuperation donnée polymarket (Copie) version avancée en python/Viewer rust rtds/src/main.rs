mod gamma;
mod models;
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
use std::io;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::time;

#[tokio::main]
async fn main() -> Result<()> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let result = run(&mut terminal).await;

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    if let Err(e) = result {
        eprintln!("Error: {}", e);
    }
    Ok(())
}

async fn run(terminal: &mut ratatui::Terminal<CrosstermBackend<io::Stdout>>) -> Result<()> {
    let state = Arc::new(Mutex::new(AppState::default()));
    let http_client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()?;

    // ── Tâche 1: Market discovery (toutes les 5 min) ─────────────────────────
    let state_gamma = state.clone();
    let http_gamma = http_client.clone();
    tokio::spawn(async move {
        loop {
            match gamma::fetch_active_market(&http_gamma).await {
                Ok(market_info) => {
                    let next_slot = gamma::next_slot_ts();
                    let token_up = market_info.token_up.clone();
                    let token_down = market_info.token_down.clone();
                    let end_ms = market_info.end_date_ms;
                    let window_start_ms = end_ms.saturating_sub(300_000);

                    {
                        let mut st = state_gamma.lock().unwrap();
                        if st.market.as_ref().map(|m| &m.slug) != Some(&market_info.slug) {
                            st.reset_for_new_market(token_up, token_down);
                            // Recalculer la bougie pour la nouvelle fenêtre
                            st.update_chainlink_candle(window_start_ms, end_ms);
                        }
                        st.market = Some(market_info);
                        st.next_slot_ts = next_slot;
                    }

                    let now_ms = chrono::Utc::now().timestamp_millis() as u64;
                    let wait_ms = if end_ms > now_ms {
                        end_ms - now_ms + 2000
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

    // ── Tâche 2: RTDS WebSocket (Chainlink BTC/USD + Binance) ────────────────
    let state_rtds = state.clone();
    tokio::spawn(async move {
        ws_rtds::run_rtds_ws(state_rtds).await;
    });

    // ── Tâche 3: CLOB WebSocket (orderbook + trades marché) ──────────────────
    let state_clob = state.clone();
    tokio::spawn(async move {
        let mut current_slug = String::new();
        loop {
            let market_info = {
                let st = state_clob.lock().unwrap();
                st.market.clone()
            };

            if let Some(ref m) = market_info {
                if m.slug != current_slug {
                    current_slug = m.slug.clone();
                    let token_up = m.token_up.clone();
                    let token_down = m.token_down.clone();
                    let state_ws = state_clob.clone();
                    tokio::spawn(async move {
                        ws_clob::run_clob_ws(state_ws, token_up, token_down).await.ok();
                    });
                }
            }
            time::sleep(Duration::from_secs(1)).await;
        }
    });

    // ── Boucle UI (100ms = 10 fps) ────────────────────────────────────────────
    let tick_rate = Duration::from_millis(100);

    loop {
        terminal.draw(|f| ui::draw(f, &state))?;

        if event::poll(tick_rate)? {
            if let Event::Key(key) = event::read()? {
                if key.kind == KeyEventKind::Press {
                    match key.code {
                        KeyCode::Char('q') | KeyCode::Char('Q') | KeyCode::Esc => break,
                        _ => {}
                    }
                }
            }
        }
    }

    Ok(())
}
