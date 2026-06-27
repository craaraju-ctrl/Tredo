//! Settings & Control tab — Interactive system configuration.
//!
//! Panel focus navigation:
//!   - Tab or Left/Right: switch between panels (Models, Agents, Skills, Risk)
//!   - Up/Down: navigate rows within the focused panel
//!   - Enter: toggle agent enable/disable or start risk parameter editing
//!   - +/-: adjust risk parameter value (when editing)
//!   - y/N: confirm/cancel risk parameter change
//!   - Esc: exit editing mode or deselect

use crate::prelude::*;
use crate::AppState;

/// Panel indices
pub(crate) const PANEL_MODELS: usize = 0;
pub(crate) const PANEL_AGENTS: usize = 1;
pub(crate) const PANEL_SKILLS: usize = 2;
pub(crate) const PANEL_RISK: usize = 3;

/// Agent names matching the 7-layer pipeline (indices 0-6)
pub(crate) const AGENT_NAMES: &[&str] = &[
    "HardRulesGate",
    "Identifier",
    "Verifier",
    "BullTeam",
    "BearTeam",
    "Judge",
    "Execution",
];

pub fn render_settings(f: &mut Frame, area: Rect, app: &mut AppState) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // Title
            Constraint::Min(5),    // Content
        ])
        .split(area);

    // ── Title ──────────────────────────────────────────────────────────────
    let title = Paragraph::new(Line::from(Span::styled(
        "⚙️  SETTINGS & CONTROL  —  ← → panels  |  ↑ ↓ navigate  |  Enter toggle/edit",
        Style::default()
            .fg(THEME.brand)
            .add_modifier(Modifier::BOLD),
    )));
    f.render_widget(title, chunks[0]);

    // ── Two-column layout ──────────────────────────────────────────────────
    let columns = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Ratio(1, 2), Constraint::Ratio(1, 2)])
        .split(chunks[1]);

    let left = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Ratio(1, 2), Constraint::Ratio(1, 2)])
        .split(columns[0]);

    let right = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Ratio(1, 2), Constraint::Ratio(1, 2)])
        .split(columns[1]);

    let focused = app.settings_panel_focus;
    let editing = app.risk_editing;
    let row = app.settings_row_focus;

    render_llm_models_panel(f, left[0], app, focused == PANEL_MODELS, row);
    render_agents_panel(f, left[1], app, focused == PANEL_AGENTS, row);
    render_skills_panel(f, right[0], app, focused == PANEL_SKILLS, row);
    render_risk_params_panel(f, right[1], app, focused == PANEL_RISK, row, editing);

    // ── Settings message overlay ───────────────────────────────────────────
    if let Some((msg, time)) = &app.settings_message {
        if time.elapsed() < std::time::Duration::from_secs(3) {
            let overlay = Paragraph::new(Line::from(Span::styled(
                msg,
                Style::default()
                    .fg(THEME.positive)
                    .add_modifier(Modifier::BOLD),
            )))
            .alignment(Alignment::Center);
            let area = Rect {
                x: 2,
                y: area.y + area.height - 2,
                width: area.width - 4,
                height: 1,
            };
            f.render_widget(overlay, area);
        } else {
            app.settings_message = None;
        }
    }

    // ── Confirmation dialog ────────────────────────────────────────────────
    if let Some(ref msg) = app.settings_confirm {
        render_confirm_dialog(f, area, msg);
    }
}

fn panel_border_color(focused: bool) -> Color {
    if focused {
        THEME.neutral
    } else {
        THEME.border
    }
}

fn render_llm_models_panel(f: &mut Frame, area: Rect, app: &AppState, focused: bool, _row: usize) {
    let border_color = panel_border_color(focused);
    let title_prefix = if focused { "▶ " } else { "  " };
    let block = Block::default()
        .title(Span::styled(
            format!("{}🤖 LLM Models", title_prefix),
            Style::default()
                .fg(if focused { THEME.neutral } else { THEME.info })
                .add_modifier(Modifier::BOLD),
        ))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border_color));

    let inner = block.inner(area);
    f.render_widget(block, area);

    let current = app.current_model.as_deref().unwrap_or("unknown");
    let mut lines = vec![
        Line::from(vec![
            Span::styled("Active: ", Style::default().fg(THEME.muted)),
            Span::styled(
                current,
                Style::default()
                    .fg(THEME.positive)
                    .add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(""),
        Line::from(Span::styled(
            "Available:",
            Style::default()
                .fg(THEME.highlight)
                .add_modifier(Modifier::BOLD),
        )),
    ];

    for (i, model) in app.models.iter().enumerate() {
        let name = model
            .get("name")
            .and_then(|n| n.as_str())
            .unwrap_or("unknown");
        let is_active = Some(name) == app.current_model.as_deref();
        let marker = if is_active { "●" } else { "○" };
        let color = if is_active {
            THEME.positive
        } else if focused && i == _row {
            THEME.neutral
        } else {
            THEME.muted
        };

        lines.push(Line::from(vec![
            Span::styled(format!("  {} ", marker), Style::default().fg(color)),
            Span::styled(name, Style::default().fg(color)),
        ]));
    }

    if app.models.is_empty() {
        lines.push(Line::from(Span::styled(
            "  No models detected",
            Style::default().fg(THEME.muted),
        )));
    }

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "  ↑/↓ select, Enter to switch",
        Style::default().fg(THEME.muted),
    )));

    let p = Paragraph::new(lines).wrap(Wrap { trim: true });
    f.render_widget(p, inner);
}

fn render_agents_panel(
    f: &mut Frame,
    area: Rect,
    app: &AppState,
    focused: bool,
    selected_row: usize,
) {
    let border_color = panel_border_color(focused);
    let title_prefix = if focused { "▶ " } else { "  " };
    let block = Block::default()
        .title(Span::styled(
            format!("{}🎭 Agent Hierarchy", title_prefix),
            Style::default()
                .fg(if focused {
                    THEME.neutral
                } else {
                    THEME.warning
                })
                .add_modifier(Modifier::BOLD),
        ))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border_color));

    let inner = block.inner(area);
    f.render_widget(block, area);

    let mut lines = vec![Line::from(Span::styled(
        "5-Layer Pipeline:",
        Style::default()
            .fg(THEME.highlight)
            .add_modifier(Modifier::BOLD),
    ))];

    for (i, (name, enabled)) in AGENT_NAMES.iter().zip(app.agent_enabled.iter()).enumerate() {
        let is_selected = focused && i == selected_row;
        let toggle_marker = if *enabled { "●" } else { "○" };
        let toggle_color = if *enabled {
            THEME.positive
        } else {
            THEME.negative
        };
        let row_color = if is_selected {
            THEME.neutral
        } else {
            THEME.highlight
        };

        lines.push(Line::from(vec![
            Span::styled(
                format!("  {} ", toggle_marker),
                Style::default().fg(toggle_color),
            ),
            Span::styled(
                *name,
                Style::default().fg(row_color).add_modifier(if is_selected {
                    Modifier::BOLD
                } else {
                    Modifier::empty()
                }),
            ),
            Span::styled(
                if *enabled { " ON" } else { " OFF" },
                Style::default().fg(toggle_color),
            ),
        ]));
    }

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "  ↑↓ navigate, Enter toggle",
        Style::default().fg(THEME.muted),
    )));

    let p = Paragraph::new(lines).wrap(Wrap { trim: true });
    f.render_widget(p, inner);
}

fn render_skills_panel(f: &mut Frame, area: Rect, app: &AppState, focused: bool, _row: usize) {
    let border_color = panel_border_color(focused);
    let title_prefix = if focused { "▶ " } else { "  " };
    let block = Block::default()
        .title(Span::styled(
            format!("{}🔧 Skills & Tools", title_prefix),
            Style::default()
                .fg(if focused {
                    THEME.neutral
                } else {
                    THEME.positive
                })
                .add_modifier(Modifier::BOLD),
        ))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border_color));

    let inner = block.inner(area);
    f.render_widget(block, area);

    let mut lines = vec![Line::from(Span::styled(
        "Active Skills:",
        Style::default()
            .fg(THEME.highlight)
            .add_modifier(Modifier::BOLD),
    ))];

    if app.skill_votes.is_empty() {
        lines.push(Line::from(Span::styled(
            "  No skill data available",
            Style::default().fg(THEME.muted),
        )));
    } else {
        for (name, vote) in &app.skill_votes {
            let direction = vote
                .get("direction")
                .and_then(|d| d.as_str())
                .unwrap_or("Neutral");
            let score = vote.get("score").and_then(|s| s.as_f64()).unwrap_or(0.0);
            let dir_color = match direction {
                "Bullish" => THEME.positive,
                "Bearish" => THEME.negative,
                _ => THEME.muted,
            };
            let marker = match direction {
                "Bullish" => "▲",
                "Bearish" => "▼",
                _ => "◆",
            };

            lines.push(Line::from(vec![
                Span::styled(format!("  {} ", marker), Style::default().fg(dir_color)),
                Span::styled(
                    name,
                    Style::default()
                        .fg(THEME.highlight)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(format!(" ({:.2})", score), Style::default().fg(THEME.muted)),
                Span::styled(format!(" {}", direction), Style::default().fg(dir_color)),
            ]));
        }
    }

    // Aggregated signal
    if let Some(ref agg) = app.aggregated_signal {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "Aggregated Signal:",
            Style::default()
                .fg(THEME.highlight)
                .add_modifier(Modifier::BOLD),
        )));
        let consensus = agg
            .get("consensus")
            .and_then(|c| c.as_str())
            .unwrap_or("Neutral");
        let conviction = agg
            .get("conviction")
            .and_then(|c| c.as_f64())
            .unwrap_or(0.0);
        let consensus_color = match consensus {
            "Bullish" => THEME.positive,
            "Bearish" => THEME.negative,
            _ => THEME.muted,
        };

        lines.push(Line::from(vec![
            Span::styled("  Consensus: ", Style::default().fg(THEME.muted)),
            Span::styled(
                consensus,
                Style::default()
                    .fg(consensus_color)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!(" (conviction: {:.0}%)", conviction * 100.0),
                Style::default().fg(THEME.info),
            ),
        ]));
    }

    let p = Paragraph::new(lines).wrap(Wrap { trim: true });
    f.render_widget(p, inner);
}

/// Risk parameters: Max Risk/Trade, Max Daily DD, Max Portfolio Heat, Max Daily Trades, Max Consec Losses
const RISK_PARAM_NAMES: &[&str] = &[
    "Max Risk/Trade",
    "Max Daily DD",
    "Max Portfolio Heat",
    "Max Daily Trades",
    "Max Consec Losses",
];
fn render_risk_params_panel(
    f: &mut Frame,
    area: Rect,
    app: &AppState,
    focused: bool,
    selected_row: usize,
    editing: bool,
) {
    let border_color = panel_border_color(focused);
    let title_prefix = if focused { "▶ " } else { "  " };
    let block = Block::default()
        .title(Span::styled(
            format!("{}⚠️  Risk Parameters", title_prefix),
            Style::default()
                .fg(if focused {
                    THEME.neutral
                } else {
                    THEME.negative
                })
                .add_modifier(Modifier::BOLD),
        ))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border_color));

    let inner = block.inner(area);
    f.render_widget(block, area);

    let status = app.status.as_ref();

    let max_risk = status
        .and_then(|s| s.get("max_risk_per_trade"))
        .and_then(|v| v.as_f64())
        .unwrap_or(0.01);

    let max_dd = status
        .and_then(|s| s.get("max_daily_drawdown"))
        .and_then(|v| v.as_f64())
        .unwrap_or(0.03);

    let max_heat = status
        .and_then(|s| s.get("max_portfolio_heat"))
        .and_then(|v| v.as_f64())
        .unwrap_or(0.15);

    let max_trades = status
        .and_then(|s| s.get("max_daily_trades"))
        .and_then(|v| v.as_u64())
        .unwrap_or(10);

    let max_losses = status
        .and_then(|s| s.get("max_consecutive_losses"))
        .and_then(|v| v.as_u64())
        .unwrap_or(3);

    let values = [
        max_risk * 100.0,
        max_dd * 100.0,
        max_heat * 100.0,
        max_trades as f64,
        max_losses as f64,
    ];
    let units = ["%", "%", "%", "", ""];

    let mut lines = vec![Line::from(Span::styled(
        "Hard Limits:",
        Style::default()
            .fg(THEME.highlight)
            .add_modifier(Modifier::BOLD),
    ))];

    for (i, (name, (val, unit))) in RISK_PARAM_NAMES
        .iter()
        .zip(values.iter().zip(units.iter()))
        .enumerate()
    {
        let is_selected = focused && i == selected_row;
        let is_editing_this = is_selected && editing;

        let value_display = if *unit == "%" {
            format!("{:.1}{}", val, unit)
        } else {
            format!("{:.0}{}", val, unit)
        };

        let marker = if is_editing_this {
            "✎"
        } else if is_selected {
            "▶"
        } else {
            " "
        };

        let value_color = if is_editing_this {
            THEME.neutral
        } else {
            THEME.negative
        };

        lines.push(Line::from(vec![
            Span::styled(
                format!("  {} ", marker),
                Style::default().fg(if is_selected {
                    THEME.neutral
                } else {
                    THEME.muted
                }),
            ),
            Span::styled(
                format!("{:<18}", name),
                Style::default()
                    .fg(if is_selected {
                        THEME.neutral
                    } else {
                        THEME.muted
                    })
                    .add_modifier(if is_selected {
                        Modifier::BOLD
                    } else {
                        Modifier::empty()
                    }),
            ),
            Span::styled(
                value_display,
                Style::default()
                    .fg(value_color)
                    .add_modifier(Modifier::BOLD),
            ),
            if is_editing_this {
                Span::styled("  ± Enter to confirm", Style::default().fg(THEME.muted))
            } else {
                Span::raw("")
            },
        ]));
    }

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "Gate Status:",
        Style::default()
            .fg(THEME.highlight)
            .add_modifier(Modifier::BOLD),
    )));
    lines.push(Line::from(vec![
        Span::styled("  Trading: ", Style::default().fg(THEME.muted)),
        Span::styled(
            "ENABLED",
            Style::default()
                .fg(THEME.positive)
                .add_modifier(Modifier::BOLD),
        ),
    ]));
    lines.push(Line::from(vec![
        Span::styled("  Mode:    ", Style::default().fg(THEME.muted)),
        Span::styled(
            "PAPER",
            Style::default().fg(THEME.info).add_modifier(Modifier::BOLD),
        ),
    ]));
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "  ↑↓ navigate  |  Enter edit  |  +/- adjust",
        Style::default().fg(THEME.muted),
    )));

    let p = Paragraph::new(lines).wrap(Wrap { trim: true });
    f.render_widget(p, inner);
}

fn render_confirm_dialog(f: &mut Frame, area: Rect, msg: &str) {
    let dialog_width = area.width.min(60);
    let dialog_height = 5;
    let x = area.x + (area.width.saturating_sub(dialog_width)) / 2;
    let y = area.y + (area.height.saturating_sub(dialog_height)) / 2;
    let dialog_area = Rect {
        x,
        y,
        width: dialog_width,
        height: dialog_height,
    };

    let block = Block::default()
        .title(Span::styled(
            "⚠️  CONFIRM CHANGE",
            Style::default()
                .fg(THEME.warning)
                .add_modifier(Modifier::BOLD),
        ))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(THEME.warning));

    let inner = block.inner(dialog_area);
    f.render_widget(block, dialog_area);

    let lines = vec![
        Line::from(Span::styled(msg, Style::default().fg(THEME.highlight))),
        Line::from(""),
        Line::from(vec![
            Span::styled("  Press ", Style::default().fg(THEME.muted)),
            Span::styled(
                "y",
                Style::default()
                    .fg(THEME.positive)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(" to confirm, ", Style::default().fg(THEME.muted)),
            Span::styled(
                "n",
                Style::default()
                    .fg(THEME.negative)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(" / Esc to cancel", Style::default().fg(THEME.muted)),
        ]),
    ];

    let p = Paragraph::new(lines).wrap(Wrap { trim: true });
    f.render_widget(p, inner);
}
