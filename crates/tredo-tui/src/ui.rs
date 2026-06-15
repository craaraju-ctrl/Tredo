//! Main UI layout: header, tabs, content, action buttons, footer, error overlay.
//! Also renders the interactive action buttons bar.

use crate::prelude::*;
use crate::{
    render_cot, render_dashboard, render_help, render_models, render_positions, render_rules,
    render_tree, render_watchlist, AppState, Tab, ALL_BUTTONS,
};

/// Main UI renderer — layouts the screen and delegates to tab-specific renderers.
pub fn ui(f: &mut Frame, app: &mut AppState) {
    let size = f.area();

    // Guard against tiny terminals
    if size.width < 60 || size.height < 20 {
        let msg = Paragraph::new("Terminal too small. Resize to at least 60x20.")
            .style(Style::default().fg(Color::Red))
            .alignment(Alignment::Center);
        f.render_widget(msg, size);
        return;
    }

    // ── Layout: header, tabs, content, action buttons, footer ──────────────
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // header
            Constraint::Length(3), // tabs
            Constraint::Min(3),    // content
            Constraint::Length(3), // action buttons bar
            Constraint::Length(2), // footer
        ])
        .split(size);

    // ── Header ─────────────────────────────────────────────────────────────
    let header = Paragraph::new(vec![
        Line::from(vec![
            Span::styled(
                "⚡ tredo",
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw("  —  "),
            Span::styled(
                "Trading Real-time Edge Decision Optimisation",
                Style::default().fg(Color::White),
            ),
        ]),
        Line::from(Span::styled(
            "Full Terminal UI  •  Autonomous • Paper Only  •  Press q to quit, Tab/1-8 to navigate, Enter to activate",
            Style::default().fg(Color::DarkGray),
        )),
    ])
    .block(
        Block::default()
            .borders(Borders::BOTTOM)
            .border_style(Style::default().fg(Color::Cyan)),
    );
    f.render_widget(header, chunks[0]);

    // ── Tabs ──────────────────────────────────────────────────────────────
    let tab_titles: Vec<Line> = (0..8)
        .map(|i| {
            let tab = match i {
                0 => Tab::Dashboard,
                1 => Tab::Cot,
                2 => Tab::Positions,
                3 => Tab::Watchlist,
                4 => Tab::Models,
                5 => Tab::Tree,
                6 => Tab::Rules,
                _ => Tab::Help,
            };
            if i == app.selected_tab {
                Line::from(Span::styled(
                    tab.title(),
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD),
                ))
            } else {
                Line::from(Span::styled(tab.title(), Style::default().fg(Color::White)))
            }
        })
        .collect();

    let tabs = WidgetTabs::new(tab_titles)
        .block(Block::default().borders(Borders::BOTTOM))
        .select(app.selected_tab);
    f.render_widget(tabs, chunks[1]);

    // ── Compute tab areas for mouse click detection ────────────────────────
    {
        let tab_area = chunks[1];
        let num_tabs = 8usize;
        let base_width = tab_area.width / num_tabs as u16;
        let extra = tab_area.width - (base_width * num_tabs as u16);
        let mut areas = Vec::with_capacity(num_tabs);
        let mut x_offset = tab_area.x;
        for i in 0..num_tabs {
            let w = if (i as u16) < extra {
                base_width + 1
            } else {
                base_width
            };
            areas.push(Rect {
                x: x_offset,
                y: tab_area.y,
                width: w,
                height: tab_area.height,
            });
            x_offset += w;
        }
        app.tab_areas = areas;
    }

    // ── Tab Content ───────────────────────────────────────────────────────
    match app.selected_tab {
        0 => render_dashboard(f, chunks[2], app),
        1 => render_cot(f, chunks[2], app),
        2 => render_positions(f, chunks[2], app),
        3 => render_watchlist(f, chunks[2], app),
        4 => render_models(f, chunks[2], app),
        5 => render_tree(f, chunks[2], app),
        6 => render_rules(f, chunks[2], app),
        _ => render_help(f, chunks[2]),
    }

    // ── Action Buttons Bar ────────────────────────────────────────────────
    render_button_bar(f, chunks[3], app);

    // ── Footer / status — read health indicators from /api/health ─────────
    let footer_text = if let Some(h) = &app.health {
        let k = if h.get("kronos").and_then(|v| v.as_bool()).unwrap_or(false) {
            "🔷"
        } else {
            "❌"
        };
        let o = if h
            .get("orchestrator")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
        {
            "⚙️"
        } else {
            "❌"
        };
        let l = if h.get("llm").and_then(|v| v.as_bool()).unwrap_or(false) {
            "🤖"
        } else {
            "❌"
        };
        let m = app.current_model.as_deref().unwrap_or("unknown");
        format!(
            "K:{} O:{} L:{} | Model: {} | Last: {:?}s ago",
            k,
            o,
            l,
            m,
            app.last_poll.map(|t| t.elapsed().as_secs()).unwrap_or(0)
        )
    } else {
        "Connecting to backend... (run `tredo` or `tredo start`)".to_string()
    };

    let footer = Paragraph::new(footer_text)
        .style(Style::default().fg(Color::DarkGray))
        .block(Block::default().borders(Borders::TOP));
    f.render_widget(footer, chunks[4]);

    // ── Error overlay ─────────────────────────────────────────────────────
    if let Some(err) = &app.error {
        let err_p = Paragraph::new(err.as_str())
            .style(Style::default().fg(Color::Red))
            .wrap(Wrap { trim: true });
        let err_area = Rect {
            x: 2,
            y: size.height.saturating_sub(4),
            width: size.width - 4,
            height: 2,
        };
        f.render_widget(err_p, err_area);
    }
}

// ═══════════════════════════════════════════════════════════════════════════
//  ACTION BUTTONS BAR
// ═══════════════════════════════════════════════════════════════════════════

/// Render the interactive action buttons bar at the bottom of the screen.
pub fn render_button_bar(f: &mut Frame, area: Rect, app: &mut AppState) {
    // Each button gets a fixed width based on its label + borders + padding
    let button_widths: Vec<Constraint> = ALL_BUTTONS
        .iter()
        .map(|b| {
            let label_len = b.icon().len() + b.label().len() + 4; // padding + borders
            Constraint::Length(label_len as u16)
        })
        .collect();

    let mut constraints = button_widths;
    constraints.push(Constraint::Min(2)); // filler for hint / message text

    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints(constraints)
        .split(area);

    // ── Render each action button ──────────────────────────────────────────
    for (i, button) in ALL_BUTTONS.iter().enumerate() {
        let focused = app.button_focus_offset == i;
        let running = app.action_running == Some(*button);

        let display_text = if running {
            format!(" {} ⏳", button.label().trim())
        } else {
            format!("{}{}", button.icon(), button.label())
        };

        let (text_style, border_color) = if running {
            (
                Style::default().fg(Color::Cyan).add_modifier(Modifier::DIM),
                Color::Cyan,
            )
        } else if focused {
            (
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
                Color::Yellow,
            )
        } else {
            (Style::default().fg(Color::White), Color::DarkGray)
        };

        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(border_color));

        let p = Paragraph::new(Line::from(Span::styled(display_text, text_style)))
            .block(block)
            .alignment(Alignment::Center);
        f.render_widget(p, chunks[i]);
    }

    // ── Store button areas for mouse click detection ──────────────────────
    app.button_areas = chunks[..ALL_BUTTONS.len()].to_vec();

    // ── Hint / status message in the filler area ──────────────────────────
    if let Some(filler) = chunks.last() {
        let hint = if app.confirm_action.is_some() {
            // Show the confirmation prompt prominently
            Line::from(vec![
                Span::styled("  ", Style::default().fg(Color::White)),
                Span::styled(
                    app.action_message
                        .as_ref()
                        .map(|(m, _)| m.as_str())
                        .unwrap_or("Confirm?"),
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled("  —  Press ", Style::default().fg(Color::DarkGray)),
                Span::styled(
                    "y",
                    Style::default()
                        .fg(Color::Green)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(" to confirm, ", Style::default().fg(Color::DarkGray)),
                Span::styled(
                    "n",
                    Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
                ),
                Span::styled(" / ", Style::default().fg(Color::DarkGray)),
                Span::styled("Esc", Style::default().fg(Color::Red)),
                Span::styled(" to cancel", Style::default().fg(Color::DarkGray)),
            ])
        } else if let Some((msg, _)) = &app.action_message {
            Line::from(vec![Span::styled(msg, Style::default().fg(Color::Green))])
        } else {
            let focused_desc = ALL_BUTTONS
                .get(app.button_focus_offset)
                .map(|b| b.description())
                .unwrap_or("");
            Line::from(vec![
                Span::styled("  ← →  ", Style::default().fg(Color::DarkGray)),
                Span::styled("Enter", Style::default().fg(Color::Yellow)),
                Span::styled("  ", Style::default().fg(Color::DarkGray)),
                Span::styled(
                    focused_desc,
                    Style::default()
                        .fg(Color::DarkGray)
                        .add_modifier(Modifier::ITALIC),
                ),
            ])
        };
        let hint_p = Paragraph::new(hint).alignment(Alignment::Left);
        f.render_widget(hint_p, *filler);
    }
}

// Alias to avoid naming conflict with the Tabs enum
type WidgetTabs = ratatui::widgets::Tabs<'static>;
