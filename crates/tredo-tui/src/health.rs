//! System Health tab — Service status, latency, and resource monitoring dashboard.
//!
//! Displays the health of backend services (Kronos, Ollama, Orchestrator loops),
//! polling latency, and data freshness indicators.

use crate::prelude::*;
use crate::AppState;

pub fn render_health(f: &mut Frame, area: Rect, app: &AppState) {
    let health = app.health.as_ref();
    let _status = app.status.as_ref();
    let policy_cache = app.policy_cache.as_ref();

    let kronos_up = health
        .and_then(|h| h.get("kronos"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let orch_up = health
        .and_then(|h| h.get("orchestrator"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let llm_up = health
        .and_then(|h| h.get("llm"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let running = health
        .and_then(|h| h.get("running"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let model = app.current_model.as_deref().unwrap_or("unknown");

    let cache_size = policy_cache
        .and_then(|c| c.get("total_entries"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let cot_count = app.cot.len();
    let watch_count = app.watchlist.len();
    let poll_secs = app
        .last_poll
        .map(|t| t.elapsed().as_secs())
        .unwrap_or(999);

    // Layout: top row (service cards), bottom row (stats + gauges)
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(6), // Service status cards
            Constraint::Length(6), // Stats cards
            Constraint::Min(3),    // Model + uptime details
        ])
        .split(area);

    // ── Service Status Row ─────────────────────────────────────────────────
    let service_row = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Ratio(1, 3),
            Constraint::Ratio(1, 3),
            Constraint::Ratio(1, 3),
        ])
        .split(chunks[0]);

    // Kronos card
    render_service_card(
        f,
        service_row[0],
        "Kronos Data Service",
        if kronos_up { "🟢 ONLINE" } else { "🔴 OFFLINE" },
        if kronos_up { THEME.positive } else { THEME.danger },
        "Powers market data & feature computation",
    );

    // Ollama / LLM card
    render_service_card(
        f,
        service_row[1],
        "Ollama LLM",
        if llm_up { "🟢 ONLINE" } else { "🔴 OFFLINE" },
        if llm_up { THEME.positive } else { THEME.danger },
        &format!("Model: {}", model),
    );

    // Orchestrator card
    render_service_card(
        f,
        service_row[2],
        "Orchestrator Loops",
        if running { "🟢 RUNNING" } else { "⏸️ STOPPED" },
        if running { THEME.positive } else { THEME.warning },
        if running {
            "Fast(5s) + Medium(5m) + Slow(24h)"
        } else {
            "Start via API: POST /api/start"
        },
    );

    // ── Stats Row ──────────────────────────────────────────────────────────
    let stats_row = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Ratio(1, 3),
            Constraint::Ratio(1, 3),
            Constraint::Ratio(1, 3),
        ])
        .split(chunks[1]);

    // Poll latency card
    let poll_color = if poll_secs < 5 {
        THEME.positive
    } else if poll_secs < 15 {
        THEME.warning
    } else {
        THEME.danger
    };
    render_stat_card(
        f,
        stats_row[0],
        "🔄 Poll Status",
        &format!("{}s ago", poll_secs),
        poll_color,
        "Data freshness indicator",
    );

    // Data volume card
    render_stat_card(
        f,
        stats_row[1],
        "📊 Data Volume",
        &format!("COT: {}, WL: {}, Cache: {}", cot_count, watch_count, cache_size),
        THEME.info,
        "COT entries | Watchlist items | Cache entries",
    );

    // Backend status card
    let all_ok = kronos_up && orch_up && llm_up;
    render_stat_card(
        f,
        stats_row[2],
        "✅ Overall Health",
        if all_ok { "ALL SYSTEMS OK" } else { "SOME ISSUES" },
        if all_ok { THEME.positive } else { THEME.warning },
        if all_ok {
            "All services operational"
        } else {
            "Check individual service status"
        },
    );

    // ── Details area ───────────────────────────────────────────────────────
    let mut detail_lines = vec![
        Line::from(Span::styled(
            "  SYSTEM DETAILS",
            Style::default()
                .fg(THEME.brand)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
    ];

    // Model info
    detail_lines.push(Line::from(vec![
        Span::styled("  LLM Model:     ", Style::default().fg(THEME.muted)),
        Span::styled(model, Style::default().fg(THEME.highlight)),
    ]));

    // Cache info
    detail_lines.push(Line::from(vec![
        Span::styled("  Policy Cache:  ", Style::default().fg(THEME.muted)),
        Span::styled(
            format!("{} entries", cache_size),
            Style::default().fg(THEME.highlight),
        ),
    ]));

    // Watchlist info
    detail_lines.push(Line::from(vec![
        Span::styled("  Watchlist:     ", Style::default().fg(THEME.muted)),
        Span::styled(
            format!("{} symbols: {}", watch_count, app.watchlist.join(", ")),
            Style::default().fg(THEME.highlight),
        ),
    ]));

    // COT history count
    detail_lines.push(Line::from(vec![
        Span::styled("  COT History:   ", Style::default().fg(THEME.muted)),
        Span::styled(
            format!("{} entries", cot_count),
            Style::default().fg(THEME.highlight),
        ),
    ]));

    detail_lines.push(Line::from(""));
    detail_lines.push(Line::from(Span::styled(
        "  Press ? for keyboard shortcuts",
        Style::default().fg(THEME.muted),
    )));

    let para = Paragraph::new(detail_lines)
        .block(
            Block::default()
                .title("Details")
                .borders(Borders::ALL)
                .border_style(Style::default().fg(THEME.border)),
        )
        .wrap(Wrap { trim: true });
    f.render_widget(para, chunks[2]);
}

/// Render a service status card with a title, status text, color, and subtitle.
fn render_service_card(
    f: &mut Frame,
    area: Rect,
    title: &str,
    status: &str,
    status_color: Color,
    subtitle: &str,
) {
    let inner = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(2), Constraint::Length(1), Constraint::Min(1)])
        .split(
            Block::default()
                .title(title)
                .borders(Borders::ALL)
                .border_style(Style::default().fg(THEME.border))
                .inner(area),
        );

    // Border + title
    let block = Block::default()
        .title(Span::styled(title, Style::default().fg(THEME.brand)))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(THEME.border));
    f.render_widget(block, area);

    // Status text (centered)
    let status_para = Paragraph::new(Line::from(Span::styled(
        status,
        Style::default()
            .fg(status_color)
            .add_modifier(Modifier::BOLD),
    )))
    .alignment(Alignment::Center);
    f.render_widget(status_para, inner[0]);

    // Subtitle
    let sub_para = Paragraph::new(Line::from(Span::styled(
        subtitle,
        Style::default().fg(THEME.muted),
    )))
    .alignment(Alignment::Center);
    f.render_widget(sub_para, inner[2]);
}

/// Render a stat card with an emoji prefix, value, and subtitle.
fn render_stat_card(
    f: &mut Frame,
    area: Rect,
    title: &str,
    value: &str,
    value_color: Color,
    subtitle: &str,
) {
    let block = Block::default()
        .title(Span::styled(title, Style::default().fg(THEME.muted)))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(THEME.border));
    let inner = block.inner(area);
    f.render_widget(block, area);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(2), Constraint::Min(1)])
        .split(inner);

    let val_para = Paragraph::new(Line::from(Span::styled(
        value,
        Style::default()
            .fg(value_color)
            .add_modifier(Modifier::BOLD),
    )))
    .alignment(Alignment::Center);
    f.render_widget(val_para, chunks[0]);

    let sub_para = Paragraph::new(Line::from(Span::styled(
        subtitle,
        Style::default().fg(THEME.muted),
    )))
    .alignment(Alignment::Center);
    f.render_widget(sub_para, chunks[1]);
}
