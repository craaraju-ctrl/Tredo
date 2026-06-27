//! Backtest Results Viewer tab — Displays structured backtest results from the orchestrator API.
//!
//! Shows summary stats (trades, win rate, P&L, drawdown, Sharpe) and the full decision log.
//! Polls GET /api/backtest/results for structured JSON data.

use crate::prelude::*;
use crate::AppState;

pub fn render_backtest(f: &mut Frame, area: Rect, app: &AppState) {
    let data = app.backtest_result.as_ref();
    let now = std::time::Instant::now();

    // ── Loading state ───────────────────────────────────────────────────────
    if data.is_none() {
        let spinner = loading_spinner(now);
        let msg = Paragraph::new(vec![
            Line::from(Span::styled(
                format!("{} Loading backtest results...", spinner),
                Style::default()
                    .fg(THEME.neutral)
                    .add_modifier(Modifier::BOLD),
            )),
            Line::from(""),
            Line::from(Span::styled(
                "  Press Enter to run a new backtest, or Poll waits for data.",
                Style::default().fg(THEME.muted),
            )),
        ])
        .block(
            Block::default()
                .title("📊 Backtest Results")
                .borders(Borders::ALL)
                .border_style(Style::default().fg(THEME.border)),
        )
        .wrap(Wrap { trim: true });
        f.render_widget(msg, area);
        return;
    }

    let cache = data.unwrap();

    let trades = cache
        .get("total_trades")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let win_rate = cache
        .get("win_rate")
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0);
    let pnl = cache
        .get("total_pnl")
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0);
    let dd = cache
        .get("max_drawdown")
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0);
    let sharpe = cache
        .get("sharpe_ratio")
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0);
    let decisions: Vec<String> = cache
        .get("decisions")
        .and_then(|v| v.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();

    // ── Layout ──────────────────────────────────────────────────────────────
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(5), // Summary cards
            Constraint::Length(1), // Gap
            Constraint::Min(3),    // Decision log
        ])
        .split(area);

    // ── Summary Cards Row ───────────────────────────────────────────────────
    let pnl_color = if pnl >= 0.0 {
        THEME.positive
    } else {
        THEME.danger
    };
    let wr_color = if win_rate >= 0.55 {
        THEME.positive
    } else if win_rate >= 0.4 {
        THEME.warning
    } else {
        THEME.danger
    };
    let dd_color = if dd <= 0.05 {
        THEME.positive
    } else if dd <= 0.10 {
        THEME.warning
    } else {
        THEME.danger
    };
    let sharpe_color = if sharpe >= 1.0 {
        THEME.positive
    } else if sharpe >= 0.5 {
        THEME.warning
    } else {
        THEME.danger
    };

    let summary_row = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Ratio(1, 4),
            Constraint::Ratio(1, 4),
            Constraint::Ratio(1, 4),
            Constraint::Ratio(1, 4),
        ])
        .split(chunks[0]);

    render_backtest_card(
        f,
        summary_row[0],
        "Trades",
        &format!("{}", trades),
        THEME.info,
    );
    render_backtest_card(
        f,
        summary_row[1],
        "Win Rate",
        &format!("{:.1}%", win_rate * 100.0),
        wr_color,
    );
    render_backtest_card(
        f,
        summary_row[2],
        "Total P&L",
        &format!("₹{:.0}", pnl),
        pnl_color,
    );
    render_backtest_card(
        f,
        summary_row[3],
        "Max DD",
        &format!("{:.1}%", dd * 100.0),
        dd_color,
    );

    // ── Decision Log ────────────────────────────────────────────────────────
    let mut lines: Vec<Line> = Vec::new();

    // Header
    lines.push(Line::from(vec![
        Span::styled("  Sharpe: ", Style::default().fg(THEME.muted)),
        Span::styled(
            format!("{:.2}", sharpe),
            Style::default()
                .fg(sharpe_color)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            "  |  Entries sorted by time",
            Style::default().fg(THEME.muted),
        ),
    ]));
    lines.push(Line::from(""));

    for decision in &decisions {
        let is_win = decision.contains("WIN");
        let is_loss = decision.contains("LOSS");
        let is_entry = decision.contains("BUY") || decision.contains("SELL");
        let color = if is_win {
            THEME.positive
        } else if is_loss {
            THEME.danger
        } else if is_entry {
            THEME.warning
        } else {
            THEME.muted
        };
        lines.push(Line::from(Span::styled(
            format!("  {}", decision),
            Style::default().fg(color),
        )));
    }

    if lines.len() <= 2 {
        lines.push(Line::from(Span::styled(
            "  No decisions recorded in this backtest run.",
            Style::default().fg(THEME.muted),
        )));
    }

    let para = Paragraph::new(lines)
        .block(
            Block::default()
                .title("📋 Decision Log")
                .borders(Borders::ALL)
                .border_style(Style::default().fg(THEME.border)),
        )
        .wrap(Wrap { trim: true });
    f.render_widget(para, chunks[2]);
}

/// Render a backtest summary card with label and value.
fn render_backtest_card(f: &mut Frame, area: Rect, label: &str, value: &str, accent: Color) {
    let block = Block::default()
        .title(Span::styled(
            label,
            Style::default().fg(accent).add_modifier(Modifier::BOLD),
        ))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(THEME.border));
    let inner = block.inner(area);
    f.render_widget(block, area);

    let val = Paragraph::new(Line::from(Span::styled(
        value,
        Style::default()
            .fg(THEME.highlight)
            .add_modifier(Modifier::BOLD),
    )))
    .alignment(Alignment::Center);
    f.render_widget(val, inner);
}
