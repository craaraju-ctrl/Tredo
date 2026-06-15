//! LLM Models tab — View and switch between available Ollama models.

use crate::prelude::*;
use crate::AppState;

pub fn render_models(f: &mut Frame, area: Rect, app: &AppState) {
    let current = app.current_model.as_deref().unwrap_or("unknown");

    let header_text = vec![
        Line::from(vec![Span::styled(
            "🤖 LLM Model Selection",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )]),
        Line::from(""),
        Line::from(vec![
            Span::styled("Current: ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                current,
                Style::default()
                    .fg(Color::Green)
                    .add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(""),
        Line::from(Span::styled(
            "Use ↑/↓ to select, Enter to switch model",
            Style::default().fg(Color::DarkGray),
        )),
    ];

    let header = Paragraph::new(header_text)
        .block(Block::default().borders(Borders::BOTTOM))
        .wrap(Wrap { trim: true });
    f.render_widget(
        header,
        Rect {
            x: area.x,
            y: area.y,
            width: area.width,
            height: 6,
        },
    );

    let list_area = Rect {
        x: area.x,
        y: area.y + 6,
        width: area.width,
        height: area.height.saturating_sub(7),
    };

    let items: Vec<ListItem> = app
        .models
        .iter()
        .map(|m| {
            let name = m.get("name").and_then(|n| n.as_str()).unwrap_or("unknown");
            let size = m.get("size").and_then(|s| s.as_str()).unwrap_or("-");
            let is_selected = app
                .models
                .get(app.selected_model_index)
                .and_then(|s| s.get("name"))
                .and_then(|n| n.as_str())
                .map(|n| n == name)
                .unwrap_or(false);

            let prefix = if is_selected { "👉 " } else { "  " };
            let style = if is_selected {
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::White)
            };

            ListItem::new(Line::from(vec![
                Span::styled(prefix, style),
                Span::styled(name, style),
                Span::raw("  "),
                Span::styled(
                    format!("({})", size),
                    Style::default().fg(Color::DarkGray),
                ),
            ]))
        })
        .collect();

    let list = List::new(items).block(
        Block::default()
            .title("Available Models")
            .borders(Borders::ALL),
    );
    f.render_widget(list, list_area);
}
