use chrono::{DateTime, Local};
use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Cell, Paragraph, Row, Table},
    Frame,
};
use std::sync::{Arc, Mutex};

use crate::models::AppState;

pub fn draw(f: &mut Frame, state: &Arc<Mutex<AppState>>) {
    let st = state.lock().unwrap();
    let area = f.area();

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),  // header
            Constraint::Min(10),    // body
            Constraint::Length(3),  // footer
        ])
        .split(area);

    draw_header(f, &st, chunks[0]);
    draw_body(f, &st, chunks[1]);
    draw_footer(f, &st, chunks[2]);
}

// ─── HEADER ──────────────────────────────────────────────────────────────────

fn draw_header(f: &mut Frame, st: &AppState, area: Rect) {
    let now_ms = now_ms();

    let (title, countdown) = if let Some(ref m) = st.market {
        let remaining_secs = if m.end_date_ms > now_ms {
            (m.end_date_ms - now_ms) / 1000
        } else {
            0
        };
        let mm = remaining_secs / 60;
        let ss = remaining_secs % 60;
        (m.title.clone(), format!("{:02}:{:02}", mm, ss))
    } else {
        ("Recherche du marché actif...".to_string(), "--:--".to_string())
    };

    // Prix BTC Chainlink (source de résolution)
    let btc_cl = st.btc_price_chainlink
        .map(|p| format!("${:.2}", p))
        .unwrap_or_else(|| "---".to_string());

    // Age du dernier tick Chainlink
    let cl_age = if st.chainlink_timestamp_ms > 0 {
        let age_ms = now_ms.saturating_sub(st.chainlink_timestamp_ms);
        if age_ms < 2000 {
            format!("({:.0}ms)", age_ms)
        } else {
            format!("({:.1}s ago)", age_ms as f64 / 1000.0)
        }
    } else {
        "(---)".to_string()
    };

    // Prix BTC Binance (rapide, pour affichage seulement)
    let btc_bin = st.btc_price_binance
        .map(|p| format!("${:.2}", p))
        .unwrap_or_else(|| "---".to_string());

    let header_text = Line::from(vec![
        Span::styled(
            format!(" ⚡ {}", title),
            Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!(" [{}] ", countdown),
            Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
        ),
        Span::raw("  "),
        Span::styled("Chainlink: ", Style::default().fg(Color::DarkGray)),
        Span::styled(btc_cl, Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)),
        Span::styled(format!(" {} ", cl_age), Style::default().fg(Color::DarkGray)),
        Span::styled("Binance: ", Style::default().fg(Color::DarkGray)),
        Span::styled(btc_bin, Style::default().fg(Color::Cyan)),
    ]);

    let para = Paragraph::new(header_text)
        .block(Block::default().borders(Borders::ALL).border_style(Style::default().fg(Color::DarkGray)))
        .alignment(Alignment::Left);
    f.render_widget(para, area);
}

// ─── BODY ─────────────────────────────────────────────────────────────────────

fn draw_body(f: &mut Frame, st: &AppState, area: Rect) {
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(30),  // orderbook UP
            Constraint::Percentage(30),  // orderbook DOWN
            Constraint::Percentage(40),  // bougie BTC Chainlink + trades
        ])
        .split(area);

    draw_orderbook(f, st, cols[0], true);
    draw_orderbook(f, st, cols[1], false);
    draw_right_panel(f, st, cols[2]);
}

// ─── ORDERBOOK ────────────────────────────────────────────────────────────────

fn draw_orderbook(f: &mut Frame, st: &AppState, area: Rect, is_up: bool) {
    let (label, color, book) = if is_up {
        ("UP ▲", Color::Green, &st.book_up)
    } else {
        ("DOWN ▼", Color::Red, &st.book_down)
    };

    let best_bid = book.best_bid();
    let best_ask = book.best_ask();
    let mid = book.mid();
    let spread = match (best_bid, best_ask) {
        (Some(b), Some(a)) => Some(a - b),
        _ => None,
    };

    let title = format!(
        " {} | Bid:{} Ask:{} Spd:{}",
        label,
        best_bid.map(|p| format!("{:.3}", p)).unwrap_or("---".into()),
        best_ask.map(|p| format!("{:.3}", p)).unwrap_or("---".into()),
        spread.map(|s| format!("{:.3}", s)).unwrap_or("---".into()),
    );

    let max_levels = ((area.height.saturating_sub(4)) / 2) as usize;
    let max_levels = max_levels.max(3);

    let asks: Vec<(&ordered_float::OrderedFloat<f64>, &f64)> =
        book.asks.iter().take(max_levels).collect();
    let bids: Vec<(&ordered_float::OrderedFloat<f64>, &f64)> =
        book.bids.iter().rev().take(max_levels).collect();

    let mut rows: Vec<Row> = Vec::new();

    for (price, size) in asks.iter().rev() {
        rows.push(Row::new(vec![
            Cell::from(format!("{:.3}", price.0)).style(Style::default().fg(Color::Red)),
            Cell::from(format!("{:.1}", size)).style(Style::default().fg(Color::DarkGray)),
            Cell::from("SELL").style(Style::default().fg(Color::Red)),
        ]));
    }

    if let Some(m) = mid {
        rows.push(Row::new(vec![
            Cell::from(format!("──{:.3}──", m))
                .style(Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
            Cell::from(""),
            Cell::from("mid").style(Style::default().fg(Color::Yellow)),
        ]));
    }

    for (price, size) in &bids {
        rows.push(Row::new(vec![
            Cell::from(format!("{:.3}", price.0)).style(Style::default().fg(Color::Green)),
            Cell::from(format!("{:.1}", size)).style(Style::default().fg(Color::DarkGray)),
            Cell::from("BUY").style(Style::default().fg(Color::Green)),
        ]));
    }

    let widths = [
        Constraint::Length(9),
        Constraint::Length(9),
        Constraint::Length(5),
    ];

    let table = Table::new(rows, widths)
        .block(
            Block::default()
                .title(title)
                .borders(Borders::ALL)
                .border_style(Style::default().fg(color)),
        )
        .header(
            Row::new(vec!["Price", "Size ($)", "Side"])
                .style(Style::default().fg(Color::DarkGray).add_modifier(Modifier::BOLD)),
        );

    f.render_widget(table, area);
}

// ─── PANEL DROIT : bougie Chainlink + trades ──────────────────────────────────

fn draw_right_panel(f: &mut Frame, st: &AppState, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(12),   // bougie BTC Chainlink
            Constraint::Min(5),       // trades récents (marché Polymarket)
        ])
        .split(area);

    draw_chainlink_candle(f, st, chunks[0]);
    draw_recent_trades(f, st, chunks[1]);
}

fn draw_chainlink_candle(f: &mut Frame, st: &AppState, area: Rect) {
    let c = &st.chainlink_candle;

    let content = if !c.is_valid() {
        vec![
            Line::from(""),
            Line::from(Span::styled(
                "  En attente des ticks Chainlink...",
                Style::default().fg(Color::DarkGray),
            )),
            Line::from(Span::styled(
                "  (résolution officielle de l'événement)",
                Style::default().fg(Color::DarkGray),
            )),
        ]
    } else {
        let direction_span = match c.direction() {
            Some(true) => Span::styled(
                "▲ UP  (close ≥ open)",
                Style::default().fg(Color::Green).add_modifier(Modifier::BOLD),
            ),
            Some(false) => Span::styled(
                "▼ DOWN  (close < open)",
                Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
            ),
            None => Span::styled("En cours...", Style::default().fg(Color::DarkGray)),
        };

        // Fenêtre de temps
        let win_start = format_time_ms(c.window_start_ms);
        let win_end = format_time_ms(c.window_end_ms);
        let first_tick = if c.first_tick_ms > 0 { format_time_ms(c.first_tick_ms) } else { "---".to_string() };
        let last_tick = if c.last_tick_ms > 0 { format_time_ms(c.last_tick_ms) } else { "---".to_string() };

        // Amplitude en dollars et en %
        let range_usd = c.high - c.low;
        let range_pct = if c.open > 0.0 { (range_usd / c.open) * 100.0 } else { 0.0 };
        let ret_pct = if c.open > 0.0 { ((c.close - c.open) / c.open) * 100.0 } else { 0.0 };

        vec![
            Line::from(vec![
                Span::styled("  Fenêtre : ", Style::default().fg(Color::DarkGray)),
                Span::styled(format!("{} → {}", win_start, win_end), Style::default().fg(Color::White)),
            ]),
            Line::from(vec![
                Span::styled("  Ticks Chainlink: ", Style::default().fg(Color::DarkGray)),
                Span::styled(format!("{}", c.tick_count), Style::default().fg(Color::Cyan)),
                Span::styled(format!("  ({} → {})", first_tick, last_tick), Style::default().fg(Color::DarkGray)),
            ]),
            Line::from(""),
            Line::from(vec![
                Span::styled("  Open : ", Style::default().fg(Color::DarkGray)),
                Span::styled(format!("${:.2}", c.open), Style::default().fg(Color::White)),
                Span::styled("  ← prix de départ (strike)", Style::default().fg(Color::DarkGray)),
            ]),
            Line::from(vec![
                Span::styled("  High : ", Style::default().fg(Color::DarkGray)),
                Span::styled(format!("${:.2}", c.high), Style::default().fg(Color::Green)),
            ]),
            Line::from(vec![
                Span::styled("  Low  : ", Style::default().fg(Color::DarkGray)),
                Span::styled(format!("${:.2}", c.low), Style::default().fg(Color::Red)),
            ]),
            Line::from(vec![
                Span::styled("  Close: ", Style::default().fg(Color::DarkGray)),
                Span::styled(
                    format!("${:.2}", c.close),
                    Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    format!("  ({:+.3}%)", ret_pct),
                    Style::default().fg(if ret_pct >= 0.0 { Color::Green } else { Color::Red }),
                ),
                Span::styled("  ← prix de résolution", Style::default().fg(Color::DarkGray)),
            ]),
            Line::from(vec![
                Span::styled("  Range: ", Style::default().fg(Color::DarkGray)),
                Span::styled(
                    format!("${:.2} ({:.3}%)", range_usd, range_pct),
                    Style::default().fg(Color::White),
                ),
            ]),
            Line::from(""),
            Line::from(vec![
                Span::styled("  → ", Style::default().fg(Color::White)),
                direction_span,
            ]),
        ]
    };

    let title = format!(
        " BTC/USD Chainlink — fenêtre 5min ({} ticks) ",
        if c.is_valid() { c.tick_count.to_string() } else { "0".to_string() }
    );

    let para = Paragraph::new(content)
        .block(
            Block::default()
                .title(title)
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Magenta)),
        );
    f.render_widget(para, area);
}

fn draw_recent_trades(f: &mut Frame, st: &AppState, area: Rect) {
    let rows: Vec<Row> = st.recent_trades.iter().rev().take(20).map(|t| {
        let side_color = if t.side == "BUY" { Color::Green } else { Color::Red };
        let ts = format_time_ms(t.timestamp_ms);
        Row::new(vec![
            Cell::from(ts).style(Style::default().fg(Color::DarkGray)),
            Cell::from(format!("{:.4}", t.price)).style(Style::default().fg(side_color)),
            Cell::from(format!("{:.1}", t.size)).style(Style::default().fg(Color::White)),
            Cell::from(t.side.clone()).style(Style::default().fg(side_color)),
        ])
    }).collect();

    let widths = [
        Constraint::Length(9),
        Constraint::Length(7),
        Constraint::Length(8),
        Constraint::Length(5),
    ];

    let table = Table::new(rows, widths)
        .block(
            Block::default()
                .title(" Trades marché Polymarket (Up/Down) ")
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Cyan)),
        )
        .header(
            Row::new(vec!["Time", "Price", "Size ($)", "Side"])
                .style(Style::default().fg(Color::DarkGray).add_modifier(Modifier::BOLD)),
        );

    f.render_widget(table, area);
}

// ─── FOOTER ───────────────────────────────────────────────────────────────────

fn draw_footer(f: &mut Frame, st: &AppState, area: Rect) {
    let now = now_ms();

    let clob_age = if st.last_ws_clob_ms > 0 {
        (now - st.last_ws_clob_ms) as i64
    } else {
        -1
    };
    let rtds_age = if st.last_ws_rtds_ms > 0 {
        (now - st.last_ws_rtds_ms) as i64
    } else {
        -1
    };

    let clob_status = if clob_age < 0 {
        Span::styled("CLOB: connexion...", Style::default().fg(Color::DarkGray))
    } else if clob_age < 2000 {
        Span::styled(
            format!("CLOB ✓ {}ms", st.clob_latency_ms),
            Style::default().fg(Color::Green),
        )
    } else {
        Span::styled(
            format!("CLOB ⚠ {}ms", clob_age),
            Style::default().fg(Color::Yellow),
        )
    };

    let rtds_status = if rtds_age < 0 {
        Span::styled("RTDS: connexion...", Style::default().fg(Color::DarkGray))
    } else if rtds_age < 3000 {
        Span::styled(
            format!("RTDS ✓ {}ms", st.rtds_latency_ms),
            Style::default().fg(Color::Green),
        )
    } else {
        Span::styled(
            format!("RTDS ⚠ {}ms", rtds_age),
            Style::default().fg(Color::Yellow),
        )
    };

    let chainlink_age = if st.chainlink_timestamp_ms > 0 {
        let age = now.saturating_sub(st.chainlink_timestamp_ms);
        if age < 2000 {
            Span::styled(format!("CL {}ms", age), Style::default().fg(Color::Green))
        } else {
            Span::styled(format!("CL {:.1}s", age as f64 / 1000.0), Style::default().fg(Color::Yellow))
        }
    } else {
        Span::styled("CL ---", Style::default().fg(Color::DarkGray))
    };

    let now_local = Local::now().format("%H:%M:%S%.3f").to_string();

    let footer = Paragraph::new(Line::from(vec![
        Span::raw(" "),
        clob_status,
        Span::raw("  "),
        rtds_status,
        Span::raw("  "),
        chainlink_age,
        Span::raw("  "),
        Span::styled(format!("{}", now_local), Style::default().fg(Color::DarkGray)),
        Span::raw("  "),
        Span::styled("[q] Quitter", Style::default().fg(Color::DarkGray)),
    ]))
    .block(Block::default().borders(Borders::ALL).border_style(Style::default().fg(Color::DarkGray)));

    f.render_widget(footer, area);
}

// ─── Helpers ─────────────────────────────────────────────────────────────────

fn format_time_ms(ts_ms: u64) -> String {
    let secs = (ts_ms / 1000) as i64;
    let dt = DateTime::from_timestamp(secs, 0).unwrap_or_default();
    let local: DateTime<Local> = dt.with_timezone(&Local);
    local.format("%H:%M:%S").to_string()
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
