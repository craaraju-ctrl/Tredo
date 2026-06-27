//! Rules tab — Structured discipline rules toggle switches.
//!
//! Full TUI upgrade:
//!   Left  (50%) — Active rules with toggle indicators + progress bars
//!   Right (50%) — Recent rule violations log + recent agent actions

use crate::prelude::*;
use crate::AppState;

/// Build a horizontal fill bar for a 0.0-1.0 value.
fn fill_bar(value: f64, width: usize, filled_char: &str, empty_char: &str) -> String {
    let filled = ((value * width as f64).round() as usize).min(width);
    let empty = width - filled;
    format!("{}{}", filled_char.repeat(filled), empty_char.repeat(empty))
}

pub fn render_rules(f: &mut Frame, area: Rect, app: &AppState) {
    // ── Split: Rules panel | Activity panel ───────────────────────────────
    let split = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(52), Constraint::Percentage(48)])
        .split(area);

    render_rules_panel(f, split[0], app);
    render_activity_panel(f, split[1], app);
}

// ── Left: Rules Panel ─────────────────────────────────────────────────────

fn render_rules_panel(f: &mut Frame, area: Rect, app: &AppState) {
    let status = app.status.as_ref();

    // ── Extract all rule values ──────────────────────────────────────────
    let use_confluence = status
        .and_then(|s| s.get("use_confluence"))
        .and_then(|v| v.as_bool())
        .unwrap_or(true);
    let respect_session = status
        .and_then(|s| s.get("respect_session_timing"))
        .and_then(|v| v.as_bool())
        .unwrap_or(true);
    let min_confidence: f64 = status
        .and_then(|s| s.get("min_confidence_score"))
        .and_then(|v| v.as_f64())
        .unwrap_or(0.60);
    let max_daily_loss: f64 = status
        .and_then(|s| s.get("max_daily_loss"))
        .and_then(|v| v.as_f64())
        .unwrap_or(5000.0);
    let max_position_size: f64 = status
        .and_then(|s| s.get("max_position_size"))
        .and_then(|v| v.as_f64())
        .unwrap_or(10000.0);
    let daily_pnl: f64 = status
        .and_then(|s| s.get("daily_pnl"))
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0);
    let trade_count: u64 = status
        .and_then(|s| s.get("total_trades"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let win_rate: f64 = status
        .and_then(|s| s.get("win_rate"))
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0);

    // ── Build content ────────────────────────────────────────────────────
    let bar_width = 16usize;

    let mut lines = vec![
        Line::from(vec![Span::styled(
            "  ⚖️  ACTIVE DISCIPLINE RULES",
            Style::default()
                .fg(THEME.brand)
                .add_modifier(Modifier::BOLD),
        )]),
        Line::from(""),
    ];

    // ─ Section 1: Boolean Toggles ─
    lines.push(Line::from(vec![Span::styled(
        "  ┌─ BOOLEAN TOGGLES ─",
        Style::default().fg(Color::DarkGray),
    )]));
    lines.push(Line::from(""));

    // Confluence toggle
    let conf_badge = if use_confluence {
        Span::styled(
            "  ● ON ",
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD),
        )
    } else {
        Span::styled("  ○ OFF", Style::default().fg(Color::DarkGray))
    };
    lines.push(Line::from(vec![
        Span::styled("  │ ", Style::default().fg(Color::DarkGray)),
        Span::styled(
            "Use Confluence Score   ",
            Style::default().fg(THEME.highlight),
        ),
        conf_badge,
        Span::styled(
            "  [toggle: tredo rules use_confluence=true]",
            Style::default().fg(Color::Rgb(60, 60, 80)),
        ),
    ]));
    lines.push(Line::from(""));

    // Session timing toggle
    let sess_badge = if respect_session {
        Span::styled(
            "  ● ON ",
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD),
        )
    } else {
        Span::styled("  ○ OFF", Style::default().fg(Color::DarkGray))
    };
    lines.push(Line::from(vec![
        Span::styled("  │ ", Style::default().fg(Color::DarkGray)),
        Span::styled(
            "Respect Session Timing ",
            Style::default().fg(THEME.highlight),
        ),
        sess_badge,
        Span::styled(
            "  [toggle: tredo rules respect_session_timing=true]",
            Style::default().fg(Color::Rgb(60, 60, 80)),
        ),
    ]));
    lines.push(Line::from(""));

    // ─ Section 2: Numeric Thresholds ─
    lines.push(Line::from(vec![Span::styled(
        "  ├─ NUMERIC THRESHOLDS ─",
        Style::default().fg(Color::DarkGray),
    )]));
    lines.push(Line::from(""));

    // Min confidence score
    let conf_bar = fill_bar(min_confidence, bar_width, "▰", "▱");
    let conf_color = if min_confidence >= 0.7 {
        Color::Green
    } else if min_confidence >= 0.5 {
        Color::Yellow
    } else {
        Color::Red
    };
    lines.push(Line::from(vec![
        Span::styled("  │ ", Style::default().fg(Color::DarkGray)),
        Span::styled(
            format!("{:<24}", "Min Confidence Score"),
            Style::default().fg(THEME.highlight),
        ),
        Span::styled(
            format!("{:.2}  ", min_confidence),
            Style::default().fg(conf_color).add_modifier(Modifier::BOLD),
        ),
        Span::styled(conf_bar, Style::default().fg(conf_color)),
    ]));
    lines.push(Line::from(""));

    // Max daily loss
    let loss_ratio = (-daily_pnl).max(0.0) / max_daily_loss.max(1.0);
    let loss_ratio_clamped = loss_ratio.min(1.0);
    let loss_bar = fill_bar(loss_ratio_clamped, bar_width, "▰", "▱");
    let loss_color = if loss_ratio < 0.5 {
        Color::Green
    } else if loss_ratio < 0.8 {
        Color::Yellow
    } else {
        Color::Red
    };
    lines.push(Line::from(vec![
        Span::styled("  │ ", Style::default().fg(Color::DarkGray)),
        Span::styled(
            format!("{:<24}", "Max Daily Loss"),
            Style::default().fg(THEME.highlight),
        ),
        Span::styled(
            format!("₹{:<8.0}", max_daily_loss),
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(loss_bar, Style::default().fg(loss_color)),
        Span::styled(
            format!(" used {:.0}%", loss_ratio_clamped * 100.0),
            Style::default().fg(loss_color),
        ),
    ]));
    lines.push(Line::from(""));

    // Max position size
    lines.push(Line::from(vec![
        Span::styled("  │ ", Style::default().fg(Color::DarkGray)),
        Span::styled(
            format!("{:<24}", "Max Position Size"),
            Style::default().fg(THEME.highlight),
        ),
        Span::styled(
            format!("₹{:<8.0}", max_position_size),
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        ),
    ]));
    lines.push(Line::from(""));

    // ─ Section 3: Live Performance Stats ─
    lines.push(Line::from(vec![Span::styled(
        "  ├─ LIVE PERFORMANCE ─",
        Style::default().fg(Color::DarkGray),
    )]));
    lines.push(Line::from(""));

    // Daily P&L
    let pnl_color = if daily_pnl >= 0.0 {
        Color::Green
    } else {
        Color::Red
    };
    lines.push(Line::from(vec![
        Span::styled("  │ ", Style::default().fg(Color::DarkGray)),
        Span::styled(
            format!("{:<24}", "Today's P&L"),
            Style::default().fg(THEME.highlight),
        ),
        Span::styled(
            format!("₹{:+.2}", daily_pnl),
            Style::default().fg(pnl_color).add_modifier(Modifier::BOLD),
        ),
    ]));
    lines.push(Line::from(""));

    // Win rate
    let wr_bar = fill_bar(win_rate, bar_width, "█", "░");
    let wr_color = if win_rate >= 0.6 {
        Color::Green
    } else if win_rate >= 0.5 {
        Color::Yellow
    } else {
        Color::Red
    };
    lines.push(Line::from(vec![
        Span::styled("  │ ", Style::default().fg(Color::DarkGray)),
        Span::styled(
            format!("{:<24}", "Win Rate"),
            Style::default().fg(THEME.highlight),
        ),
        Span::styled(
            format!("{:.0}%  ", win_rate * 100.0),
            Style::default().fg(wr_color).add_modifier(Modifier::BOLD),
        ),
        Span::styled(wr_bar, Style::default().fg(wr_color)),
    ]));
    lines.push(Line::from(""));

    // Trade count
    lines.push(Line::from(vec![
        Span::styled("  │ ", Style::default().fg(Color::DarkGray)),
        Span::styled(
            format!("{:<24}", "Total Trades"),
            Style::default().fg(THEME.highlight),
        ),
        Span::styled(
            format!("{}", trade_count),
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
    ]));
    lines.push(Line::from(""));

    // ─ Footer hint ─
    lines.push(Line::from(vec![Span::styled(
        "  └─────────────────────────────────────────────────────",
        Style::default().fg(Color::DarkGray),
    )]));
    lines.push(Line::from(""));
    lines.push(Line::from(vec![Span::styled(
        "  ℹ️  Set rules via: tredo rules key=value",
        Style::default().fg(THEME.muted),
    )]));
    lines.push(Line::from(vec![Span::styled(
        "     min_confidence_score=0.72 | max_daily_loss=3000",
        Style::default().fg(THEME.muted),
    )]));

    let p = Paragraph::new(lines)
        .block(
            Block::default()
                .title(Span::styled(
                    " ⚖️  Discipline Rules ",
                    Style::default()
                        .fg(THEME.brand)
                        .add_modifier(Modifier::BOLD),
                ))
                .borders(Borders::ALL)
                .border_style(Style::default().fg(THEME.border)),
        )
        .wrap(Wrap { trim: true });
    f.render_widget(p, area);
}

// ── Right: Recent Activity / Violations Panel ─────────────────────────────

fn render_activity_panel(f: &mut Frame, area: Rect, app: &AppState) {
    // Split into top (agent summary) and bottom (comm log)
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(9), // Agent status grid
            Constraint::Min(3),    // Recent communications
        ])
        .split(area);

    render_agent_status_grid(f, chunks[0], app);
    render_recent_comm(f, chunks[1], app);
}

fn render_agent_status_grid(f: &mut Frame, area: Rect, app: &AppState) {
    // Show current action + confidence for each pipeline agent
    let agents = [
        ("TredoAgent", "Tredo", Color::Cyan),
        ("IdentifierAgent", "Identifier", Color::Green),
        ("VerifierAgent", "Verifier", Color::Yellow),
        ("ExecuterAgent", "Executer", Color::Magenta),
        ("GuardianAgent", "Guardian", Color::Red),
    ];

    let mut lines = vec![
        Line::from(vec![Span::styled(
            "  AGENT STATUS SNAPSHOT",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )]),
        Line::from(""),
    ];

    for (full_name, short_name, color) in &agents {
        // Try both full and short name lookups
        let cot = app
            .cot_by_agent
            .get(*full_name)
            .or_else(|| app.cot_by_agent.get(*short_name));

        let (action, conf_pct, action_col) = if let Some(entry) = cot {
            let action = entry
                .get("action")
                .and_then(|a| a.as_str())
                .unwrap_or("idle");
            let conf = entry
                .get("confidence")
                .and_then(|c| c.as_f64())
                .unwrap_or(0.0);
            let col = match action {
                a if a.contains("PASS") || a.contains("BUY") => Color::Green,
                a if a.contains("FAIL") || a.contains("HALT") => Color::Red,
                a if a.contains("HOLD") || a.contains("SKIP") => Color::Yellow,
                _ => Color::White,
            };
            (action.to_string(), conf * 100.0, col)
        } else {
            ("—".to_string(), 0.0, Color::DarkGray)
        };

        // Tiny inline bar
        let bar_w = 8usize;
        let filled = ((conf_pct / 100.0 * bar_w as f64) as usize).min(bar_w);
        let bar: String = (0..bar_w)
            .map(|i| if i < filled { '▪' } else { '·' })
            .collect();

        lines.push(Line::from(vec![
            Span::styled(
                format!("  {:<10}", short_name),
                Style::default().fg(*color).add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!("{:<12}", &action[..action.len().min(12)]),
                Style::default().fg(action_col),
            ),
            Span::styled(bar, Style::default().fg(*color)),
            Span::styled(
                format!(" {:.0}%", conf_pct),
                Style::default().fg(Color::DarkGray),
            ),
        ]));
    }

    let para = Paragraph::new(lines)
        .block(
            Block::default()
                .title(Span::styled(
                    " 🤖 Agent Status ",
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                ))
                .borders(Borders::ALL)
                .border_style(Style::default().fg(THEME.border)),
        )
        .wrap(Wrap { trim: true });
    f.render_widget(para, area);
}

fn render_recent_comm(f: &mut Frame, area: Rect, app: &AppState) {
    let max_visible = area.height.saturating_sub(2) as usize;

    if app.live_comm_log.is_empty() {
        let empty = Paragraph::new(Line::from(Span::styled(
            "  ⏳ No communications yet — run a pipeline cycle",
            Style::default()
                .fg(Color::DarkGray)
                .add_modifier(Modifier::ITALIC),
        )))
        .block(
            Block::default()
                .title(Span::styled(
                    " 📡 Recent Comms ",
                    Style::default()
                        .fg(Color::DarkGray)
                        .add_modifier(Modifier::BOLD),
                ))
                .borders(Borders::ALL)
                .border_style(Style::default().fg(THEME.border)),
        );
        f.render_widget(empty, area);
        return;
    }

    let mut lines: Vec<Line> = Vec::new();
    for (from, _to, msg, ts) in &app.live_comm_log {
        let elapsed = ts.elapsed().as_secs();
        let ts_str = if elapsed < 60 {
            format!("{:>3}s", elapsed)
        } else {
            format!("{:>2}m", elapsed / 60)
        };

        let from_col = if from.contains("Tredo") || from.contains("Meta") {
            Color::Cyan
        } else if from.contains("Identifier") {
            Color::Green
        } else if from.contains("Verifier") {
            Color::Yellow
        } else if from.contains("Executer") {
            Color::Magenta
        } else if from.contains("Guardian") {
            Color::Red
        } else if from.contains("Ollama") {
            Color::Blue
        } else if from.contains("Kronos") {
            Color::LightBlue
        } else {
            Color::DarkGray
        };

        let msg_col = {
            let u = msg.to_uppercase();
            if u.contains("PASS") || u.contains("BUY") {
                Color::Green
            } else if u.contains("FAIL") || u.contains("HALT") || u.contains("ABORT") {
                Color::Red
            } else if u.contains("HOLD") || u.contains("SKIP") {
                Color::Yellow
            } else {
                Color::White
            }
        };

        let max_chars = (area.width as usize).saturating_sub(22).max(10);
        let short_msg: String = msg.chars().take(max_chars).collect();

        lines.push(Line::from(vec![
            Span::styled(
                format!("[{}] ", ts_str),
                Style::default().fg(Color::Rgb(70, 70, 90)),
            ),
            Span::styled(
                format!("{:<10}", &from[..from.len().min(10)]),
                Style::default().fg(from_col).add_modifier(Modifier::BOLD),
            ),
            Span::styled(" ", Style::default()),
            Span::styled(short_msg, Style::default().fg(msg_col)),
        ]));
    }

    // Show newest at bottom
    let total = lines.len();
    let skip = total.saturating_sub(max_visible);
    let visible: Vec<Line> = lines.into_iter().skip(skip).collect();

    let widget = List::new(visible).block(
        Block::default()
            .title(Span::styled(
                format!(" 📡 Recent Comms ({} msgs) ", app.live_comm_log.len()),
                Style::default()
                    .fg(Color::DarkGray)
                    .add_modifier(Modifier::BOLD),
            ))
            .borders(Borders::ALL)
            .border_style(Style::default().fg(THEME.border)),
    );
    f.render_widget(widget, area);
}
