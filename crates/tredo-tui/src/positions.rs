//! Positions tab — Open positions table with inline trade execution panel.
//!
//! Shows open positions and allows placing paper trades via an inline form
//! (press 't' or 'y' to toggle). Use Up/Down to adjust values, Left/Right
//! to switch fields, Enter to submit, Esc to cancel.

use crate::prelude::*;
use crate::AppState;

/// Number of fields in the trade entry form
const TRADE_FIELDS: usize = 5;

fn position_direction_label(pos: &serde_json::Value) -> &'static str {
    match pos.get("direction").and_then(|v| v.as_str()) {
        Some("Long") | Some("long") | Some("BUY") | Some("buy") => "Long",
        Some("Short") | Some("short") | Some("SELL") | Some("sell") => "Short",
        _ => "Long",
    }
}

fn live_mark_price(app: &AppState, symbol: &str, fallback: f64) -> f64 {
    app.crypto_prices
        .get(symbol)
        .and_then(|p| p.get("price").and_then(|v| v.as_f64()))
        .filter(|p| *p > 0.0)
        .unwrap_or(fallback)
}

pub fn render_positions(f: &mut Frame, area: Rect, app: &AppState) {
    let status = app.status.as_ref();
    let positions: Vec<serde_json::Value> = status
        .and_then(|s| s.get("open_positions"))
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    // ── Layout: positions table + optional trade form ──────────────────────
    let (table_height, form_height) = if app.trade_entry_visible {
        (area.height / 2, area.height / 2)
    } else {
        (area.height, 0)
    };

    let trade_panel_area = if app.trade_entry_visible {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(table_height),
                Constraint::Length(form_height),
            ])
            .split(area);
        Some((chunks[0], chunks[1]))
    } else {
        None
    };

    let table_area = trade_panel_area.map(|(t, _)| t).unwrap_or(area);

    // ── Positions Table ───────────────────────────────────────────────────
    let lines: Vec<Line> = if positions.is_empty() {
        vec![Line::from(Span::styled(
            "No open positions (paper).  Press 't' to open trade entry.",
            Style::default().fg(THEME.muted),
        ))]
    } else {
        let mut lines = vec![
            Line::from(vec![
                Span::styled(
                    "SYMBOL",
                    Style::default()
                        .fg(THEME.brand)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw("  "),
                Span::styled(
                    "DIR",
                    Style::default()
                        .fg(THEME.brand)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw("  "),
                Span::styled(
                    "QTY",
                    Style::default()
                        .fg(THEME.brand)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw("  "),
                Span::styled(
                    "ENTRY",
                    Style::default()
                        .fg(THEME.brand)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw("  "),
                Span::styled(
                    "CURRENT",
                    Style::default()
                        .fg(THEME.brand)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw("  "),
                Span::styled(
                    "P&L",
                    Style::default()
                        .fg(THEME.brand)
                        .add_modifier(Modifier::BOLD),
                ),
            ]),
            Line::from(""),
        ];

        for p in positions.iter() {
            let sym = p.get("symbol").and_then(|v| v.as_str()).unwrap_or("?");
            let dir = position_direction_label(p);
            let qty = p.get("quantity").and_then(|v| v.as_f64()).unwrap_or(0.0);
            let entry = p.get("entry_price").and_then(|v| v.as_f64()).unwrap_or(0.0);
            let stored_current = p
                .get("current_price")
                .and_then(|v| v.as_f64())
                .unwrap_or(entry);
            let current = live_mark_price(app, sym, stored_current);
            let pnl = if dir == "Long" {
                (current - entry) * qty
            } else {
                (entry - current) * qty
            };
            let pnl_color = if pnl >= 0.0 {
                THEME.positive
            } else {
                THEME.negative
            };

            lines.push(Line::from(vec![
                Span::styled(sym.to_string(), Style::default().fg(THEME.highlight)),
                Span::raw("  "),
                Span::styled(
                    dir.to_string(),
                    Style::default().fg(if dir == "Long" {
                        THEME.positive
                    } else {
                        THEME.negative
                    }),
                ),
                Span::raw("  "),
                Span::styled(format!("{:.4}", qty), Style::default().fg(THEME.highlight)),
                Span::raw("  "),
                Span::styled(
                    format!("{:.2}", entry),
                    Style::default().fg(THEME.highlight),
                ),
                Span::raw("  "),
                Span::styled(
                    format!("{:.2}", current),
                    Style::default().fg(THEME.highlight),
                ),
                Span::raw("  "),
                Span::styled(format!("₹{:+.2}", pnl), Style::default().fg(pnl_color)),
            ]));
        }
        lines
    };

    let title_str = if app.trade_entry_visible {
        "Open Positions  |  t: close form"
    } else {
        "Open Positions  |  t: open trade entry"
    };
    let p = Paragraph::new(lines)
        .block(Block::default().title(title_str).borders(Borders::ALL))
        .wrap(Wrap { trim: true });
    f.render_widget(p, table_area);

    // ── Trade Entry Form (inline, editable) ───────────────────────────────
    if let Some((_, form_area)) = trade_panel_area {
        render_trade_form(f, form_area, app);
    }
}

/// Render the inline trade entry form with editable fields.
fn render_trade_form(f: &mut Frame, area: Rect, app: &AppState) {
    let block = Block::default()
        .title(format!(
            "Trade Entry  |  ↑↓ adjust  ←→ field  Enter submit  Esc cancel  [field {}/{}]",
            app.trade_entry_focus + 1,
            TRADE_FIELDS
        ))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(THEME.neutral));
    let inner = block.inner(area);
    f.render_widget(block, area);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // Symbol
            Constraint::Length(1), // Direction
            Constraint::Length(1), // Entry price
            Constraint::Length(1), // Stop loss
            Constraint::Length(1), // Take profit
            Constraint::Min(1),    // Submit hint + price hints
        ])
        .split(inner);

    let fields: [(String, String); TRADE_FIELDS] = [
        ("Symbol".to_string(), app.trade_entry_symbol.clone()),
        ("Direction".to_string(), app.trade_entry_direction.clone()),
        ("Entry".to_string(), format!("{:.2}", app.trade_entry_price)),
        (
            "Stop Loss".to_string(),
            format!("{:.2}", app.trade_entry_sl),
        ),
        (
            "Take Profit".to_string(),
            format!("{:.2}", app.trade_entry_tp),
        ),
    ];

    for (i, (label, value)) in fields.iter().enumerate() {
        let focused = i == app.trade_entry_focus;
        let cursor = if focused { "◀" } else { " " };

        let (fg, label_fg) = if focused {
            (THEME.neutral, THEME.brand)
        } else {
            (THEME.highlight, THEME.muted)
        };

        let line = Line::from(vec![
            Span::raw("  "),
            Span::styled(
                format!("{:<12}", label),
                Style::default().fg(label_fg).add_modifier(if focused {
                    Modifier::BOLD
                } else {
                    Modifier::default()
                }),
            ),
            Span::styled(
                value,
                Style::default().fg(fg).add_modifier(if focused {
                    Modifier::BOLD
                } else {
                    Modifier::default()
                }),
            ),
            Span::raw("  "),
            Span::styled(cursor, Style::default().fg(THEME.brand)),
        ]);
        f.render_widget(Paragraph::new(line), chunks[i]);
    }

    // Build context-sensitive hint based on focused field
    let hint = match app.trade_entry_focus {
        0 => Line::from(vec![
            Span::styled("  ← → switch field  |  ", Style::default().fg(THEME.muted)),
            Span::styled(
                "↑ ↓",
                Style::default()
                    .fg(THEME.neutral)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                " cycle symbol in watchlist  |  ",
                Style::default().fg(THEME.muted),
            ),
            Span::styled(
                "Enter",
                Style::default()
                    .fg(THEME.positive)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(" submit  ", Style::default().fg(THEME.muted)),
            Span::styled("Esc", Style::default().fg(THEME.negative)),
            Span::styled(" cancel", Style::default().fg(THEME.muted)),
        ]),
        1 => Line::from(vec![
            Span::styled("  ← → switch field  |  ", Style::default().fg(THEME.muted)),
            Span::styled(
                "↑ ↓",
                Style::default()
                    .fg(THEME.neutral)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(" toggle Long/Short  |  ", Style::default().fg(THEME.muted)),
            Span::styled(
                "Enter",
                Style::default()
                    .fg(THEME.positive)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(" submit  ", Style::default().fg(THEME.muted)),
            Span::styled("Esc", Style::default().fg(THEME.negative)),
            Span::styled(" cancel", Style::default().fg(THEME.muted)),
        ]),
        2..=4 => {
            let step = if app.trade_entry_price > 1000.0 {
                10.0
            } else if app.trade_entry_price > 100.0 {
                1.0
            } else if app.trade_entry_price > 10.0 {
                0.5
            } else {
                0.05
            };
            Line::from(vec![
                Span::styled("  ← → switch field  |  ", Style::default().fg(THEME.muted)),
                Span::styled(
                    "↑ ↓",
                    Style::default()
                        .fg(THEME.neutral)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    format!(" adjust by {:.2}  |  ", step),
                    Style::default().fg(THEME.muted),
                ),
                Span::styled(
                    "Enter",
                    Style::default()
                        .fg(THEME.positive)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(" submit  ", Style::default().fg(THEME.muted)),
                Span::styled("Esc", Style::default().fg(THEME.negative)),
                Span::styled(" cancel", Style::default().fg(THEME.muted)),
            ])
        }
        _ => Line::from(Span::styled(
            "  Enter to submit, Esc to cancel",
            Style::default().fg(THEME.muted),
        )),
    };

    f.render_widget(Paragraph::new(hint), chunks[5]);
}
