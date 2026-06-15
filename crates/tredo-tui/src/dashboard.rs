//! Dashboard tab — Card-based layout with Gauge progress bars.

use crate::prelude::*;
use crate::AppState;

pub fn render_dashboard(f: &mut Frame, area: Rect, app: &AppState) {
    let status = app.status.as_ref();

    let equity = status
        .and_then(|s| s.get("total_equity"))
        .and_then(|v| v.as_f64())
        .unwrap_or(100_000.0);
    let cash = status
        .and_then(|s| s.get("cash_balance"))
        .and_then(|v| v.as_f64())
        .unwrap_or(100_000.0);
    let pnl = status
        .and_then(|s| s.get("daily_pnl"))
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0);
    let trades = status
        .and_then(|s| s.get("total_trades_today"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let wins = status
        .and_then(|s| s.get("winning_trades_today"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let losses = status
        .and_then(|s| s.get("losing_trades_today"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0);

    let pnl_color = if pnl >= 0.0 { Color::Green } else { Color::Red };
    let win_rate = if trades > 0 {
        wins as f64 / trades as f64 * 100.0
    } else {
        0.0
    };
    let equity_used_pct =
        if equity > 0.0 {
            ((equity - cash) / equity * 100.0).min(100.0) as u16
        } else {
            0
        };
    let cash_pct = if equity > 0.0 {
        (cash / equity * 100.0) as u16
    } else {
        100
    };

    // ── Layout ────────────────────────────────────────────────────────────
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // Title
            Constraint::Length(1), // gap
            Constraint::Length(5), // Top row: EQUITY + CASH
            Constraint::Length(1), // gap
            Constraint::Length(5), // Bottom row: P&L + WIN RATE
            Constraint::Length(1), // gap
            Constraint::Length(3), // Stats bar
        ])
        .split(area);

    // ── Title ─────────────────────────────────────────────────────────────
    let title = Paragraph::new(Line::from(Span::styled(
        "📊 PORTFOLIO DASHBOARD",
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD),
    )));
    f.render_widget(title, chunks[0]);

    // ── Top row: EQUITY + CASH ────────────────────────────────────────────
    let top_row = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Ratio(1, 2), Constraint::Ratio(1, 2)])
        .split(chunks[2]);

    render_metric_card(
        f,
        top_row[0],
        "EQUITY",
        &format!("₹{:>10.2}", equity),
        equity_used_pct,
        Color::Cyan,
        "used",
    );
    render_metric_card(
        f,
        top_row[1],
        "CASH",
        &format!("₹{:>10.2}", cash),
        cash_pct,
        Color::Green,
        "free",
    );

    // ── Bottom row: P&L + WIN RATE ────────────────────────────────────────
    let bottom_row = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Ratio(1, 2), Constraint::Ratio(1, 2)])
        .split(chunks[4]);

    let pnl_pct = if equity > 0.0 {
        ((pnl / equity) * 100.0) as u16
    } else {
        0
    };
    render_metric_card(
        f,
        bottom_row[0],
        "DAILY P&L",
        &format!("₹{:>+9.2}", pnl),
        pnl_pct,
        pnl_color,
        "today",
    );
    render_metric_card(
        f,
        bottom_row[1],
        "WIN RATE",
        &format!("{:>7.1}%", win_rate),
        win_rate as u16,
        if win_rate >= 50.0 {
            Color::Green
        } else if win_rate > 0.0 {
            Color::Yellow
        } else {
            Color::DarkGray
        },
        &format!("{}/{} wins", wins, trades),
    );

    // ── Stats bar ─────────────────────────────────────────────────────────
    let stats_text = format!(
        "  TRADES: {}  │  WINS: {}  │  LOSSES: {}  │  EQUITY USED: {}%",
        trades, wins, losses, equity_used_pct
    );
    let stats = Paragraph::new(Line::from(Span::styled(
        stats_text,
        Style::default().fg(Color::White),
    )))
    .block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::DarkGray)),
    );
    f.render_widget(stats, chunks[6]);
}

/// Render a single metric card with a bordered block, value display, and Gauge progress bar.
fn render_metric_card(
    f: &mut Frame,
    area: Rect,
    title: &str,
    value: &str,
    percent: u16,
    accent: Color,
    subtitle: &str,
) {
    let block = Block::default()
        .title(Span::styled(
            title,
            Style::default()
                .fg(accent)
                .add_modifier(Modifier::BOLD),
        ))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(accent));

    let inner = block.inner(area);
    f.render_widget(block, area);

    // Split card inner into value row + gauge row + subtitle row
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2), // value
            Constraint::Min(1),    // gauge
            Constraint::Length(1), // subtitle
        ])
        .split(inner);

    // ── Value ──
    let val_para = Paragraph::new(Line::from(Span::styled(
        value,
        Style::default()
            .fg(Color::White)
            .add_modifier(Modifier::BOLD),
    )))
    .alignment(Alignment::Center);
    f.render_widget(val_para, chunks[0]);

    // ── Gauge bar ──
    let gauge_color = match percent {
        0..=30 => Color::Green,
        31..=70 => Color::Yellow,
        _ => Color::Red,
    };
    let clamped = percent.min(100).max(0);

    let gauge = Gauge::default()
        .gauge_style(Style::default().fg(gauge_color).bg(Color::DarkGray))
        .label(format!("{}%", clamped))
        .percent(clamped);
    f.render_widget(gauge, chunks[1]);

    // ── Subtitle ──
    let sub_para = Paragraph::new(Line::from(Span::styled(
        subtitle,
        Style::default().fg(Color::DarkGray),
    )))
    .alignment(Alignment::Center);
    f.render_widget(sub_para, chunks[2]);
}
