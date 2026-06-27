//! LLM Models tab — Rich model selection + live agent communication feed.
//!
//! Layout (split-pane):
//!   Left  (40%) — Available model list with status badges + selection indicator
//!   Right (60%) — Live inter-agent communication stream fed from WS COT events

use crate::prelude::*;
use crate::AppState;

/// Agent color coding consistent with tree.rs
fn agent_color(name: &str) -> Color {
    if name.contains("Tredo") || name.contains("Meta") {
        Color::Cyan
    } else if name.contains("Identifier") {
        Color::Green
    } else if name.contains("Verifier") {
        Color::Yellow
    } else if name.contains("Executer") {
        Color::Magenta
    } else if name.contains("Guardian") {
        Color::Red
    } else if name.contains("Ollama") {
        Color::Blue
    } else if name.contains("Kronos") {
        Color::LightBlue
    } else if name == "System" {
        Color::DarkGray
    } else {
        Color::White
    }
}

/// Agent short icon
fn agent_icon(name: &str) -> &'static str {
    if name.contains("Tredo") || name.contains("Meta") {
        "🤖"
    } else if name.contains("Identifier") {
        "🔍"
    } else if name.contains("Verifier") {
        "✅"
    } else if name.contains("Executer") {
        "⚡"
    } else if name.contains("Guardian") {
        "🛡"
    } else if name.contains("Ollama") {
        "🧠"
    } else if name.contains("Kronos") {
        "⏳"
    } else {
        "📡"
    }
}

/// Action color for comm messages
fn action_color(msg: &str) -> Color {
    let upper = msg.to_uppercase();
    if upper.contains("PASS")
        || upper.contains("BUY")
        || upper.contains("ANALYZED")
        || upper.contains("OK")
    {
        Color::Green
    } else if upper.contains("FAIL")
        || upper.contains("HALT")
        || upper.contains("ABORT")
        || upper.contains("REJECT")
    {
        Color::Red
    } else if upper.contains("HOLD") || upper.contains("SKIP") || upper.contains("WAIT") {
        Color::Yellow
    } else if upper.contains("START") || upper.contains("UPDATE") || upper.contains("SWITCH") {
        Color::Cyan
    } else {
        Color::White
    }
}

pub fn render_models(f: &mut Frame, area: Rect, app: &AppState) {
    let current = app.current_model.as_deref().unwrap_or("unknown");

    // ── Split: 38% model list | 62% live comm ──────────────────────────────
    let split = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(38), Constraint::Percentage(62)])
        .split(area);

    render_model_list(f, split[0], app, current);
    render_live_comm(f, split[1], app);
}

// ── Left Panel: Model List ─────────────────────────────────────────────────

fn render_model_list(f: &mut Frame, area: Rect, app: &AppState, current: &str) {
    // Header strip above model list
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(5), // header block
            Constraint::Min(3),    // scrollable model list
        ])
        .split(area);

    // ── Header ──
    let ws_dot = if app.ws_connected { "🟢" } else { "🔴" };
    let header_lines = vec![
        Line::from(vec![Span::styled(
            "🤖 LLM Model Selection",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )]),
        Line::from(""),
        Line::from(vec![
            Span::styled("Active: ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                current,
                Style::default()
                    .fg(Color::Green)
                    .add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(vec![
            Span::styled(ws_dot, Style::default()),
            Span::styled(
                if app.ws_connected {
                    " WS Connected"
                } else {
                    " WS Disconnected"
                },
                Style::default().fg(if app.ws_connected {
                    Color::Green
                } else {
                    Color::Red
                }),
            ),
            Span::styled(
                format!("  |  {} COT events", app.ws_cot_count),
                Style::default().fg(Color::DarkGray),
            ),
        ]),
    ];

    let header = Paragraph::new(header_lines)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(THEME.border))
                .title(Span::styled(
                    " 🤖 Models ",
                    Style::default()
                        .fg(THEME.brand)
                        .add_modifier(Modifier::BOLD),
                )),
        )
        .wrap(Wrap { trim: true });
    f.render_widget(header, chunks[0]);

    // ── Model List ──
    let items: Vec<ListItem> = if app.models.is_empty() {
        vec![ListItem::new(Line::from(Span::styled(
            "  No models found. Is Ollama running?",
            Style::default().fg(Color::DarkGray),
        )))]
    } else {
        app.models
            .iter()
            .enumerate()
            .map(|(idx, m)| {
                let name = m.get("name").and_then(|n| n.as_str()).unwrap_or("unknown");
                let size = m.get("size").and_then(|s| s.as_str()).unwrap_or("-");
                let is_cursor = idx == app.selected_model_index;
                let is_active = name == current;

                let prefix = if is_cursor { "▶ " } else { "  " };
                let active_badge = if is_active { " ★" } else { "" };

                let (row_fg, row_mod) = if is_cursor {
                    (Color::Yellow, Modifier::BOLD)
                } else if is_active {
                    (Color::Green, Modifier::empty())
                } else {
                    (Color::White, Modifier::empty())
                };

                ListItem::new(Line::from(vec![
                    Span::styled(prefix, Style::default().fg(row_fg).add_modifier(row_mod)),
                    Span::styled(name, Style::default().fg(row_fg).add_modifier(row_mod)),
                    Span::styled(
                        active_badge,
                        Style::default()
                            .fg(Color::Green)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(
                        format!("  ({})", size),
                        Style::default().fg(Color::DarkGray),
                    ),
                ]))
            })
            .collect()
    };

    let list = List::new(items)
        .block(
            Block::default()
                .title(Span::styled(
                    " Available Models ",
                    Style::default().fg(Color::DarkGray),
                ))
                .borders(Borders::ALL)
                .border_style(Style::default().fg(THEME.border)),
        )
        .highlight_style(Style::default().fg(Color::Yellow));
    f.render_widget(list, chunks[1]);
}

// ── Right Panel: Live Inter-Agent Communication Feed ──────────────────────

fn render_live_comm(f: &mut Frame, area: Rect, app: &AppState) {
    // Header bar + scrollable message list
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // legend/filter bar
            Constraint::Min(3),    // messages
        ])
        .split(area);

    // ── Legend bar ──
    let agent_list_str = vec![
        ("🔍 Identifier", Color::Green),
        ("✅ Verifier", Color::Yellow),
        ("⚡ Executer", Color::Magenta),
        ("🛡 Guardian", Color::Red),
    ];
    let mut legend_spans = vec![Span::styled(
        "  Agents: ",
        Style::default().fg(Color::DarkGray),
    )];
    for (label, color) in &agent_list_str {
        legend_spans.push(Span::styled(*label, Style::default().fg(*color)));
        legend_spans.push(Span::styled("  ", Style::default()));
    }
    let legend_line = Line::from(legend_spans);

    let comm_count = app.live_comm_log.len();
    let subtitle = Line::from(vec![Span::styled(
        format!(
            "  {} messages in buffer  |  ↑↓ to scroll  |  Live WS feed",
            comm_count
        ),
        Style::default().fg(Color::DarkGray),
    )]);

    let legend_para = Paragraph::new(vec![legend_line, subtitle]).block(
        Block::default()
            .borders(Borders::TOP | Borders::LEFT | Borders::RIGHT)
            .border_style(Style::default().fg(THEME.border)),
    );
    f.render_widget(legend_para, chunks[0]);

    // ── Messages ──
    let max_visible = chunks[1].height.saturating_sub(2) as usize;

    if app.live_comm_log.is_empty() {
        let waiting = Paragraph::new(vec![
            Line::from(""),
            Line::from(Span::styled(
                "  ⏳ Waiting for agent communications...",
                Style::default()
                    .fg(Color::DarkGray)
                    .add_modifier(Modifier::ITALIC),
            )),
            Line::from(""),
            Line::from(Span::styled(
                "  Run a pipeline cycle to see real-time agent messages here.",
                Style::default().fg(Color::DarkGray),
            )),
            Line::from(""),
            Line::from(Span::styled(
                "  Each agent's chain-of-thought is captured as it flows through:",
                Style::default().fg(Color::DarkGray),
            )),
            Line::from(vec![
                Span::styled("  Tredo", Style::default().fg(Color::Cyan)),
                Span::styled(" → ", Style::default().fg(Color::DarkGray)),
                Span::styled("Identifier", Style::default().fg(Color::Green)),
                Span::styled(" → ", Style::default().fg(Color::DarkGray)),
                Span::styled("Verifier", Style::default().fg(Color::Yellow)),
                Span::styled(" → ", Style::default().fg(Color::DarkGray)),
                Span::styled("Executer", Style::default().fg(Color::Magenta)),
                Span::styled(" → ", Style::default().fg(Color::DarkGray)),
                Span::styled("Guardian", Style::default().fg(Color::Red)),
            ]),
        ])
        .block(
            Block::default()
                .title(Span::styled(
                    " 📡 Live Agent Communications ",
                    Style::default()
                        .fg(THEME.brand)
                        .add_modifier(Modifier::BOLD),
                ))
                .borders(Borders::ALL)
                .border_style(Style::default().fg(THEME.border)),
        )
        .wrap(Wrap { trim: true });
        f.render_widget(waiting, chunks[1]);
        return;
    }

    // Build message lines from the live_comm_log ring-buffer (newest at bottom)
    let all_msgs: Vec<(String, String, String, std::time::Instant)> =
        app.live_comm_log.iter().cloned().collect();

    let mut lines: Vec<Line> = Vec::new();

    for (from, to, msg, ts) in &all_msgs {
        let elapsed = ts.elapsed().as_secs();
        let ts_str = if elapsed < 60 {
            format!("{}s", elapsed)
        } else {
            format!("{}m{}s", elapsed / 60, elapsed % 60)
        };

        let from_color = agent_color(from);
        let to_color = agent_color(to);
        let msg_color = action_color(msg);
        let from_icon = agent_icon(from);
        let to_icon = agent_icon(to);

        // Row 1: [time] FROM → TO
        lines.push(Line::from(vec![
            Span::styled(
                format!("[{:>4}] ", ts_str),
                Style::default().fg(Color::DarkGray),
            ),
            Span::styled(from_icon, Style::default()),
            Span::styled(" ", Style::default()),
            Span::styled(
                from.clone(),
                Style::default().fg(from_color).add_modifier(Modifier::BOLD),
            ),
            Span::styled(" ──▶ ", Style::default().fg(Color::DarkGray)),
            Span::styled(to_icon, Style::default()),
            Span::styled(" ", Style::default()),
            Span::styled(to.clone(), Style::default().fg(to_color)),
        ]));

        // Row 2: message body (indented)
        // Truncate to fit panel width
        let max_msg_chars = (area.width as usize).saturating_sub(10).max(20);
        let display_msg: String = msg.chars().take(max_msg_chars).collect();
        let suffix = if msg.len() > max_msg_chars { "…" } else { "" };

        lines.push(Line::from(vec![
            Span::styled("         ", Style::default()),
            Span::styled(
                format!("{}{}", display_msg, suffix),
                Style::default().fg(msg_color),
            ),
        ]));

        lines.push(Line::from(Span::styled(
            "         ─────────────────────────────────────────────────────",
            Style::default().fg(Color::Rgb(40, 40, 60)),
        )));
    }

    // Show newest messages (scroll to end)
    let total_lines = lines.len();
    let skip = total_lines.saturating_sub(max_visible);
    let visible: Vec<Line> = lines.into_iter().skip(skip).collect();

    let msg_widget = List::new(visible).block(
        Block::default()
            .title(Span::styled(
                " 📡 Live Agent Communications ",
                Style::default()
                    .fg(THEME.brand)
                    .add_modifier(Modifier::BOLD),
            ))
            .borders(Borders::ALL)
            .border_style(Style::default().fg(THEME.border)),
    );
    f.render_widget(msg_widget, chunks[1]);
}
