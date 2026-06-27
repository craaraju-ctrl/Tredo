//! Performance Analytics tab — Equity curve, P&L over time, win rate trend.
//!
//! Uses data from the policy cache API (pnl_history, equity_history, confidence_history)
//! to render performance charts and summary statistics.

use crate::prelude::*;
use crate::AppState;

pub fn render_performance(f: &mut Frame, area: Rect, app: &AppState) {
    let cache = app.policy_cache.as_ref();

    let eq_history: Vec<f64> = cache
        .and_then(|c| c.get("equity_history"))
        .and_then(|h| h.as_array())
        .map(|a| a.iter().filter_map(|v| v.as_f64()).collect::<Vec<f64>>())
        .filter(|h: &Vec<f64>| !h.is_empty())
        .unwrap_or_else(|| app.equity_history.clone());
    let pnl_history: Vec<f64> = cache
        .and_then(|c| c.get("pnl_history"))
        .and_then(|h| h.as_array())
        .map(|a| a.iter().filter_map(|v| v.as_f64()).collect::<Vec<f64>>())
        .filter(|h: &Vec<f64>| !h.is_empty())
        .unwrap_or_else(|| app.pnl_history.clone());
    let conf_history: Vec<f64> = cache
        .and_then(|c| c.get("confidence_history"))
        .and_then(|h| h.as_array())
        .map(|a| a.iter().filter_map(|v| v.as_f64()).collect())
        .unwrap_or_default();

    let hit_history: Vec<f64> = cache
        .and_then(|c| c.get("hit_rate_history"))
        .and_then(|h| h.as_array())
        .map(|a| a.iter().filter_map(|v| v.as_f64()).collect())
        .unwrap_or_default();

    // Layout: top row summary cards, middle equity sparkline, bottom charts
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(4), // Summary cards
            Constraint::Length(4), // Equity curve sparkline (large)
            Constraint::Min(3),    // P&L + Conf sparklines side by side
        ])
        .split(area);

    // ── Summary Cards ──────────────────────────────────────────────────────
    let eq_avg = eq_history.iter().copied().sum::<f64>() / eq_history.len().max(1) as f64;
    let _eq_min = eq_history.iter().copied().fold(f64::INFINITY, f64::min);
    let _eq_max = eq_history.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    let eq_start = eq_history.first().copied().unwrap_or(0.0);
    let eq_return = if eq_start > 0.0 {
        ((eq_avg - eq_start) / eq_start) * 100.0
    } else {
        0.0
    };

    let pnl_total: f64 = pnl_history.iter().sum();
    let conf_avg = conf_history.iter().copied().sum::<f64>() / conf_history.len().max(1) as f64;
    let _hit_avg = hit_history.iter().copied().sum::<f64>() / hit_history.len().max(1) as f64;

    let summary_row = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Ratio(1, 4),
            Constraint::Ratio(1, 4),
            Constraint::Ratio(1, 4),
            Constraint::Ratio(1, 4),
        ])
        .split(chunks[0]);

    render_summary_card(
        f,
        summary_row[0],
        "Avg Equity",
        &format!("₹{:.0}", eq_avg),
        THEME.info,
    );
    render_summary_card(
        f,
        summary_row[1],
        "Return",
        &format!("{:+.1}%", eq_return),
        if eq_return >= 0.0 {
            THEME.positive
        } else {
            THEME.danger
        },
    );
    render_summary_card(
        f,
        summary_row[2],
        "Total P&L",
        &format!("₹{:.0}", pnl_total),
        if pnl_total >= 0.0 {
            THEME.positive
        } else {
            THEME.danger
        },
    );
    render_summary_card(
        f,
        summary_row[3],
        "Avg Confidence",
        &format!("{:.1}%", conf_avg * 100.0),
        THEME.neutral,
    );

    // ── Equity Curve Sparkline (full width) ────────────────────────────────
    render_large_sparkline(f, chunks[1], "Equity Curve", &eq_history, THEME.info, false);

    // ── Bottom: P&L sparkline + Confidence/Hit Rate sparkline ──────────────
    let bottom_row = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Ratio(1, 2), Constraint::Ratio(1, 2)])
        .split(chunks[2]);

    let conf_label = if conf_history.len() >= 2 {
        "Avg Confidence Trend"
    } else {
        "Confidence History"
    };
    let hit_label = if hit_history.len() >= 2 {
        "Avg Hit Rate Trend"
    } else {
        "Hit Rate History"
    };

    render_large_sparkline(
        f,
        bottom_row[0],
        "P&L Trend",
        &pnl_history,
        THEME.positive,
        false,
    );

    // Right side: show confidence if available, otherwise hit rate
    if conf_history.len() >= 2 {
        render_large_sparkline(
            f,
            bottom_row[1],
            conf_label,
            &conf_history,
            THEME.neutral,
            true,
        );
    } else {
        render_large_sparkline(
            f,
            bottom_row[1],
            hit_label,
            &hit_history,
            THEME.neutral,
            true,
        );
    }
}

/// Render a simple summary card with a label and value.
fn render_summary_card(f: &mut Frame, area: Rect, label: &str, value: &str, accent: Color) {
    let block = Block::default()
        .title(Span::styled(
            label,
            Style::default().fg(accent).add_modifier(Modifier::BOLD),
        ))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(THEME.border));
    let inner = block.inner(area);
    f.render_widget(block, area);

    let val_para = Paragraph::new(Line::from(Span::styled(
        value,
        Style::default()
            .fg(THEME.highlight)
            .add_modifier(Modifier::BOLD),
    )))
    .alignment(Alignment::Center);
    f.render_widget(val_para, inner);
}

/// Render a large sparkline with Y-axis labels inside a bordered block.
fn render_large_sparkline(
    f: &mut Frame,
    area: Rect,
    label: &str,
    history: &[f64],
    accent: Color,
    as_percentage: bool,
) {
    let block = Block::default()
        .title(Span::styled(label, Style::default().fg(accent)))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(THEME.border));
    let inner = block.inner(area);
    f.render_widget(block, area);

    if history.len() < 2 {
        let para = Paragraph::new(Line::from(Span::styled(
            "waiting for data...",
            Style::default().fg(THEME.muted),
        )));
        f.render_widget(para, inner);
        return;
    }

    let min_val = history.iter().copied().fold(f64::INFINITY, f64::min);
    let max_val = history.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    let range = (max_val - min_val).max(0.001);
    let avg = history.iter().sum::<f64>() / history.len() as f64;

    let bars: [&str; 8] = ["▁", "▂", "▃", "▄", "▅", "▆", "▇", "█"];
    let sparkline: String = history
        .iter()
        .map(|v| {
            let idx = ((v - min_val) / range * 7.0).round().clamp(0.0, 7.0) as usize;
            bars[idx]
        })
        .collect();

    let (max_lbl, min_lbl, avg_lbl) = if as_percentage {
        (
            format!("{:.0}%", max_val * 100.0),
            format!("{:.0}%", min_val * 100.0),
            format!("avg {:.0}%", avg * 100.0),
        )
    } else {
        (
            format!("{:+.1}", max_val),
            format!("{:+.1}", min_val),
            format!("avg {:+.1}", avg),
        )
    };

    // Split inner into top row (max + sparkline + avg) and bottom row (min)
    let inner_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Length(1)])
        .split(inner);

    let top_line = Line::from(vec![
        Span::styled(
            &max_lbl,
            Style::default().fg(accent).add_modifier(Modifier::BOLD),
        ),
        Span::styled("  ", Style::default().fg(THEME.muted)),
        Span::styled(
            &sparkline,
            Style::default().fg(accent).add_modifier(Modifier::BOLD),
        ),
        Span::styled(format!("  {}", avg_lbl), Style::default().fg(THEME.muted)),
    ]);
    f.render_widget(Paragraph::new(top_line), inner_chunks[0]);

    let bot_line = Line::from(Span::styled(&min_lbl, Style::default().fg(accent)));
    f.render_widget(Paragraph::new(bot_line), inner_chunks[1]);
}
