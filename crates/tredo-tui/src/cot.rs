//! COT Log tab — Live Chain of Thought entries with search and severity badges.

use crate::prelude::*;
use crate::AppState;

/// Determine severity level from agent name and action.
fn severity_badge(agent: &str, action: &str) -> (&'static str, Color) {
    let a = action.to_uppercase();
    let ag = agent.to_lowercase();

    if a.contains("REJECT")
        || a.contains("FAIL")
        || a.contains("HALT")
        || a.contains("ABORT")
        || a.contains("ERROR")
        || ag.contains("guardian")
        || ag.contains("drawdown")
    {
        ("🔴", THEME.danger)
    } else if a.contains("OVERRIDE")
        || a.contains("WARN")
        || a.contains("SKIP")
        || a.contains("RISK")
        || ag.contains("verifier")
        || ag.contains("risk")
    {
        ("🟡", THEME.warning)
    } else if a.contains("BUY") || a.contains("SELL") || a.contains("TRADE") || a.contains("PASS") {
        ("🟢", THEME.positive)
    } else if a.contains("UPDATED")
        || a.contains("START")
        || a.contains("MODEL_SWITCH")
        || a.contains("PIPELINE")
        || ag.contains("metacontrol")
    {
        ("🔵", THEME.info)
    } else {
        ("⚪", THEME.muted)
    }
}

pub fn render_cot(f: &mut Frame, area: Rect, app: &AppState) {
    let filter_active = app.cot_filter_active;
    let filter_text = &app.cot_filter;
    let filter_height: u16 = if filter_active || !filter_text.is_empty() {
        3
    } else {
        0
    };

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(filter_height), Constraint::Min(1)])
        .split(area);

    // ── Search bar ────────────────────────────────────────────────────────
    if filter_height > 0 {
        let search_border_color = if filter_active {
            THEME.neutral
        } else {
            THEME.border
        };
        let search_title = if filter_active {
            " 🔍 Filter COT (Esc to close) "
        } else {
            " 🔍 COT filter "
        };
        let display_text = if filter_text.is_empty() {
            if filter_active {
                "Type to filter by agent or action..."
            } else {
                "No filter active"
            }
        } else {
            filter_text.as_str()
        };

        let search_block = Block::default()
            .title(search_title)
            .borders(Borders::ALL)
            .border_style(Style::default().fg(search_border_color));
        let inner = search_block.inner(chunks[0]);
        f.render_widget(search_block, chunks[0]);

        let cursor_vis = if filter_active { "█" } else { "" };
        let display_str = format!(" {}{}", display_text, cursor_vis);
        let search_content = Paragraph::new(Line::from(vec![Span::styled(
            &display_str,
            Style::default().fg(if filter_active {
                THEME.highlight
            } else {
                THEME.muted
            }),
        )]));
        f.render_widget(search_content, inner);
    }

    // ── Filter items ──────────────────────────────────────────────────────
    let lower_filter = filter_text.to_lowercase();
    let filtered: Vec<&serde_json::Value> = if lower_filter.is_empty() {
        app.cot.iter().collect()
    } else {
        app.cot
            .iter()
            .filter(|entry| {
                let agent = entry.get("agent").and_then(|v| v.as_str()).unwrap_or("");
                let action = entry.get("action").and_then(|v| v.as_str()).unwrap_or("");
                let reason = entry.get("reason").and_then(|v| v.as_str()).unwrap_or("");
                agent.to_lowercase().contains(&lower_filter)
                    || action.to_lowercase().contains(&lower_filter)
                    || reason.to_lowercase().contains(&lower_filter)
            })
            .collect()
    };

    // ── Entries list ──────────────────────────────────────────────────────
    let items: Vec<ListItem> = filtered
        .iter()
        .rev()
        .skip(app.scroll_offset)
        .take(chunks[1].height as usize - 2)
        .map(|entry| {
            let ts = entry
                .get("timestamp")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let agent = entry.get("agent").and_then(|v| v.as_str()).unwrap_or("?");
            let action = entry.get("action").and_then(|v| v.as_str()).unwrap_or("");
            let reason = entry.get("reason").and_then(|v| v.as_str()).unwrap_or("");

            let agent_color = match agent {
                a if a.contains("Identifier") || a.contains("Market") => THEME.positive,
                a if a.contains("Verifier") || a.contains("Risk") => THEME.warning,
                a if a.contains("Executer") || a.contains("Execution") => Color::Magenta,
                a if a.contains("Guardian") || a.contains("Drawdown") => THEME.danger,
                a if a.contains("MetaControl") => THEME.info,
                _ => THEME.highlight,
            };

            let (_badge, severity_color) = severity_badge(agent, action);

            ListItem::new(Line::from(vec![
                // Severity badge
                Span::styled(format!("{} ", _badge), Style::default().fg(severity_color)),
                // Timestamp
                Span::styled(
                    format!("[{}] ", &ts[..ts.len().min(19)]),
                    Style::default().fg(THEME.muted),
                ),
                // Agent name
                Span::styled(
                    format!("{}: ", agent),
                    Style::default()
                        .fg(agent_color)
                        .add_modifier(Modifier::BOLD),
                ),
                // Action
                Span::styled(action, Style::default().fg(THEME.highlight)),
                // Reason
                Span::styled(format!("  {}", reason), Style::default().fg(THEME.muted)),
            ]))
        })
        .collect();

    let title = if lower_filter.is_empty() {
        format!("🔀 Live COT — {} entries", filtered.len())
    } else {
        format!(
            "🔀 Live COT — {} of {} entries",
            filtered.len(),
            app.cot.len()
        )
    };

    let list = List::new(items).block(Block::default().title(title).borders(Borders::ALL));
    f.render_widget(list, chunks[1]);
}
