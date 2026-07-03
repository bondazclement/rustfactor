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
        .constraints([Constraint::Length(3), Constraint::Min(10), Constraint::Length(4)])
        .split(area);
    draw_header(f, &st, chunks[0]);
    draw_body(f, &st, chunks[1]);
    draw_footer(f, &st, chunks[2]);
}

fn draw_header(f: &mut Frame, st: &AppState, area: Rect) {
    let now = now_ms();
    let (title, countdown, slug) = if let Some(ref m) = st.market {
        let rem = if m.end_date_ms > now {
            (m.end_date_ms - now) / 1000
        } else {
            0
        };
        (
            m.title.clone(),
            format!("{:02}:{:02}", rem / 60, rem % 60),
            m.slug.clone(),
        )
    } else {
        ("Recherche du marché...".to_string(), "--:--".to_string(), "n/a".to_string())
    };
    let cl = st
        .btc_price_chainlink
        .map(|p| format!("${:.2}", p))
        .unwrap_or_else(|| "---".to_string());
    let link = if slug != "n/a" {
        format!("https://polymarket.com/event/{}", slug)
    } else {
        "n/a".to_string()
    };

    let line = Line::from(vec![
        Span::styled(" Rustector_btc_5mn ", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
        Span::styled(format!("| {} [{}] ", title, countdown), Style::default().fg(Color::Yellow)),
        Span::styled(format!("| Chainlink {} ", cl), Style::default().fg(Color::Green)),
        Span::styled(format!("| {}", link), Style::default().fg(Color::DarkGray)),
    ]);
    let p = Paragraph::new(line)
        .block(Block::default().borders(Borders::ALL))
        .alignment(Alignment::Left);
    f.render_widget(p, area);
}

fn draw_body(f: &mut Frame, st: &AppState, area: Rect) {
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(30), Constraint::Percentage(30), Constraint::Percentage(40)])
        .split(area);
    draw_orderbook(f, st, cols[0], true);
    draw_orderbook(f, st, cols[1], false);
    draw_right(f, st, cols[2]);
}

fn draw_orderbook(f: &mut Frame, st: &AppState, area: Rect, is_up: bool) {
    let (label, color, book) = if is_up {
        ("UP ▲", Color::Green, &st.book_up)
    } else {
        ("DOWN ▼", Color::Red, &st.book_down)
    };
    let title = format!(
        " {} | Bid:{} Ask:{} ",
        label,
        book.best_bid().map(|p| format!("{:.3}", p)).unwrap_or("---".to_string()),
        book.best_ask().map(|p| format!("{:.3}", p)).unwrap_or("---".to_string()),
    );
    let max_levels = ((area.height.saturating_sub(4)) / 2).max(3) as usize;
    let asks: Vec<_> = book.asks.iter().take(max_levels).collect();
    let bids: Vec<_> = book.bids.iter().rev().take(max_levels).collect();
    let mut rows = Vec::new();
    for (p, s) in asks.iter().rev() {
        rows.push(Row::new(vec![
            Cell::from(format!("{:.3}", p.0)).style(Style::default().fg(Color::Red)),
            Cell::from(format!("{:.1}", s)).style(Style::default().fg(Color::DarkGray)),
            Cell::from("SELL").style(Style::default().fg(Color::Red)),
        ]));
    }
    if let Some(m) = book.mid() {
        rows.push(Row::new(vec![Cell::from(format!("──{:.3}──", m)), Cell::from(""), Cell::from("mid")]));
    }
    for (p, s) in &bids {
        rows.push(Row::new(vec![
            Cell::from(format!("{:.3}", p.0)).style(Style::default().fg(Color::Green)),
            Cell::from(format!("{:.1}", s)).style(Style::default().fg(Color::DarkGray)),
            Cell::from("BUY").style(Style::default().fg(Color::Green)),
        ]));
    }
    let table = Table::new(rows, [Constraint::Length(9), Constraint::Length(9), Constraint::Length(5)])
        .block(Block::default().title(title).borders(Borders::ALL).border_style(Style::default().fg(color)))
        .header(Row::new(vec!["Price", "Size", "Side"]).style(Style::default().add_modifier(Modifier::BOLD)));
    f.render_widget(table, area);
}

fn draw_right(f: &mut Frame, st: &AppState, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(11), Constraint::Min(5)])
        .split(area);
    let c = &st.chainlink_candle;
    let candle = if c.is_valid() {
        vec![
            Line::from(format!("Open  ${:.2}", c.open)),
            Line::from(format!("High  ${:.2}", c.high)),
            Line::from(format!("Low   ${:.2}", c.low)),
            Line::from(format!("Close ${:.2}", c.close)),
            Line::from(format!("Ticks {}", c.tick_count)),
            Line::from(format!("RTDS ticks window: {}", st.rtds_tick_count_window)),
            Line::from(format!("CLOB events window: {}", st.clob_event_count_window)),
        ]
    } else {
        vec![Line::from("Waiting Chainlink ticks...")]
    };
    let p = Paragraph::new(candle)
        .block(Block::default().title("Chainlink candle + counters").borders(Borders::ALL));
    f.render_widget(p, chunks[0]);

    let rows: Vec<Row> = st
        .recent_trades
        .iter()
        .rev()
        .take(12)
        .map(|t| {
            Row::new(vec![
                Cell::from(format_time_ms(t.timestamp_ms)),
                Cell::from(format!("{:.4}", t.price)),
                Cell::from(format!("{:.1}", t.size)),
                Cell::from(t.side.clone()),
            ])
        })
        .collect();
    let t = Table::new(rows, [Constraint::Length(9), Constraint::Length(8), Constraint::Length(8), Constraint::Length(5)])
        .block(Block::default().title("Recent trades").borders(Borders::ALL))
        .header(Row::new(vec!["Time", "Price", "Size", "Side"]).style(Style::default().add_modifier(Modifier::BOLD)));
    f.render_widget(t, chunks[1]);
}

fn draw_footer(f: &mut Frame, st: &AppState, area: Rect) {
    let now = now_ms();
    let rec_status = if !st.recording_enabled {
        "REC OFF".to_string()
    } else if st.last_record_write_ms > 0 && now.saturating_sub(st.last_record_write_ms) < 2000 {
        format!("REC ON lines={} bytes={}", st.recording_lines, st.recording_bytes)
    } else {
        format!("REC idle lines={} bytes={}", st.recording_lines, st.recording_bytes)
    };
    let line = Line::from(vec![
        Span::styled(
            format!("CLOB lag={}ms  RTDS lag={}ms  ", st.clob_latency_ms, st.rtds_latency_ms),
            Style::default().fg(Color::White),
        ),
        Span::styled(rec_status, Style::default().fg(Color::Magenta)),
        Span::raw("  "),
        Span::styled(st.recording_file.clone(), Style::default().fg(Color::DarkGray)),
        Span::raw("  "),
        Span::styled("[q] quit", Style::default().fg(Color::DarkGray)),
    ]);
    let p = Paragraph::new(line).block(Block::default().borders(Borders::ALL));
    f.render_widget(p, area);
}

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
