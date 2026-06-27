//! System Health tab — Service status, latency, and resource monitoring dashboard.
//!
//! Displays the health of backend services (Kronos, Ollama/LLM, Orchestrator loops),
//! polling latency, and data freshness indicators.
//!
//! Service status is streamed via WebSocket as `"service_status"` messages from
//! the ServiceManager, containing per-service connection status, latency, and uptime.
//! Each service card includes a live sparkline showing the last 10 response times.

use crate::prelude::*;
use crate::AppState;

/// Helper: extract a service status field from the service_status JSON.
/// `services` is the `"services"` object from the WS message.
fn service_field(
    services: &serde_json::Value,
    service_name: &str,
    field: &str,
) -> Option<serde_json::Value> {
    services
        .get(service_name)
        .and_then(|s| s.get(field))
        .cloned()
}

/// Helper: extract a string field from a service in app.service_status.
fn service_str(services: &serde_json::Value, service_name: &str, field: &str) -> Option<String> {
    service_field(services, service_name, field).and_then(|v| v.as_str().map(String::from))
}

/// Helper: extract a f64 field from a service.
fn service_f64(services: &serde_json::Value, service_name: &str, field: &str) -> Option<f64> {
    service_field(services, service_name, field).and_then(|v| v.as_f64())
}

/// Helper: extract a u64 field from a service.
fn service_u64(services: &serde_json::Value, service_name: &str, field: &str) -> Option<u64> {
    service_field(services, service_name, field).and_then(|v| v.as_u64())
}

/// Helper: extract response_time_history as Vec<u64> from a service.
fn service_history(services: &serde_json::Value, service_name: &str) -> Vec<u64> {
    services
        .get(service_name)
        .and_then(|s| s.get("response_time_history"))
        .and_then(|v| v.as_array())
        .map(|arr| arr.iter().filter_map(|v| v.as_u64()).collect())
        .unwrap_or_default()
}

/// Compute uptime % from checks_total and consecutive_failures.
fn compute_uptime_pct(total: u64, failures: u64) -> f64 {
    if total == 0 {
        0.0
    } else {
        (total.saturating_sub(failures)) as f64 / total as f64 * 100.0
    }
}

/// Get a color for a connection status string.
fn status_color(status: &str) -> Color {
    match status {
        "Healthy" => Color::Green,
        "Degraded" => Color::Yellow,
        "Down" => Color::Red,
        _ => Color::DarkGray,
    }
}

/// Get a (dark, bright) color pair for a status string — dark for sparkline bg, bright for line.
fn sparkline_colors(status: &str) -> (Color, Color) {
    match status {
        "Healthy" => (Color::DarkGray, Color::Green),
        "Degraded" => (Color::DarkGray, Color::Yellow),
        "Down" => (Color::DarkGray, Color::Red),
        _ => (Color::DarkGray, Color::DarkGray),
    }
}

/// Get an emoji/icon for a connection status string.
fn status_emoji(status: &str) -> &'static str {
    match status {
        "Healthy" => "🟢",
        "Degraded" => "🟡",
        "Down" => "🔴",
        _ => "❓",
    }
}

/// Format latency in a human-readable way.
fn format_latency(ms_opt: Option<f64>) -> String {
    ms_opt
        .map(|ms| {
            if ms < 1.0 {
                "<1ms".to_string()
            } else if ms < 1000.0 {
                format!("{:.0}ms", ms)
            } else {
                format!("{:.1}s", ms / 1000.0)
            }
        })
        .unwrap_or_else(|| "?ms".to_string())
}

/// Format uptime percentage with appropriate color.
fn uptime_color(pct: f64) -> Color {
    if pct >= 99.0 {
        Color::Green
    } else if pct >= 90.0 {
        Color::Yellow
    } else {
        Color::Red
    }
}

/// Compute a reasonable max for sparkline scaling (at least 1, at most 5000ms, room above max data point).
fn sparkline_max(history: &[u64]) -> u64 {
    let max_val = history.iter().copied().max().unwrap_or(50);
    // Scale up to give headroom: round up to nearest nice number
    if max_val < 10 {
        50
    } else if max_val < 50 {
        100
    } else if max_val < 100 {
        200
    } else if max_val < 500 {
        1000
    } else if max_val < 1000 {
        2000
    } else {
        5000
    }
}

pub fn render_health(f: &mut Frame, area: Rect, app: &AppState) {
    let health = app.health.as_ref();
    let _status = app.status.as_ref();
    let policy_cache = app.policy_cache.as_ref();

    // ── Extract ServiceManager status ─────────────────────────────────────
    let services_json = app.service_status.as_ref().and_then(|s| s.get("services"));

    let model = app.current_model.as_deref().unwrap_or("unknown");

    // Kronos service
    let kronos_status = services_json
        .and_then(|s| service_str(s, "kronos", "status"))
        .unwrap_or_else(|| {
            if health
                .and_then(|h| h.get("kronos"))
                .and_then(|v| v.as_bool())
                .unwrap_or(false)
            {
                "Healthy"
            } else {
                "Unknown"
            }
            .to_string()
        });
    let kronos_latency =
        services_json.and_then(|s| service_f64(s, "kronos", "response_time_avg_ms"));
    let kronos_uptime = services_json.map(|s| {
        let total = service_u64(s, "kronos", "checks_total").unwrap_or(0);
        let fails = service_u64(s, "kronos", "consecutive_failures").unwrap_or(0);
        compute_uptime_pct(total, fails)
    });
    let kronos_endpoint = services_json
        .and_then(|s| service_str(s, "kronos", "endpoint"))
        .unwrap_or_else(|| "...".to_string());
    let kronos_history = services_json
        .map(|s| service_history(s, "kronos"))
        .unwrap_or_default();

    // LLM service — try to find any key containing "llm"
    let llm_key = services_json.and_then(|s| {
        s.as_object()
            .and_then(|obj| obj.keys().find(|k| k.contains("llm")).cloned())
    });
    let llm_status = llm_key
        .as_ref()
        .and_then(|key| services_json.and_then(|s| service_str(s, key, "status")))
        .unwrap_or_else(|| {
            if health
                .and_then(|h| h.get("llm"))
                .and_then(|v| v.as_bool())
                .unwrap_or(false)
            {
                "Healthy"
            } else {
                "Unknown"
            }
            .to_string()
        });
    let llm_latency = llm_key
        .as_ref()
        .and_then(|key| services_json.and_then(|s| service_f64(s, key, "response_time_avg_ms")));
    let llm_uptime = llm_key.as_ref().map(|key| {
        services_json
            .map(|s| {
                let total = service_u64(s, key, "checks_total").unwrap_or(0);
                let fails = service_u64(s, key, "consecutive_failures").unwrap_or(0);
                compute_uptime_pct(total, fails)
            })
            .unwrap_or(0.0)
    });
    let llm_endpoint = llm_key
        .as_ref()
        .and_then(|key| services_json.and_then(|s| service_str(s, key, "endpoint")))
        .unwrap_or_else(|| "...".to_string());
    let llm_history = llm_key
        .as_ref()
        .and_then(|key| services_json.map(|s| service_history(s, key)))
        .unwrap_or_default();

    // Broker service — find any key containing "broker"
    let broker_key = services_json.and_then(|s| {
        s.as_object()
            .and_then(|obj| obj.keys().find(|k| k.contains("broker")).cloned())
    });
    let broker_status = broker_key
        .as_ref()
        .and_then(|key| services_json.and_then(|s| service_str(s, key, "status")))
        .unwrap_or_else(|| "Unknown".to_string());
    let broker_latency = broker_key
        .as_ref()
        .and_then(|key| services_json.and_then(|s| service_f64(s, key, "response_time_avg_ms")));
    let broker_uptime = broker_key.as_ref().map(|key| {
        services_json
            .map(|s| {
                let total = service_u64(s, key, "checks_total").unwrap_or(0);
                let fails = service_u64(s, key, "consecutive_failures").unwrap_or(0);
                compute_uptime_pct(total, fails)
            })
            .unwrap_or(0.0)
    });
    let broker_endpoint = broker_key
        .as_ref()
        .and_then(|key| services_json.and_then(|s| service_str(s, key, "endpoint")))
        .unwrap_or_else(|| "internal (paper)".to_string());
    let broker_history = broker_key
        .as_ref()
        .and_then(|key| services_json.map(|s| service_history(s, key)))
        .unwrap_or_default();
    let broker_name = broker_key
        .as_ref()
        .map(|k| k.replace("broker_", "").replace('_', " "))
        .map(|n| {
            n.chars()
                .next()
                .map(|c| c.to_uppercase().to_string() + &n[1..])
                .unwrap_or(n)
        })
        .unwrap_or_else(|| "Broker".to_string());

    // Orchestrator loops
    let orch_up = health
        .and_then(|h| h.get("orchestrator"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let running = health
        .and_then(|h| h.get("running"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    // ── Stats ─────────────────────────────────────────────────────────────
    let cache_size = policy_cache
        .and_then(|c| c.get("total_entries"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let cot_count = app.cot.len();
    let watch_count = app.watchlist.len();
    let poll_secs = app.last_poll.map(|t| t.elapsed().as_secs()).unwrap_or(999);

    // ── Layout ────────────────────────────────────────────────────────────
    // Increased service row height from 6 to 9 to accommodate the sparkline
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(9), // Service status cards (was 6)
            Constraint::Length(6), // Stats cards
            Constraint::Min(3),    // Details
        ])
        .split(area);

    // ── Service Status Row ────────────────────────────────────────────────
    let service_row = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Ratio(1, 4),
            Constraint::Ratio(1, 4),
            Constraint::Ratio(1, 4),
            Constraint::Ratio(1, 4),
        ])
        .split(chunks[0]);

    // Kronos card — with sparkline of last 10 response times
    let (k_bg, k_line) = sparkline_colors(&kronos_status);
    render_service_card(
        f,
        service_row[0],
        "Kronos Data Service",
        &format!("{} {}", status_emoji(&kronos_status), kronos_status),
        status_color(&kronos_status),
        &format!(
            "{} avg | uptime {:>5}",
            format_latency(kronos_latency),
            kronos_uptime
                .map(|p| format!("{:.0}%", p))
                .unwrap_or_else(|| "?%".to_string()),
        ),
        &kronos_history,
        sparkline_max(&kronos_history),
        k_bg,
        k_line,
        &format!("API: {}", kronos_endpoint),
    );

    // LLM card — with sparkline of last 10 response times
    let (l_bg, l_line) = sparkline_colors(&llm_status);
    render_service_card(
        f,
        service_row[1],
        "LLM Server",
        &format!("{} {}", status_emoji(&llm_status), llm_status),
        status_color(&llm_status),
        &format!("{} avg | model: {}", format_latency(llm_latency), model,),
        &llm_history,
        sparkline_max(&llm_history),
        l_bg,
        l_line,
        llm_endpoint.as_str(),
    );

    // Broker card — with sparkline if live broker, simplified if paper
    let (b_bg, b_line) = sparkline_colors(&broker_status);
    render_service_card(
        f,
        service_row[2],
        &format!("{} API", broker_name),
        &format!("{} {}", status_emoji(&broker_status), broker_status),
        status_color(&broker_status),
        &format!(
            "{} avg | uptime {:>5}",
            format_latency(broker_latency),
            broker_uptime
                .map(|p| format!("{:.0}%", p))
                .unwrap_or_else(|| "?".to_string()),
        ),
        &broker_history,
        sparkline_max(&broker_history),
        b_bg,
        b_line,
        broker_endpoint.as_str(),
    );

    // Orchestrator card — stays from old health.json (no sparkline since no ServiceManager data)
    render_orchestrator_card(
        f,
        service_row[2],
        "Orchestrator Loops",
        if running {
            "🟢 RUNNING"
        } else {
            "⏸️ STOPPED"
        },
        if running { Color::Green } else { Color::Yellow },
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
        Color::Green
    } else if poll_secs < 15 {
        Color::Yellow
    } else {
        Color::Red
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
        &format!(
            "COT: {}, WL: {}, Cache: {}",
            cot_count, watch_count, cache_size
        ),
        Color::Cyan,
        "COT entries | Watchlist items | Cache entries",
    );

    // Overall health card — considers ServiceManager status + orchestrator
    let all_ok = kronos_status == "Healthy"
        && llm_status == "Healthy"
        && broker_status == "Healthy"
        && orch_up
        && running;
    let has_issues = kronos_status == "Down" || llm_status == "Down" || broker_status == "Down";
    let (overall_label, overall_color, overall_desc) = if all_ok {
        ("ALL SYSTEMS OK", Color::Green, "All services operational")
    } else if has_issues {
        (
            "SERVICE DOWN",
            Color::Red,
            "A critical service is unreachable",
        )
    } else {
        (
            "SOME ISSUES",
            Color::Yellow,
            "Check individual service status",
        )
    };
    render_stat_card(
        f,
        stats_row[2],
        "✅ Overall Health",
        overall_label,
        overall_color,
        overall_desc,
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

    // LLM model info
    detail_lines.push(Line::from(vec![
        Span::styled("  LLM Model:     ", Style::default().fg(Color::DarkGray)),
        Span::styled(model, Style::default().fg(Color::Cyan)),
    ]));

    // LLM endpoint
    detail_lines.push(Line::from(vec![
        Span::styled("  LLM Endpoint:  ", Style::default().fg(Color::DarkGray)),
        Span::styled(&llm_endpoint, Style::default().fg(Color::Cyan)),
    ]));

    // LLM latency + uptime
    let llm_detail = format!(
        "{} avg | {} checks | {}% uptime",
        format_latency(llm_latency),
        services_json
            .and_then(|s| llm_key
                .as_ref()
                .and_then(|k| service_u64(s, k, "checks_total")))
            .map(|c| c.to_string())
            .unwrap_or_else(|| "?".to_string()),
        llm_uptime
            .map(|p| format!("{:.0}", p))
            .unwrap_or_else(|| "?".to_string()),
    );
    detail_lines.push(Line::from(vec![
        Span::styled("  LLM Health:    ", Style::default().fg(Color::DarkGray)),
        Span::styled(
            &llm_detail,
            Style::default().fg(uptime_color(llm_uptime.unwrap_or(0.0))),
        ),
    ]));

    // LLM sparkline label
    if !llm_history.is_empty() {
        let llm_spark_str: String = llm_history
            .iter()
            .map(|v| format!("{}", v))
            .collect::<Vec<_>>()
            .join(" ");
        detail_lines.push(Line::from(vec![
            Span::styled("  LLM Latency:   ", Style::default().fg(Color::DarkGray)),
            Span::styled(llm_spark_str, Style::default().fg(Color::Green)),
            Span::styled(" ms (last 10)", Style::default().fg(Color::DarkGray)),
        ]));
    }

    // Kronos endpoint
    detail_lines.push(Line::from(vec![
        Span::styled("  Kronos Endpt:  ", Style::default().fg(Color::DarkGray)),
        Span::styled(&kronos_endpoint, Style::default().fg(Color::Cyan)),
    ]));

    // Kronos latency + uptime
    let kronos_detail = format!(
        "{} avg | {} checks | {}% uptime",
        format_latency(kronos_latency),
        services_json
            .and_then(|s| service_u64(s, "kronos", "checks_total"))
            .map(|c| c.to_string())
            .unwrap_or_else(|| "?".to_string()),
        kronos_uptime
            .map(|p| format!("{:.0}", p))
            .unwrap_or_else(|| "?".to_string()),
    );
    detail_lines.push(Line::from(vec![
        Span::styled("  Kronos Health: ", Style::default().fg(Color::DarkGray)),
        Span::styled(
            &kronos_detail,
            Style::default().fg(uptime_color(kronos_uptime.unwrap_or(0.0))),
        ),
    ]));

    // Broker endpoint
    detail_lines.push(Line::from(vec![
        Span::styled("  Broker:        ", Style::default().fg(Color::DarkGray)),
        Span::styled(broker_name, Style::default().fg(Color::Cyan)),
    ]));
    detail_lines.push(Line::from(vec![
        Span::styled("  Broker Endpt:  ", Style::default().fg(Color::DarkGray)),
        Span::styled(&broker_endpoint, Style::default().fg(Color::Cyan)),
    ]));

    // Broker latency + uptime
    let broker_detail = format!(
        "{} avg | {} checks | {}% uptime",
        format_latency(broker_latency),
        broker_key
            .as_ref()
            .and_then(|key| services_json.and_then(|s| service_u64(s, key, "checks_total")))
            .map(|c| c.to_string())
            .unwrap_or_else(|| "?".to_string()),
        broker_uptime
            .map(|p| format!("{:.0}", p))
            .unwrap_or_else(|| "?".to_string()),
    );
    detail_lines.push(Line::from(vec![
        Span::styled("  Broker Health: ", Style::default().fg(Color::DarkGray)),
        Span::styled(
            &broker_detail,
            Style::default().fg(uptime_color(broker_uptime.unwrap_or(0.0))),
        ),
    ]));

    // Policy cache
    detail_lines.push(Line::from(vec![
        Span::styled("  Policy Cache:  ", Style::default().fg(Color::DarkGray)),
        Span::styled(
            format!("{} entries", cache_size),
            Style::default().fg(Color::Cyan),
        ),
    ]));

    // Watchlist info
    detail_lines.push(Line::from(vec![
        Span::styled("  Watchlist:     ", Style::default().fg(Color::DarkGray)),
        Span::styled(
            format!("{} symbols: {}", watch_count, app.watchlist.join(", ")),
            Style::default().fg(Color::Cyan),
        ),
    ]));

    // COT history count
    detail_lines.push(Line::from(vec![
        Span::styled("  COT History:   ", Style::default().fg(Color::DarkGray)),
        Span::styled(
            format!("{} entries", cot_count),
            Style::default().fg(Color::Cyan),
        ),
    ]));

    detail_lines.push(Line::from(""));
    detail_lines.push(Line::from(Span::styled(
        "  Press ? for keyboard shortcuts",
        Style::default().fg(Color::DarkGray),
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

/// Render a service status card with rich status + live sparkline.
///
/// Layout (inside bordered card):
///   Line 0: Status emoji + label (bold, colored)
///   Line 1: Middle line (latency avg + uptime %)
///   Lines 2-4: Sparkline (last 10 response times)
///   Line 5: Bottom line (endpoint / model info)
#[allow(clippy::too_many_arguments)]
fn render_service_card(
    f: &mut Frame,
    area: Rect,
    title: &str,
    status_display: &str,
    status_color: Color,
    middle_line: &str,
    history: &[u64],
    history_max: u64,
    sparkline_bg: Color,
    sparkline_fg: Color,
    bottom_line: &str,
) {
    let block = Block::default()
        .title(Span::styled(title, Style::default().fg(THEME.brand)))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(THEME.border));
    let inner = block.inner(area);
    f.render_widget(block, area);

    // Four rows inside the card
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2), // Status line
            Constraint::Length(1), // Middle line (latency/uptime)
            Constraint::Length(3), // Sparkline area
            Constraint::Min(1),    // Bottom line (endpoint/model)
        ])
        .split(inner);

    // Status line — emoji + status label (bold, colored)
    let status_para = Paragraph::new(Line::from(Span::styled(
        status_display,
        Style::default()
            .fg(status_color)
            .add_modifier(Modifier::BOLD),
    )))
    .alignment(Alignment::Center);
    f.render_widget(status_para, chunks[0]);

    // Middle line — latency + uptime
    let mid_para = Paragraph::new(Line::from(Span::styled(
        middle_line,
        Style::default().fg(Color::DarkGray),
    )))
    .alignment(Alignment::Center);
    f.render_widget(mid_para, chunks[1]);

    // Sparkline — last 10 response times
    if history.is_empty() {
        let empty_para = Paragraph::new(Line::from(Span::styled(
            "  no data yet",
            Style::default().fg(Color::DarkGray),
        )))
        .alignment(Alignment::Center);
        f.render_widget(empty_para, chunks[2]);
    } else {
        let sparkline = ratatui::widgets::Sparkline::default()
            .block(Block::default().borders(Borders::NONE))
            .data(history)
            .max(history_max)
            .style(Style::default().fg(sparkline_fg).bg(sparkline_bg));
        f.render_widget(sparkline, chunks[2]);
    }

    // Bottom line — endpoint / model info (truncated for 4-column layout)
    // Unicode-safe: char_indices ensures we never split a multi-byte codepoint.
    let max_chars = (area.width as usize / 4).clamp(12, 35);
    let display = if bottom_line.chars().count() > max_chars {
        let cut = max_chars.saturating_sub(3);
        let byte_pos = bottom_line
            .char_indices()
            .nth(cut)
            .map(|(i, _)| i)
            .unwrap_or(bottom_line.len());
        format!("{}...", &bottom_line[..byte_pos])
    } else {
        bottom_line.to_string()
    };
    let bottom_para = Paragraph::new(Line::from(Span::styled(
        display,
        Style::default().fg(Color::DarkGray),
    )))
    .alignment(Alignment::Center);
    f.render_widget(bottom_para, chunks[3]);
}

/// Render the orchestrator card (no sparkline since it's not in ServiceManager).
fn render_orchestrator_card(
    f: &mut Frame,
    area: Rect,
    title: &str,
    status_display: &str,
    status_color: Color,
    subtitle: &str,
) {
    let block = Block::default()
        .title(Span::styled(title, Style::default().fg(THEME.brand)))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(THEME.border));
    let inner = block.inner(area);
    f.render_widget(block, area);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2),
            Constraint::Length(1),
            Constraint::Min(1),
        ])
        .split(inner);

    let status_para = Paragraph::new(Line::from(Span::styled(
        status_display,
        Style::default()
            .fg(status_color)
            .add_modifier(Modifier::BOLD),
    )))
    .alignment(Alignment::Center);
    f.render_widget(status_para, chunks[0]);

    let sub_para = Paragraph::new(Line::from(Span::styled(
        subtitle,
        Style::default().fg(Color::DarkGray),
    )))
    .alignment(Alignment::Center);
    f.render_widget(sub_para, chunks[2]);
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
        .title(Span::styled(title, Style::default().fg(Color::DarkGray)))
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
        Style::default().fg(Color::DarkGray),
    )))
    .alignment(Alignment::Center);
    f.render_widget(sub_para, chunks[1]);
}
