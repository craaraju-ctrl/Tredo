//! COT Log tab — Live Chain of Thought entries.

use crate::prelude::*;
use crate::AppState;

pub fn render_cot(f: &mut Frame, area: Rect, app: &AppState) {
    let items: Vec<ListItem> = app
        .cot
        .iter()
        .rev()
        .skip(app.scroll_offset)
        .take(area.height as usize - 2)
        .map(|entry| {
            let ts = entry
                .get("timestamp")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let agent = entry.get("agent").and_then(|v| v.as_str()).unwrap_or("?");
            let action = entry.get("action").and_then(|v| v.as_str()).unwrap_or("");
            let reason = entry.get("reason").and_then(|v| v.as_str()).unwrap_or("");

            let color = match agent {
                a if a.contains("Identifier") || a.contains("Market") => Color::Green,
                a if a.contains("Verifier") || a.contains("Risk") => Color::Yellow,
                a if a.contains("Executer") || a.contains("Execution") => Color::Magenta,
                a if a.contains("Guardian") || a.contains("Drawdown") => Color::Red,
                a if a.contains("MetaControl") => Color::Cyan,
                _ => Color::White,
            };

            ListItem::new(Line::from(vec![
                Span::styled(
                    format!("[{}] ", &ts[..ts.len().min(19)]),
                    Style::default().fg(Color::DarkGray),
                ),
                Span::styled(
                    format!("{}: ", agent),
                    Style::default().fg(color).add_modifier(Modifier::BOLD),
                ),
                Span::styled(action, Style::default().fg(Color::White)),
                Span::raw("  "),
                Span::styled(reason, Style::default().fg(Color::Gray)),
            ]))
        })
        .collect();

    let list = List::new(items).block(
        Block::default()
            .title("🔀 Live COT — Chain of Thought")
            .borders(Borders::ALL),
    );
    f.render_widget(list, area);
}
