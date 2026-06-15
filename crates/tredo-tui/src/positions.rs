//! Positions tab — Open positions table.

use crate::prelude::*;
use crate::AppState;

pub fn render_positions(f: &mut Frame, area: Rect, app: &AppState) {
    let status = app.status.as_ref();
    let positions = status
        .and_then(|s| s.get("open_positions"))
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    let lines: Vec<Line> = if positions.is_empty() {
        vec![Line::from(Span::styled(
            "No open positions (paper).",
            Style::default().fg(Color::DarkGray),
        ))]
    } else {
        let mut lines = vec![
            Line::from(vec![
                Span::styled(
                    "SYMBOL",
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw("  "),
                Span::styled(
                    "DIR",
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw("  "),
                Span::styled(
                    "ENTRY",
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw("  "),
                Span::styled(
                    "CURRENT",
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw("  "),
                Span::styled(
                    "P&L",
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                ),
            ]),
            Line::from(""),
        ];

        for p in positions.iter() {
            let sym = p.get("symbol").and_then(|v| v.as_str()).unwrap_or("?");
            let dir = p.get("direction").and_then(|v| v.as_str()).unwrap_or("?");
            let entry = p.get("entry_price").and_then(|v| v.as_f64()).unwrap_or(0.0);
            let current = p
                .get("current_price")
                .and_then(|v| v.as_f64())
                .unwrap_or(entry);
            let pnl_pct = if entry > 0.0 {
                (current - entry) / entry * 100.0
            } else {
                0.0
            };
            let pnl_color = if pnl_pct >= 0.0 {
                Color::Green
            } else {
                Color::Red
            };

            lines.push(Line::from(vec![
                Span::styled(sym.to_string(), Style::default().fg(Color::White)),
                Span::raw("  "),
                Span::styled(
                    dir.to_string(),
                    Style::default().fg(if dir == "Long" {
                        Color::Green
                    } else {
                        Color::Red
                    }),
                ),
                Span::raw("  "),
                Span::styled(format!("{:.2}", entry), Style::default().fg(Color::White)),
                Span::raw("  "),
                Span::styled(format!("{:.2}", current), Style::default().fg(Color::White)),
                Span::raw("  "),
                Span::styled(format!("{:+.2}%", pnl_pct), Style::default().fg(pnl_color)),
            ]));
        }
        lines
    };

    let p = Paragraph::new(lines)
        .block(
            Block::default()
                .title("📋 Open Positions")
                .borders(Borders::ALL),
        )
        .wrap(Wrap { trim: true });
    f.render_widget(p, area);
}
