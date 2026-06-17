//! Main UI layout: header, tabs, content, action buttons, footer, error overlay.
//! Also renders the interactive action buttons bar and floating help overlay.
//!
//! Responsive layout: adapts column widths based on terminal width.

use crate::prelude::*;
use crate::{
    render_backtest, render_broker, render_cot, render_dashboard, render_health, render_help,
    render_models, render_performance, render_policy_cache, render_positions, render_rules,
    render_scanner, render_settings, render_tree, render_watchlist, AppState, Tab, ALL_BUTTONS,
    NUM_TABS,
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
            Constraint::Length(4), // header (includes ticker)
            Constraint::Length(3), // tabs
            Constraint::Min(3),    // content
            Constraint::Length(3), // action buttons bar
            Constraint::Length(2), // footer
        ])
        .split(size);

    // ── Header with price ticker ───────────────────────────────────────────
    let ticker_text = build_ticker(app);
    let header_text = format!(
        "tredo  —  Trading Real-time Edge Decision Optimisation  |  Press ? for shortcuts"
    );
    let header = Paragraph::new(vec![
        Line::from(Span::styled(
            header_text,
            Style::default()
                .fg(THEME.brand)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(Span::styled(
            ticker_text,
            Style::default().fg(THEME.highlight),
        )),
    ])
    .block(
        Block::default()
            .borders(Borders::BOTTOM)
            .border_style(Style::default().fg(THEME.brand)),
    );
    f.render_widget(header, chunks[0]);

    // ── Tabs with responsive sizing ────────────────────────────────────────
    let tab_titles: Vec<Line> = (0..NUM_TABS)
        .map(|i| {                let tab = match i {
                    0 => Tab::Dashboard,
                    1 => Tab::Cot,
                    2 => Tab::Positions,
                    3 => Tab::Watchlist,
                    4 => Tab::Models,
                    5 => Tab::Tree,
                    6 => Tab::Rules,
                    7 => Tab::PolicyCache,
                    8 => Tab::Scanner,
                    9 => Tab::Health,
                    10 => Tab::Performance,
                    11 => Tab::Backtest,
                    12 => Tab::Broker,
                    13 => Tab::Settings,
                    _ => Tab::Help,
                };
            if i == app.selected_tab {
                Line::from(Span::styled(
                    tab.title(),
                    Style::default()
                        .fg(THEME.neutral)
                        .add_modifier(Modifier::BOLD),
                ))
            } else {
                Line::from(Span::styled(tab.title(), Style::default().fg(THEME.highlight)))
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
        let base_width = tab_area.width / NUM_TABS as u16;
        let extra = tab_area.width - (base_width * NUM_TABS as u16);
        let mut areas = Vec::with_capacity(NUM_TABS);
        let mut x_offset = tab_area.x;
        for i in 0..NUM_TABS {
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
        7 => render_policy_cache(f, chunks[2], app),
        8 => render_scanner(f, chunks[2], app),
        9 => render_health(f, chunks[2], app),
        10 => render_performance(f, chunks[2], app),
        11 => render_backtest(f, chunks[2], app),
        12 => render_broker(f, chunks[2], app),
        13 => render_settings(f, chunks[2], app),
        _ => render_help(f, chunks[2]),
    }

    // ── Action Buttons Bar ────────────────────────────────────────────────
    render_button_bar(f, chunks[3], app);

    // ── Footer / status — read health indicators from /api/health ─────────
    let ws_indicator = if app.ws_connected {
        Span::styled(" WS ●", Style::default().fg(THEME.positive))
    } else {
        Span::styled(" WS ○", Style::default().fg(THEME.muted))
    };
    let footer_text = if let Some(h) = &app.health {
        let k = if h.get("kronos").and_then(|v| v.as_bool()).unwrap_or(false) {
            "K"
        } else {
            "x"
        };
        let o = if h
            .get("orchestrator")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
        {
            "O"
        } else {
            "x"
        };
        let l = if h.get("llm").and_then(|v| v.as_bool()).unwrap_or(false) {
            "L"
        } else {
            "x"
        };
        let m = app.current_model.as_deref().unwrap_or("unknown");
        format!(
            "Services: {} {} {} | Model: {} | Ticker: {} items | Poll: {}s ago | COT: {}",
            k,
            o,
            l,
            m,
            app.crypto_prices.len(),
            app.last_poll.map(|t| t.elapsed().as_secs()).unwrap_or(0),
            app.ws_cot_count
        )
    } else {
        "Connecting to backend... (run `tredo` or `tredo start`)".to_string()
    };

    let footer = Paragraph::new(Line::from(vec![
        Span::raw(footer_text),
        ws_indicator,
    ]))
    .style(Style::default().fg(THEME.muted))
    .block(Block::default().borders(Borders::TOP));
    f.render_widget(footer, chunks[4]);

    // ── Error overlay ─────────────────────────────────────────────────────
    if let Some(err) = &app.error {
        let err_p = Paragraph::new(err.as_str())
            .style(Style::default().fg(THEME.danger))
            .wrap(Wrap { trim: true });
        let err_area = Rect {
            x: 2,
            y: size.height.saturating_sub(4),
            width: size.width - 4,
            height: 2,
        };
        f.render_widget(err_p, err_area);
    }

    // ── Keyboard Shortcuts Overlay (shown when ? is pressed) ──────────────
    if app.show_overlay {
        render_overlay(f, size);
    }
}

/// Build the scrolling ticker text from crypto prices.
fn build_ticker(app: &AppState) -> String {
    if app.crypto_prices.is_empty() {
        return "  No price data yet. Ensure backend is running.".to_string();
    }
    let mut parts: Vec<String> = Vec::new();
    for sym in ["BTC", "ETH", "SOL", "XRP", "ADA", "DOGE", "AVAX"] {
        if let Some(data) = app.crypto_prices.get(sym) {
            let price = data.get("price").and_then(|v| v.as_f64()).unwrap_or(0.0);
            let change = data
                .get("binance")
                .and_then(|b| b.get("change_pct_24h"))
                .and_then(|v| v.as_f64())
                .unwrap_or(0.0);
            let arrow = if change > 0.0 { "▲" } else { "▼" };
            parts.push(format!(
                "{} {:.0} {} {:.1}%",
                sym, price, arrow, change.abs()
            ));
        }
    }
    if parts.is_empty() {
        return "  Prices loading...".to_string();
    }
    format!("  {}", parts.join("  |  "))
}

/// Render a floating keyboard shortcuts overlay in the center of the screen.
fn render_overlay(f: &mut Frame, area: Rect) {
    let overlay_width = area.width.min(60);
    let overlay_height = 26u16;
    let overlay_x = area.x + (area.width.saturating_sub(overlay_width)) / 2;
    let overlay_y = area.y + (area.height.saturating_sub(overlay_height)) / 2;

    let overlay_area = Rect {
        x: overlay_x,
        y: overlay_y,
        width: overlay_width,
        height: overlay_height,
    };

    let help_lines = vec![
        Line::from(Span::styled(
            "  KEYBOARD SHORTCUTS",
            Style::default()
                .fg(THEME.brand)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from(vec![
            Span::styled("  Tab            ", Style::default().fg(THEME.neutral)),
            Span::styled("Switch tabs forward", Style::default().fg(THEME.highlight)),
        ]),
        Line::from(vec![
            Span::styled("  Shift+Tab      ", Style::default().fg(THEME.neutral)),
            Span::styled("Switch tabs backward", Style::default().fg(THEME.highlight)),
        ]),
        Line::from(vec![
            Span::styled("  1-0            ", Style::default().fg(THEME.neutral)),
            Span::styled("Jump to tab by number", Style::default().fg(THEME.highlight)),
        ]),
        Line::from(vec![
            Span::styled("  b              ", Style::default().fg(THEME.neutral)),
            Span::styled("Jump to Broker page", Style::default().fg(THEME.highlight)),
        ]),
        Line::from(vec![
            Span::styled("  S              ", Style::default().fg(THEME.neutral)),
            Span::styled("Jump to Settings page", Style::default().fg(THEME.highlight)),
        ]),
        Line::from(vec![
            Span::styled("  /              ", Style::default().fg(THEME.neutral)),
            Span::styled("Search (COT Log, Policy Cache)", Style::default().fg(THEME.highlight)),
        ]),
        Line::from(vec![
            Span::styled("  s              ", Style::default().fg(THEME.neutral)),
            Span::styled("Sort column (Policy Cache)", Style::default().fg(THEME.highlight)),
        ]),
        Line::from(vec![
            Span::styled("  ?              ", Style::default().fg(THEME.neutral)),
            Span::styled("Toggle this overlay", Style::default().fg(THEME.highlight)),
        ]),
        Line::from(vec![
            Span::styled("  r              ", Style::default().fg(THEME.neutral)),
            Span::styled("Force refresh all data", Style::default().fg(THEME.highlight)),
        ]),
        Line::from(vec![
            Span::styled("  t / y          ", Style::default().fg(THEME.neutral)),
            Span::styled("Toggle trade entry form (Positions)", Style::default().fg(THEME.highlight)),
        ]),
        Line::from(vec![
            Span::styled("  ↑ ↓ (in form)  ", Style::default().fg(THEME.neutral)),
            Span::styled("Adjust value / toggle option", Style::default().fg(THEME.highlight)),
        ]),
        Line::from(vec![
            Span::styled("  ← → (in form)  ", Style::default().fg(THEME.neutral)),
            Span::styled("Switch field focus", Style::default().fg(THEME.highlight)),
        ]),
        Line::from(vec![
            Span::styled("  y              ", Style::default().fg(THEME.neutral)),
            Span::styled("Confirm / Toggle trade form", Style::default().fg(THEME.highlight)),
        ]),
        Line::from(vec![
            Span::styled("  Up/Down        ", Style::default().fg(THEME.neutral)),
            Span::styled("Scroll / Select", Style::default().fg(THEME.highlight)),
        ]),
        Line::from(vec![
            Span::styled("  Left/Right     ", Style::default().fg(THEME.neutral)),
            Span::styled("Navigate buttons", Style::default().fg(THEME.highlight)),
        ]),
        Line::from(vec![
            Span::styled("  Enter          ", Style::default().fg(THEME.neutral)),
            Span::styled("Confirm / Activate / Submit trade", Style::default().fg(THEME.highlight)),
        ]),
        Line::from(vec![
            Span::styled("  Esc            ", Style::default().fg(THEME.neutral)),
            Span::styled("Back / Cancel / Close", Style::default().fg(THEME.highlight)),
        ]),
        Line::from(vec![
            Span::styled("  q / Ctrl-C     ", Style::default().fg(THEME.neutral)),
            Span::styled("Quit UI (backend keeps running)", Style::default().fg(THEME.highlight)),
        ]),
        Line::from(""),
        Line::from(Span::styled(
            "  Press Esc or Enter to close",
            Style::default().fg(THEME.muted),
        )),
    ];

    let overlay = Paragraph::new(help_lines)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(THEME.brand)),
        )
        .style(Style::default().bg(Color::Black));
    f.render_widget(overlay, overlay_area);
}

// ═══════════════════════════════════════════════════════════════════════════
//  ACTION BUTTONS BAR
// ═══════════════════════════════════════════════════════════════════════════

/// Render the interactive action buttons bar at the bottom of the screen.
pub fn render_button_bar(f: &mut Frame, area: Rect, app: &mut AppState) {
    let button_widths: Vec<Constraint> = ALL_BUTTONS
        .iter()
        .map(|b| {
            let label_len = b.icon().len() + b.label().len() + 4;
            Constraint::Length(label_len as u16)
        })
        .collect();

    let mut constraints = button_widths;
    constraints.push(Constraint::Min(2));

    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints(constraints)
        .split(area);

    for (i, button) in ALL_BUTTONS.iter().enumerate() {
        let focused = app.button_focus_offset == i;
        let running = app.action_running == Some(*button);

        let display_text = if running {
            format!(" {} ...", button.label().trim())
        } else {
            format!("{}{}", button.icon(), button.label())
        };

        let (text_style, border_color) = if running {
            (
                Style::default().fg(THEME.info).add_modifier(Modifier::DIM),
                THEME.info,
            )
        } else if focused {
            (
                Style::default()
                    .fg(THEME.neutral)
                    .add_modifier(Modifier::BOLD),
                THEME.neutral,
            )
        } else {
            (Style::default().fg(THEME.highlight), THEME.border)
        };

        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(border_color));

        let p = Paragraph::new(Line::from(Span::styled(display_text, text_style)))
            .block(block)
            .alignment(Alignment::Center);
        f.render_widget(p, chunks[i]);
    }

    app.button_areas = chunks[..ALL_BUTTONS.len()].to_vec();

    if let Some(filler) = chunks.last() {
        let hint = if app.confirm_action.is_some() {
            Line::from(vec![
                Span::styled("  ", Style::default().fg(THEME.highlight)),
                Span::styled(
                    app.action_message
                        .as_ref()
                        .map(|(m, _)| m.as_str())
                        .unwrap_or("Confirm?"),
                    Style::default()
                        .fg(THEME.neutral)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled("  --  Press ", Style::default().fg(THEME.muted)),
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
                Span::styled(" / ", Style::default().fg(THEME.muted)),
                Span::styled("Esc", Style::default().fg(THEME.negative)),
                Span::styled(" to cancel", Style::default().fg(THEME.muted)),
            ])
        } else if let Some((msg, _)) = &app.action_message {
            Line::from(vec![Span::styled(msg, Style::default().fg(THEME.positive))])
        } else {
            let focused_desc = ALL_BUTTONS
                .get(app.button_focus_offset)
                .map(|b| b.description())
                .unwrap_or("");
            Line::from(vec![
                Span::styled("  <- ->  ", Style::default().fg(THEME.muted)),
                Span::styled("Enter", Style::default().fg(THEME.neutral)),
                Span::styled("  ", Style::default().fg(THEME.muted)),
                Span::styled(
                    focused_desc,
                    Style::default()
                        .fg(THEME.muted)
                        .add_modifier(Modifier::ITALIC),
                ),
            ])
        };
        let hint_p = Paragraph::new(hint).alignment(Alignment::Left);
        f.render_widget(hint_p, *filler);
    }
}

type WidgetTabs = ratatui::widgets::Tabs<'static>;
