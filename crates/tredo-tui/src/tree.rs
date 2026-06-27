//! Agent Tree tab — Hierarchical view of all agents with COT and skill scores.
//!
//! Layout:
//!   Top  (70%) — Hierarchical tree with signal bars, action badges, reasoning
//!   Bottom (30%) — Live comm feed: recent inter-agent messages

use std::collections::HashMap;

use crate::prelude::*;
use crate::AppState;

pub fn render_tree(f: &mut Frame, area: Rect, app: &AppState) {
    let legend_height = 5u16;
    let comm_height = if area.height >= 35 { 10u16 } else { 0u16 };
    let tree_height = area
        .height
        .saturating_sub(legend_height)
        .saturating_sub(comm_height);

    let tree_area = Rect {
        x: area.x,
        y: area.y,
        width: area.width,
        height: tree_height,
    };
    let comm_area = if comm_height > 0 {
        Some(Rect {
            x: area.x,
            y: area.y + tree_height,
            width: area.width,
            height: comm_height,
        })
    } else {
        None
    };
    let legend_area = Rect {
        x: area.x,
        y: area.y + tree_height + comm_height,
        width: area.width,
        height: legend_height,
    };

    render_tree_content(f, tree_area, app);
    if let Some(ca) = comm_area {
        render_tree_live_comm(f, ca, app);
    }
    render_tree_legend(f, legend_area);
}

fn render_tree_content(f: &mut Frame, area: Rect, app: &AppState) {
    let tree_json = match &app.agents {
        Some(t) => t.clone(),
        None => {
            let msg = Paragraph::new(
                "Agent tree not available yet.\nThe Tredo hierarchy (Identifier → Verifier → Executer → Guardian) runs in the orchestrator.",
            )
            .block(
                Block::default()
                    .title("🌳 Agent & Sub-Agent Tree")
                    .borders(Borders::ALL),
            );
            f.render_widget(msg, area);
            return;
        }
    };

    let mut lines: Vec<Line> = Vec::new();

    // Aggregated signal header
    if let Some(agg) = &app.aggregated_signal {
        let net = agg
            .get("net_signal")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0);
        let conviction = agg
            .get("conviction")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0);
        let bullish_str = agg
            .get("bullish_strength")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0);
        let bearish_str = agg
            .get("bearish_strength")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0);
        let bullish_cnt = agg
            .get("bullish_count")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        let bearish_cnt = agg
            .get("bearish_count")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        let neutral_cnt = agg
            .get("neutral_count")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);

        let signal_color = if net > 0.15 {
            Color::Green
        } else if net < -0.15 {
            Color::Red
        } else {
            Color::Yellow
        };
        let direction = if net > 0.15 {
            "BULLISH"
        } else if net < -0.15 {
            "BEARISH"
        } else {
            "NEUTRAL"
        };

        lines.push(Line::from(vec![
            Span::styled(
                "  📊 SKILL CONSENSUS: ",
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                direction,
                Style::default()
                    .fg(signal_color)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!(
                    " | net={:+.3} | conv={:.0}% | bull={:.3} bear={:.3} | {}B/{}Be/{}N",
                    net,
                    conviction * 100.0,
                    bullish_str,
                    bearish_str,
                    bullish_cnt,
                    bearish_cnt,
                    neutral_cnt
                ),
                Style::default().fg(Color::DarkGray),
            ),
        ]));
        lines.push(Line::from(""));
    }

    build_tree_lines(
        &tree_json,
        "",
        true,
        true,
        &app.cot_by_agent,
        &app.skill_votes,
        &mut lines,
        0,
    );

    let max_visible = (area.height.saturating_sub(2)) as usize;
    let scroll = app.tree_scroll.min(lines.len().saturating_sub(max_visible));
    let visible: Vec<Line> = lines
        .iter()
        .skip(scroll)
        .take(max_visible)
        .cloned()
        .collect();

    let list = List::new(visible).block(
        Block::default()
            .title(Span::styled(
                " 🌳 Agent & Sub-Agent Tree ",
                Style::default()
                    .fg(THEME.brand)
                    .add_modifier(Modifier::BOLD),
            ))
            .borders(Borders::ALL)
            .border_style(Style::default().fg(THEME.border)),
    );
    f.render_widget(list, area);
}

// ── Live Comm Panel ────────────────────────────────────────────────────────

fn agent_color_tree(name: &str) -> Color {
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
    } else {
        Color::DarkGray
    }
}

fn action_color_tree(msg: &str) -> Color {
    let u = msg.to_uppercase();
    if u.contains("PASS") || u.contains("BUY") || u.contains("ANALYZED") {
        Color::Green
    } else if u.contains("FAIL") || u.contains("HALT") || u.contains("ABORT") {
        Color::Red
    } else if u.contains("HOLD") || u.contains("SKIP") {
        Color::Yellow
    } else if u.contains("START") || u.contains("UPDATE") || u.contains("SWITCH") {
        Color::Cyan
    } else {
        Color::White
    }
}

fn render_tree_live_comm(f: &mut Frame, area: Rect, app: &AppState) {
    let max_visible = area.height.saturating_sub(2) as usize;

    if app.live_comm_log.is_empty() {
        let empty = Paragraph::new(Line::from(Span::styled(
            "  ⏳ No agent communications yet — run a pipeline cycle",
            Style::default()
                .fg(Color::DarkGray)
                .add_modifier(Modifier::ITALIC),
        )))
        .block(
            Block::default()
                .title(Span::styled(
                    " 📡 Live Agent Comm ",
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                ))
                .borders(Borders::TOP | Borders::LEFT | Borders::RIGHT)
                .border_style(Style::default().fg(THEME.border)),
        );
        f.render_widget(empty, area);
        return;
    }

    // Build compact single-line entries for each comm event
    let mut lines: Vec<Line> = Vec::new();
    for (from, to, msg, ts) in &app.live_comm_log {
        let elapsed = ts.elapsed().as_secs();
        let ts_str = if elapsed < 60 {
            format!("{:>3}s", elapsed)
        } else {
            format!("{:>2}m", elapsed / 60)
        };

        let from_col = agent_color_tree(from);
        let to_col = agent_color_tree(to);
        let msg_col = action_color_tree(msg);

        // Trim message for compact display
        let max_chars = (area.width as usize).saturating_sub(40).max(20);
        let short_msg: String = msg.chars().take(max_chars).collect();
        let ellipsis = if msg.len() > max_chars { "…" } else { "" };

        lines.push(Line::from(vec![
            Span::styled(
                format!("[{}] ", ts_str),
                Style::default().fg(Color::Rgb(80, 80, 100)),
            ),
            Span::styled(
                format!("{:<12}", &from[..from.len().min(12)]),
                Style::default().fg(from_col).add_modifier(Modifier::BOLD),
            ),
            Span::styled(" ▶ ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                format!("{:<10}", &to[..to.len().min(10)]),
                Style::default().fg(to_col),
            ),
            Span::styled(" │ ", Style::default().fg(Color::Rgb(50, 50, 70))),
            Span::styled(
                format!("{}{}", short_msg, ellipsis),
                Style::default().fg(msg_col),
            ),
        ]));
    }

    // Show newest messages at bottom
    let total = lines.len();
    let skip = total.saturating_sub(max_visible);
    let visible: Vec<Line> = lines.into_iter().skip(skip).collect();

    let widget = List::new(visible).block(
        Block::default()
            .title(Span::styled(
                format!(" 📡 Live Agent Comm ({} msgs) ", app.live_comm_log.len()),
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ))
            .borders(Borders::TOP | Borders::LEFT | Borders::RIGHT)
            .border_style(Style::default().fg(THEME.border)),
    );
    f.render_widget(widget, area);
}

// ── Tree Building Helpers ──────────────────────────────────────────────────

fn find_skill_vote<'a>(
    name: &str,
    skill_index: &'a HashMap<String, serde_json::Value>,
) -> Option<&'a serde_json::Value> {
    if let Some(vote) = skill_index.get(name) {
        return Some(vote);
    }
    if let Some(stripped) = name.strip_suffix("Agent") {
        if let Some(vote) = skill_index.get(stripped) {
            return Some(vote);
        }
    }
    let with_agent = format!("{}Agent", name);
    if let Some(vote) = skill_index.get(&with_agent) {
        return Some(vote);
    }
    None
}

fn render_score_bar(score: f64, confidence: f64, width: usize) -> (String, Color) {
    let filled = ((score.abs() * width as f64) as usize).min(width);
    let bar: String = (0..width)
        .map(|i| if i < filled { '█' } else { '░' })
        .collect();
    let color = if score > 0.3 {
        Color::Green
    } else if score < -0.3 {
        Color::Red
    } else if confidence > 0.5 {
        Color::Yellow
    } else {
        Color::DarkGray
    };
    (bar, color)
}

#[allow(clippy::too_many_arguments)]
fn build_tree_lines(
    node: &serde_json::Value,
    prefix: &str,
    is_last: bool,
    is_root: bool,
    cot_index: &HashMap<String, serde_json::Value>,
    skill_index: &HashMap<String, serde_json::Value>,
    lines: &mut Vec<Line>,
    depth: usize,
) {
    let name = node.get("name").and_then(|n| n.as_str()).unwrap_or("?");
    let role = node.get("role").and_then(|r| r.as_str()).unwrap_or("");
    let children = node.get("children").and_then(|c| c.as_array());

    let connector = if is_root {
        ""
    } else if is_last {
        "└── "
    } else {
        "├── "
    };
    let display_prefix = if is_root { "" } else { prefix };
    let label = format!("{}{}", display_prefix, connector);

    let name_color = match name {
        n if n.contains("Tredo") => Color::Cyan,
        n if n.contains("Identifier") => Color::Green,
        n if n.contains("Verifier") => Color::Yellow,
        n if n.contains("Executer") => Color::Magenta,
        n if n.contains("Guardian") => Color::Red,
        n if n.contains("MetaControl") => Color::Cyan,
        _ => match depth {
            2 => Color::White,
            _ => Color::Gray,
        },
    };

    let (action_badge, reason_text, confidence_str) = if let Some(cot) = cot_index.get(name) {
        let action = cot.get("action").and_then(|a| a.as_str()).unwrap_or("");
        let reason = cot
            .get("reason")
            .and_then(|r| r.as_str())
            .or_else(|| cot.get("message").and_then(|m| m.as_str()))
            .unwrap_or("");
        let conf = cot
            .get("confidence")
            .and_then(|c| c.as_f64())
            .unwrap_or(0.0);
        (
            format!(" [{}]", action),
            reason.chars().take(50).collect(),
            format!(" ({:.0}%)", conf * 100.0),
        )
    } else {
        (String::new(), String::new(), String::new())
    };

    let action_color = if action_badge.contains("PASS")
        || action_badge.contains("BUY")
        || action_badge.contains("ANALYZED")
    {
        Color::Green
    } else if action_badge.contains("FAIL")
        || action_badge.contains("HALT")
        || action_badge.contains("ABORT")
    {
        Color::Red
    } else if action_badge.contains("HOLD") || action_badge.contains("SKIP") {
        Color::Yellow
    } else if action_badge.contains("PIPELINE_START")
        || action_badge.contains("MODEL_SWITCH")
        || action_badge.contains("UPDATED")
    {
        Color::Cyan
    } else if action_badge.is_empty() {
        Color::DarkGray
    } else {
        Color::White
    };

    let skill_info = find_skill_vote(name, skill_index);
    let (skill_score, skill_conf, skill_dir, skill_weight) = if let Some(vote) = skill_info {
        let sc = vote.get("score").and_then(|v| v.as_f64()).unwrap_or(0.0);
        let co = vote
            .get("confidence")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0);
        let dir = vote
            .get("direction")
            .and_then(|v| v.as_str())
            .unwrap_or("Neutral")
            .to_string();
        let wt = vote.get("weight").and_then(|v| v.as_f64()).unwrap_or(0.0);
        (sc, co, dir, wt)
    } else {
        (0.0_f64, 0.0_f64, String::new(), 0.0_f64)
    };

    let mut spans = Vec::new();
    if !label.is_empty() {
        spans.push(Span::styled(label, Style::default().fg(Color::DarkGray)));
    }

    let name_style = if depth == 0 {
        Style::default()
            .fg(name_color)
            .add_modifier(Modifier::BOLD | Modifier::UNDERLINED)
    } else if depth == 1 {
        Style::default().fg(name_color).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(name_color)
    };
    spans.push(Span::styled(name.to_string(), name_style));

    if !role.is_empty() && depth <= 1 {
        spans.push(Span::styled(
            format!(" — {}", role),
            Style::default()
                .fg(Color::DarkGray)
                .add_modifier(Modifier::ITALIC),
        ));
    }
    if !action_badge.is_empty() {
        spans.push(Span::styled(
            action_badge,
            Style::default()
                .fg(action_color)
                .add_modifier(Modifier::BOLD),
        ));
    }
    if !confidence_str.is_empty() {
        spans.push(Span::styled(
            confidence_str,
            Style::default().fg(Color::DarkGray),
        ));
    }

    if skill_score != 0.0 || skill_conf > 0.0 {
        let dir_icon = match skill_dir.as_str() {
            "Bullish" => "▲",
            "Bearish" => "▼",
            _ => "◆",
        };
        let dir_color = match skill_dir.as_str() {
            "Bullish" => Color::Green,
            "Bearish" => Color::Red,
            _ => Color::DarkGray,
        };
        let (bar, bar_color) = render_score_bar(skill_score, skill_conf, 6);
        spans.push(Span::raw("  "));
        spans.push(Span::styled(dir_icon, Style::default().fg(dir_color)));
        spans.push(Span::styled(
            format!("|{}|", bar),
            Style::default().fg(bar_color),
        ));
        spans.push(Span::styled(
            format!(" {:+.2} ({:.0}%)", skill_score, skill_conf * 100.0),
            Style::default().fg(Color::DarkGray),
        ));
        if skill_weight > 0.0 {
            spans.push(Span::styled(
                format!(" w={:.2}", skill_weight),
                Style::default().fg(Color::Gray),
            ));
        }
    }

    lines.push(Line::from(spans));

    if !reason_text.is_empty() && depth >= 2 {
        let indent = if is_root {
            ""
        } else if is_last {
            "    "
        } else {
            "│   "
        };
        let sub_indent = format!("{}{}      ", display_prefix, indent);
        lines.push(Line::from(vec![
            Span::styled(
                sub_indent,
                Style::default()
                    .fg(Color::DarkGray)
                    .add_modifier(Modifier::ITALIC),
            ),
            Span::styled("▸ ", Style::default().fg(action_color)),
            Span::styled(
                reason_text,
                Style::default()
                    .fg(Color::DarkGray)
                    .add_modifier(Modifier::ITALIC),
            ),
        ]));
    }

    if let Some(children) = children {
        for (i, child) in children.iter().enumerate() {
            let child_is_last = i == children.len() - 1;
            let child_prefix = if is_root {
                String::new()
            } else if is_last {
                format!("{}    ", prefix)
            } else {
                format!("{}│   ", prefix)
            };
            build_tree_lines(
                child,
                &child_prefix,
                child_is_last,
                false,
                cot_index,
                skill_index,
                lines,
                depth + 1,
            );
        }
    }

    if depth == 0 && cot_index.is_empty() && skill_index.is_empty() {
        lines.push(Line::from(Span::styled(
            "  No recent agent activity. Run a pipeline cycle or wait for autonomous operation.",
            Style::default().fg(Color::DarkGray),
        )));
    }
}

fn render_tree_legend(f: &mut Frame, area: Rect) {
    let legend = vec![
        Line::from(vec![Span::styled(
            "  📖 Color Legend",
            Style::default()
                .fg(Color::DarkGray)
                .add_modifier(Modifier::BOLD),
        )]),
        Line::from(vec![
            Span::styled("    ", Style::default()),
            Span::styled("🟢", Style::default().fg(Color::Green)),
            Span::styled(" PASS  | ", Style::default().fg(Color::Green)),
            Span::styled("🔴", Style::default().fg(Color::Red)),
            Span::styled(" FAIL/HALT/ABORT  | ", Style::default().fg(Color::Red)),
            Span::styled("🟡", Style::default().fg(Color::Yellow)),
            Span::styled(" HOLD/SKIP  | ", Style::default().fg(Color::Yellow)),
            Span::styled("🔵", Style::default().fg(Color::Cyan)),
            Span::styled(" START/UPDATED", Style::default().fg(Color::Cyan)),
        ]),
        Line::from(vec![
            Span::styled("    ", Style::default()),
            Span::styled("⬜", Style::default().fg(Color::DarkGray)),
            Span::styled(" Idle  | ", Style::default().fg(Color::DarkGray)),
            Span::styled("▲ ", Style::default().fg(Color::Green)),
            Span::styled("Bullish  | ", Style::default().fg(Color::Green)),
            Span::styled("▼ ", Style::default().fg(Color::Red)),
            Span::styled("Bearish  | ", Style::default().fg(Color::Red)),
            Span::styled("◆ ", Style::default().fg(Color::DarkGray)),
            Span::styled("Neutral  | ", Style::default().fg(Color::DarkGray)),
            Span::styled("▸", Style::default().fg(Color::DarkGray)),
            Span::styled(" Reasoning", Style::default().fg(Color::DarkGray)),
        ]),
    ];

    let p = Paragraph::new(legend)
        .style(Style::default().fg(Color::DarkGray))
        .block(
            Block::default()
                .borders(Borders::TOP)
                .border_style(Style::default().fg(Color::DarkGray)),
        );
    f.render_widget(p, area);
}
