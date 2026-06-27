//! Dashboard tab — Card-based layout with Gauge progress bars and mini trendlines.
//!
//! All four sparkline cards include configurable SMA + EMA overlay lines,
//! crossover detection for SMA-3 / SMA-5, Rate-of-Change (ROC) momentum on
//! the P&L card, and maximum drawdown annotations on the EQUITY gauge card.

use crate::prelude::*;
use crate::AppState;

pub fn render_dashboard(f: &mut Frame, area: Rect, app: &mut AppState) {
    // Clear clickable areas before re-rendering so stale rects don't persist
    app.pipeline_layer_areas.clear();

    let status = app.status.as_ref();

    let equity = status
        .and_then(|s| s.get("total_equity"))
        .and_then(|v| v.as_f64())
        .unwrap_or(100_000.0);
    let cash = status
        .and_then(|s| s.get("cash_balance"))
        .and_then(|v| v.as_f64())
        .unwrap_or(100_000.0);
    let pnl = status
        .and_then(|s| s.get("daily_pnl"))
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0);
    let trades = status
        .and_then(|s| s.get("total_trades_today"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let wins = status
        .and_then(|s| s.get("winning_trades_today"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let losses = status
        .and_then(|s| s.get("losing_trades_today"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let open_positions = status
        .and_then(|s| s.get("open_positions_count"))
        .and_then(|v| v.as_u64())
        .or_else(|| {
            status
                .and_then(|s| s.get("open_positions"))
                .and_then(|v| v.as_array())
                .map(|a| a.len() as u64)
        })
        .unwrap_or(0);

    let pnl_color = if pnl >= 0.0 {
        THEME.positive
    } else {
        THEME.negative
    };
    let win_rate = if trades > 0 {
        wins as f64 / trades as f64 * 100.0
    } else {
        0.0
    };
    let equity_used_pct = if equity > 0.0 {
        ((equity - cash) / equity * 100.0).min(100.0) as u16
    } else {
        0
    };
    let cash_pct = if equity > 0.0 {
        (cash / equity * 100.0) as u16
    } else {
        100
    };

    // Live trend history from WebSocket portfolio snapshots (~1 min cadence)
    let eq_history = &app.equity_history;
    let pnl_history = &app.pnl_history;
    let win_rate_history = &app.win_rate_history;
    let loss_streak_history = &app.consecutive_losses_history;

    // ── Compute drawdown from equity history (for EQUITY gauge card) ────────
    let drawdown_pct = compute_drawdown(eq_history);
    let equity_subtitle = if let Some(dd) = drawdown_pct {
        format!("used  |  DD: {:.1}%", dd * 100.0)
    } else {
        "used".to_string()
    };

    let now = std::time::Instant::now();
    let is_loading = status.is_none();

    // ── Extract ServiceManager status for the service strip ────────────────
    let services_json = app.service_status.as_ref().and_then(|s| s.get("services"));
    let health = app.health.as_ref();

    // Helper: extract string field from a service
    let svc_str = |name: &str, field: &str| -> String {
        services_json
            .and_then(|s| s.get(name))
            .and_then(|s| s.get(field))
            .and_then(|v| v.as_str())
            .map(String::from)
            .unwrap_or_else(|| {
                if health
                    .and_then(|h| h.get(name))
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false)
                {
                    "Healthy".to_string()
                } else {
                    "Unknown".to_string()
                }
            })
    };
    let svc_f64 = |name: &str, field: &str| -> Option<f64> {
        services_json
            .and_then(|s| s.get(name))
            .and_then(|s| s.get(field))
            .and_then(|v| v.as_f64())
    };

    let k_status = svc_str("kronos", "status");
    let k_latency = svc_f64("kronos", "response_time_avg_ms");
    let llm_key = services_json.and_then(|s| {
        s.as_object()
            .and_then(|obj| obj.keys().find(|k| k.contains("llm")).cloned())
    });
    let l_status = llm_key
        .as_ref()
        .map(|key| svc_str(key, "status"))
        .unwrap_or_else(|| {
            if health
                .and_then(|h| h.get("llm"))
                .and_then(|v| v.as_bool())
                .unwrap_or(false)
            {
                "Healthy".to_string()
            } else {
                "Unknown".to_string()
            }
        });
    let l_latency = llm_key
        .as_ref()
        .and_then(|key| svc_f64(key, "response_time_avg_ms"));

    // Broker service — find any key containing "broker"
    let broker_key = services_json.and_then(|s| {
        s.as_object()
            .and_then(|obj| obj.keys().find(|k| k.contains("broker")).cloned())
    });
    let b_status = broker_key
        .as_ref()
        .map(|key| svc_str(key, "status"))
        .unwrap_or_else(|| "Unknown".to_string());
    let b_latency = broker_key
        .as_ref()
        .and_then(|key| svc_f64(key, "response_time_avg_ms"));
    let _broker_name = broker_key
        .as_ref()
        .map(|k| k.replace("broker_", "").replace('_', " "))
        .map(|n| {
            n.chars()
                .next()
                .map(|c| c.to_uppercase().to_string() + &n[1..])
                .unwrap_or(n)
        })
        .unwrap_or_else(|| "Broker".to_string());

    // ── Layout ─────────────────────────────────────────────────────────────
    let (top_row_h, bot_row_h, spark_h, min_bottom) = if area.height < 30 {
        (4, 4, 0, 4)
    } else if area.height < 38 {
        (5, 5, 5, 6)
    } else {
        (6, 6, 7, 6)
    };

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2),         // Title (line 1) + service status strip (line 2)
            Constraint::Length(1),         // gap
            Constraint::Length(top_row_h), // Top row
            Constraint::Length(if top_row_h > 0 { 1 } else { 0 }), // gap
            Constraint::Length(bot_row_h), // Bottom row
            Constraint::Length(if bot_row_h > 0 { 1 } else { 0 }), // gap
            Constraint::Length(3),         // Stats bar
            Constraint::Length(spark_h),   // Four sparkline cards
            Constraint::Length(if spark_h > 0 { 1 } else { 0 }), // gap
            Constraint::Min(min_bottom),   // 5-Layer Pipeline Flow + Agent Comms + Judge
        ])
        .split(area);

    // ── Title with loading indicator + service status strip ────────────────
    let title_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Length(1)])
        .split(chunks[0]);

    let spinner = if is_loading { loading_spinner(now) } else { "" };
    let title = Paragraph::new(Line::from(Span::styled(
        format!("PORTFOLIO DASHBOARD {}", spinner),
        Style::default()
            .fg(THEME.brand)
            .add_modifier(Modifier::BOLD),
    )));
    f.render_widget(title, title_chunks[0]);

    // Service status strip — compact one-liner with emoji + latency
    fn status_emoji(s: &str) -> &'static str {
        match s {
            "Healthy" => "🟢",
            "Degraded" => "🟡",
            "Down" => "🔴",
            _ => "❓",
        }
    }
    fn fmt_latency(ms: Option<f64>) -> String {
        ms.map(|v| {
            if v < 1.0 {
                "<1ms".into()
            } else if v < 1000.0 {
                format!("{:.0}ms", v)
            } else {
                format!("{:.1}s", v / 1000.0)
            }
        })
        .unwrap_or_else(|| "?ms".into())
    }

    let k_emoji = status_emoji(&k_status);
    let l_emoji = status_emoji(&l_status);
    let b_emoji = status_emoji(&b_status);
    let k_lat = fmt_latency(k_latency);
    let l_lat = fmt_latency(l_latency);
    let b_lat = fmt_latency(b_latency);

    let all_healthy = k_status == "Healthy" && l_status == "Healthy" && b_status == "Healthy";
    let any_down = k_status == "Down" || l_status == "Down" || b_status == "Down";
    let overall = if all_healthy {
        ("✅ All Healthy", THEME.positive)
    } else if any_down {
        ("🔴 Service Down", THEME.negative)
    } else {
        ("🟡 Issues Detected", THEME.warning)
    };

    let k_part = format!(
        "{} Kronos {}{}",
        k_emoji,
        k_lat,
        if k_status == "Healthy" {
            String::new()
        } else {
            format!(" {}", k_status)
        },
    );
    let l_part = format!(
        "{} LLM {}{}",
        l_emoji,
        l_lat,
        if l_status == "Healthy" {
            String::new()
        } else {
            format!(" {}", l_status)
        },
    );
    let b_part = format!(
        "{} {} {}{}",
        b_emoji,
        "Brkr",
        b_lat,
        if b_status == "Healthy" {
            String::new()
        } else {
            format!(" {}", b_status)
        },
    );
    let status_line = format!("{} | {} | {} | {}", k_part, l_part, b_part, overall.0);

    let status_para = Paragraph::new(Line::from(Span::styled(
        status_line.trim().to_string(),
        Style::default().fg(overall.1),
    )));
    f.render_widget(status_para, title_chunks[1]);

    // ── Top row: EQUITY + CASH ────────────────────────────────────────────
    let top_row = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Ratio(1, 2), Constraint::Ratio(1, 2)])
        .split(chunks[2]);

    render_metric_card(
        f,
        top_row[0],
        "EQUITY",
        &format!("{:.2}", equity),
        equity_used_pct,
        THEME.info,
        &equity_subtitle,
        eq_history,
    );
    render_metric_card(
        f,
        top_row[1],
        "CASH",
        &format!("{:.2}", cash),
        cash_pct,
        THEME.positive,
        "free",
        &[],
    );

    // ── Bottom row: P&L + WIN RATE ────────────────────────────────────────
    let bottom_row = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Ratio(1, 2), Constraint::Ratio(1, 2)])
        .split(chunks[4]);

    let pnl_pct = if equity > 0.0 {
        ((pnl / equity) * 100.0) as u16
    } else {
        0
    };
    render_metric_card(
        f,
        bottom_row[0],
        "DAILY P&L",
        &format!("{:+.2}", pnl),
        pnl_pct,
        pnl_color,
        "today",
        pnl_history,
    );
    render_metric_card(
        f,
        bottom_row[1],
        "WIN RATE",
        &format!("{:.1}%", win_rate),
        win_rate as u16,
        if win_rate >= 50.0 {
            THEME.positive
        } else if win_rate > 0.0 {
            THEME.warning
        } else {
            THEME.muted
        },
        &format!("{}/{} wins", wins, trades),
        &[],
    );

    // ── Stats bar ─────────────────────────────────────────────────────────
    let stats_text = format!(
        "  POSITIONS: {}  |  TRADES: {}  |  WINS: {}  |  LOSSES: {}  |  EQUITY USED: {}%  |  DD: {}",
        open_positions,
        trades,
        wins,
        losses,
        equity_used_pct,
        drawdown_pct
            .map(|d| format!("{:.1}%", d * 100.0))
            .unwrap_or_else(|| "—".to_string()),
    );
    let stats_para = Paragraph::new(Line::from(Span::styled(
        stats_text,
        Style::default().fg(THEME.highlight),
    )))
    .block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(THEME.border)),
    );
    f.render_widget(stats_para, chunks[6]);

    // ── Four Sparkline Cards ──────────────────────────────────────────────
    if spark_h > 0 {
        let sparkline_row = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Ratio(1, 4),
                Constraint::Ratio(1, 4),
                Constraint::Ratio(1, 4),
                Constraint::Ratio(1, 4),
            ])
            .split(chunks[7]);

        app.sparkline_card_areas = sparkline_row.to_vec();

        // P&L Trend — SMA-3, SMA-5 + ROC-20 momentum
        render_sparkline_with_sma(
            f,
            sparkline_row[0],
            pnl_history,
            "P&L Trend",
            false,
            &[3, 5],
            &[],
            Some(20),
            false,
        );

        // Equity Trend — SMA-3, SMA-5, SMA-10 + EMA-10 + crossover signals
        render_sparkline_with_sma(
            f,
            sparkline_row[1],
            eq_history,
            "Equity Trend",
            false,
            &[3, 5, 10],
            &[10],
            None,
            true,
        );

        // Win Rate — SMA-3, SMA-5 (percentage mode)
        render_sparkline_with_sma(
            f,
            sparkline_row[2],
            win_rate_history,
            "Win Rate",
            true,
            &[3, 5],
            &[],
            None,
            false,
        );

        // Loss Streak — SMA-3, SMA-5
        render_sparkline_with_sma(
            f,
            sparkline_row[3],
            loss_streak_history,
            "Loss Streak",
            false,
            &[3, 5],
            &[],
            None,
            false,
        );
    } else {
        app.sparkline_card_areas.clear();
    }

    // ── 5-Layer Pipeline Flow + Agent Comms + Judge Panel + Tech Indicators ──
    let bottom_section = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Ratio(3, 10), // 5-Layer Flow
            Constraint::Ratio(3, 10), // Agent Communication
            Constraint::Ratio(2, 10), // Judge Decision
            Constraint::Ratio(2, 10), // Tech Indicators
        ])
        .split(chunks[9]);

    render_pipeline_flow(f, bottom_section[0], app);
    render_agent_comms(f, bottom_section[1], app);
    render_judge_panel(f, bottom_section[2], app);
    render_indicators_panel(f, bottom_section[3], app);
}

// ── 5-Layer Pipeline Flow Visualization ─────────────────────────────────────

/// Render the 5-layer pipeline flow as a horizontal diagram with status indicators.
/// Each layer shows its name, status, and a brief description.
fn render_pipeline_flow(f: &mut Frame, area: Rect, app: &mut AppState) {
    let block = Block::default()
        .title(Span::styled(
            "⚙️ 5-LAYER PIPELINE",
            Style::default()
                .fg(THEME.brand)
                .add_modifier(Modifier::BOLD),
        ))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(THEME.border));

    let inner = block.inner(area);
    f.render_widget(block, area);

    // Determine layer statuses from latest COT entries
    let cot = &app.cot;
    let layer_statuses = determine_layer_statuses(cot);

    let layer_data = [
        ("L1", "Gate", "Rules", layer_statuses[0]),
        ("L2", "Ident", "Data", layer_statuses[1]),
        ("L3", "Debate", "12v11", layer_statuses[2]),
        ("L4", "Judge", "Quality", layer_statuses[3]),
        ("L5", "Exec", "Trade", layer_statuses[4]),
    ];

    // Layout: 5 layer boxes with arrows between them
    let mut constraints = Vec::new();
    for i in 0..5 {
        constraints.push(Constraint::Ratio(1, 9));
        if i < 4 {
            constraints.push(Constraint::Length(3)); // arrow gap
        }
    }

    let flow_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints(constraints)
        .split(inner);

    for (i, (id, name, desc, status)) in layer_data.iter().copied().enumerate() {
        let (color, status_text) = match status {
            LayerStatus::Passed => (THEME.positive, "● PASS"),
            LayerStatus::Blocked => (THEME.negative, "● BLOCK"),
            LayerStatus::Running => (THEME.warning, "◌ RUN"),
            LayerStatus::Skipped => (THEME.muted, "○ SKIP"),
            LayerStatus::Pending => (THEME.muted, "○ WAIT"),
        };

        let layer_block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(color));
        let layer_inner = layer_block.inner(flow_chunks[i * 2]);
        f.render_widget(layer_block, flow_chunks[i * 2]);

        let lines = vec![
            Line::from(Span::styled(
                format!("{} {}", id, name),
                Style::default().fg(color).add_modifier(Modifier::BOLD),
            )),
            Line::from(Span::styled(status_text, Style::default().fg(color))),
            Line::from(Span::styled(desc, Style::default().fg(THEME.muted))),
        ];
        let p = Paragraph::new(lines).alignment(Alignment::Center);
        f.render_widget(p, layer_inner);

        // Store clickable area for mouse navigation
        app.pipeline_layer_areas.push(flow_chunks[i * 2]);

        // Render arrow between layers
        if i < 4 {
            let arrow = Paragraph::new(Line::from(Span::styled(
                " → ",
                Style::default().fg(THEME.highlight),
            )))
            .alignment(Alignment::Center);
            f.render_widget(arrow, flow_chunks[i * 2 + 1]);
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
enum LayerStatus {
    Passed,
    Blocked,
    Running,
    Skipped,
    Pending,
}

/// Determine the status of each pipeline layer from COT entries.
fn determine_layer_statuses(cot: &[serde_json::Value]) -> [LayerStatus; 5] {
    let mut statuses = [LayerStatus::Pending; 5];

    // Look at the most recent COT entries to determine layer statuses
    for entry in cot.iter().rev().take(20) {
        let agent = entry.get("agent").and_then(|a| a.as_str()).unwrap_or("");
        let action = entry.get("action").and_then(|a| a.as_str()).unwrap_or("");

        match agent {
            a if a.contains("HardRules") || a == "Gate" => {
                if statuses[0] == LayerStatus::Pending {
                    statuses[0] = if action == "BLOCKED" || action == "REJECT" {
                        LayerStatus::Blocked
                    } else if action == "PASSED" || action == "PASS" {
                        LayerStatus::Passed
                    } else {
                        LayerStatus::Running
                    };
                }
            }
            a if a.contains("Identifier") || a.contains("Verifier") => {
                if statuses[1] == LayerStatus::Pending {
                    statuses[1] = if action == "ANALYZED" || action == "PASS" {
                        LayerStatus::Passed
                    } else if action == "SKIP" {
                        LayerStatus::Skipped
                    } else {
                        LayerStatus::Running
                    };
                }
            }
            a if a.contains("Debate") => {
                if statuses[2] == LayerStatus::Pending {
                    statuses[2] = if action == "BUY" || action == "SELL" || action == "HOLD" {
                        LayerStatus::Passed
                    } else {
                        LayerStatus::Running
                    };
                }
            }
            a if a.contains("Judge") => {
                if statuses[3] == LayerStatus::Pending {
                    statuses[3] = if action == "APPROVE" {
                        LayerStatus::Passed
                    } else if action == "VETO" {
                        LayerStatus::Blocked
                    } else {
                        LayerStatus::Running
                    };
                }
            }
            a if (a.contains("Execution") || a.contains("Exec"))
                && statuses[4] == LayerStatus::Pending =>
            {
                statuses[4] = if action == "EXECUTED" || action == "FILLED" || action == "LOGGED" {
                    LayerStatus::Passed
                } else if action == "HOLD" || action == "SKIPPED" {
                    LayerStatus::Skipped
                } else if action == "REJECTED"
                    || action == "BLOCKED"
                    || action == "FAILED"
                    || action == "EXECUTION_FAILED"
                {
                    LayerStatus::Blocked
                } else {
                    LayerStatus::Running
                };
            }
            _ => {}
        }
    }

    statuses
}

// ── Agent Communication Panel ───────────────────────────────────────────────

/// Render a panel showing recent agent-to-agent messages from the COT log.
fn render_agent_comms(f: &mut Frame, area: Rect, app: &AppState) {
    let block = Block::default()
        .title(Span::styled(
            "💬 AGENT COMMUNICATIONS",
            Style::default().fg(THEME.info).add_modifier(Modifier::BOLD),
        ))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(THEME.border));

    let inner = block.inner(area);
    f.render_widget(block, area);

    let mut lines = vec![];

    // Show the last 12 COT entries (most recent first)
    let recent: Vec<_> = app.cot.iter().rev().take(12).collect();

    if recent.is_empty() {
        lines.push(Line::from(Span::styled(
            "  Waiting for agent communications...",
            Style::default().fg(THEME.muted),
        )));
    } else {
        for entry in &recent {
            let agent = entry.get("agent").and_then(|a| a.as_str()).unwrap_or("?");
            let action = entry.get("action").and_then(|a| a.as_str()).unwrap_or("?");
            let message = entry
                .get("message")
                .or_else(|| entry.get("reason"))
                .or_else(|| entry.get("input"))
                .and_then(|m| m.as_str())
                .unwrap_or("");

            let agent_color = match action {
                "PASS" | "PASSED" | "APPROVE" | "EXECUTED" | "FILLED" | "ANALYZED" | "LOGGED"
                | "TRADE_EXECUTED" => THEME.positive,
                "BLOCKED" | "REJECT" | "REJECTED" | "VETO" | "FAILED" | "EXECUTION_FAILED" => {
                    THEME.negative
                }
                "HOLD" | "SKIP" | "SKIPPED" => THEME.muted,
                "INFO" => THEME.info,
                _ => THEME.highlight,
            };

            // Unicode-safe truncation (avoids panic on multi-byte chars like em-dash —)
            let max_msg_len = inner.width.saturating_sub(20) as usize;
            let truncated_msg = safe_truncate(message, max_msg_len);

            lines.push(Line::from(vec![
                Span::styled(
                    format!("  {:<10}", agent),
                    Style::default()
                        .fg(agent_color)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(format!(" {:<8}", action), Style::default().fg(agent_color)),
                Span::styled(truncated_msg, Style::default().fg(THEME.muted)),
            ]));
        }
    }

    let p = Paragraph::new(lines).wrap(Wrap { trim: true });
    f.render_widget(p, inner);
}

// ── Judge Decision Panel ────────────────────────────────────────────────────

/// Render a panel showing the latest judge decision with scores and reasoning.
fn render_judge_panel(f: &mut Frame, area: Rect, app: &AppState) {
    let block = Block::default()
        .title(Span::styled(
            "⚖️ JUDGE",
            Style::default()
                .fg(THEME.warning)
                .add_modifier(Modifier::BOLD),
        ))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(THEME.border));

    let inner = block.inner(area);
    f.render_widget(block, area);

    // Find the latest Judge COT entry
    let judge_entry = app.cot.iter().rev().find(|e| {
        e.get("agent")
            .and_then(|a| a.as_str())
            .map(|a| a.contains("Judge") || a.contains("DebateLayer"))
            .unwrap_or(false)
    });

    let mut lines = vec![];

    if let Some(entry) = judge_entry {
        let action = entry.get("action").and_then(|a| a.as_str()).unwrap_or("?");
        let message = entry
            .get("message")
            .or_else(|| entry.get("reason"))
            .or_else(|| entry.get("input"))
            .and_then(|m| m.as_str())
            .unwrap_or("");
        let confidence = entry.get("confidence").and_then(|c| c.as_f64());

        let action_color = match action {
            "APPROVE" | "BUY" | "SELL" => THEME.positive,
            "HOLD" => THEME.muted,
            "VETO" => THEME.negative,
            _ => THEME.highlight,
        };

        lines.push(Line::from(Span::styled(
            "  Verdict:",
            Style::default().fg(THEME.muted),
        )));
        lines.push(Line::from(Span::styled(
            format!("    {}", action),
            Style::default()
                .fg(action_color)
                .add_modifier(Modifier::BOLD),
        )));

        if let Some(conf) = confidence {
            lines.push(Line::from(""));
            lines.push(Line::from(vec![
                Span::styled("  Confidence: ", Style::default().fg(THEME.muted)),
                Span::styled(
                    format!("{:.0}%", conf * 100.0),
                    Style::default().fg(THEME.highlight),
                ),
            ]));
        }

        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "  Reasoning:",
            Style::default().fg(THEME.muted),
        )));
        // Unicode-safe truncation (avoids panic on multi-byte chars like em-dash —)
        let max_reason_len = inner.width.saturating_sub(4) as usize;
        let truncated = safe_truncate(message, max_reason_len);
        lines.push(Line::from(Span::styled(
            format!("    {}", truncated),
            Style::default().fg(THEME.highlight),
        )));
    } else {
        lines.push(Line::from(Span::styled(
            "  No judge decision yet.",
            Style::default().fg(THEME.muted),
        )));
        lines.push(Line::from(Span::styled(
            "  Waiting for debate...",
            Style::default().fg(THEME.muted),
        )));
    }

    let p = Paragraph::new(lines).wrap(Wrap { trim: true });
    f.render_widget(p, inner);
}

// ── Helper Functions ────────────────────────────────────────────────────────

/// Unicode-safe string truncation.
///
/// Truncates `s` to at most `max_chars` **characters** (not bytes), then appends
/// `"..."` if truncation occurred. This prevents panics when the string contains
/// multi-byte characters such as em-dash `—` (3 bytes) or other Unicode code-points.
///
/// Unlike `&s[..n]` (byte-index slice), this method always lands on a valid
/// UTF-8 char boundary.
fn safe_truncate(s: &str, max_chars: usize) -> String {
    if max_chars < 3 {
        return "...".to_string();
    }
    // Use char_indices so we find byte positions of char boundaries
    let char_count = s.chars().count();
    if char_count <= max_chars {
        return s.to_string();
    }
    // Find the byte position of the (max_chars - 3)th character boundary
    let cut = max_chars.saturating_sub(3);
    let byte_pos = s.char_indices().nth(cut).map(|(i, _)| i).unwrap_or(s.len());
    format!("{}...", &s[..byte_pos])
}

/// Compute a simple moving average.
fn compute_sma(data: &[f64], period: usize) -> Vec<f64> {
    if data.is_empty() {
        return Vec::new();
    }
    data.iter()
        .enumerate()
        .map(|(i, _)| {
            let window = if i + 1 < period { i + 1 } else { period };
            data[i + 1 - window..=i].iter().sum::<f64>() / window as f64
        })
        .collect()
}

/// Compute an exponential moving average.
/// Smoothing factor α = 2 / (period + 1).  Starts with SMA for the initial seed.
fn compute_ema(data: &[f64], period: usize) -> Vec<f64> {
    if data.is_empty() || period == 0 {
        return Vec::new();
    }
    let alpha = 2.0 / (period as f64 + 1.0);
    let mut ema = Vec::with_capacity(data.len());
    // Seed: SMA of the first `period` values
    let seed_n = period.min(data.len());
    let seed = data[..seed_n].iter().sum::<f64>() / seed_n as f64;
    for (i, &val) in data.iter().enumerate() {
        if i == 0 {
            ema.push(seed);
        } else {
            let prev = ema[i - 1];
            ema.push(val * alpha + prev * (1.0 - alpha));
        }
    }
    ema
}

/// Compute Rate-of-Change (ROC) as percentage: (current - past) / past * 100.
/// Returns a zero-centered vector of the same length (first `period` entries are 0).
fn compute_roc(data: &[f64], period: usize) -> Vec<f64> {
    if data.len() <= period || period == 0 {
        return vec![0.0; data.len()];
    }
    let mut roc = Vec::with_capacity(data.len());
    for i in 0..data.len() {
        if i < period {
            roc.push(0.0);
        } else {
            let past = data[i - period];
            if past.abs() > 1e-12 {
                roc.push((data[i] - past) / past * 100.0);
            } else {
                roc.push(0.0);
            }
        }
    }
    roc
}

/// Compute the maximum drawdown from peak as a positive fraction
/// (0.05 = 5% drawdown).  Returns None for very short series.
fn compute_drawdown(data: &[f64]) -> Option<f64> {
    if data.len() < 2 {
        return None;
    }
    let mut peak = data[0];
    let mut max_dd = 0.0_f64;
    for &v in data.iter().skip(1) {
        if v > peak {
            peak = v;
        }
        let dd = (peak - v) / peak.abs().max(1.0);
        if dd > max_dd {
            max_dd = dd;
        }
    }
    Some(max_dd)
}

/// Detect the latest SMA-3 / SMA-5 crossover direction.
fn detect_sma_crossover(sma3: &[f64], sma5: &[f64]) -> Option<bool> {
    if sma3.len() < 2 || sma5.len() < 2 {
        return None;
    }
    let (p3, c3) = (sma3[sma3.len() - 2], sma3[sma3.len() - 1]);
    let (p5, c5) = (sma5[sma5.len() - 2], sma5[sma5.len() - 1]);
    if p3 < p5 && c3 >= c5 {
        Some(true)
    } else if p3 > p5 && c3 <= c5 {
        Some(false)
    } else {
        None
    }
}

// ── Formatting Helpers ──────────────────────────────────────────────────────

fn fmt_val(val: f64, as_pct: bool) -> String {
    if as_pct {
        format!("{:.1}%", val * 100.0)
    } else {
        format!("{:+.1}", val)
    }
}

fn fmt_label(val: f64, as_pct: bool) -> String {
    if as_pct {
        format!("{:.0}%", val * 100.0)
    } else {
        format!("{:+.1}", val)
    }
}

// ── Sparkline helpers ───────────────────────────────────────────────────────

fn sparkline_bars(vals: &[f64], min: f64, range: f64) -> String {
    if range < 0.001 || vals.is_empty() {
        return String::new();
    }
    let bars: [&str; 8] = ["▁", "▂", "▃", "▄", "▅", "▆", "▇", "█"];
    vals.iter()
        .map(|v| {
            let idx = ((v - min) / range * 7.0).round().clamp(0.0, 7.0) as usize;
            bars[idx]
        })
        .collect()
}

fn series_min_max<'a>(series: impl Iterator<Item = &'a [f64]>) -> (f64, f64, f64) {
    let mut all_min = f64::INFINITY;
    let mut all_max = f64::NEG_INFINITY;
    let mut all_len = 0;
    for s in series {
        all_len += s.len();
        for &v in s {
            if v < all_min {
                all_min = v;
            }
            if v > all_max {
                all_max = v;
            }
        }
    }
    if all_len == 0 {
        all_min = 0.0;
        all_max = 1.0;
    }
    let range = (all_max - all_min).max(0.001);
    (all_min, all_max, range)
}

// ── Main Sparkline Renderer ─────────────────────────────────────────────────

#[allow(clippy::too_many_arguments)]
/// Render a sparkline card with SMA, EMA, and optional ROC overlay lines.
///
/// - `sma_periods` — SMAs to render (shares Y-axis with the main series)
/// - `ema_periods` — EMAs to render (also shares the same Y-axis)
/// - `roc_period`   — optional ROC period; rendered on its own scaled row
/// - `show_crossovers` — when true and SMA-3/SMA-5 are both present, shows ▲/▼
fn render_sparkline_with_sma(
    f: &mut Frame,
    area: Rect,
    history: &[f64],
    label: &str,
    as_percentage: bool,
    sma_periods: &[usize],
    ema_periods: &[usize],
    roc_period: Option<usize>,
    show_crossovers: bool,
) {
    let smas: Vec<Vec<f64>> = sma_periods
        .iter()
        .map(|&p| compute_sma(history, p))
        .collect();
    let emas: Vec<Vec<f64>> = ema_periods
        .iter()
        .map(|&p| compute_ema(history, p))
        .collect();
    let roc: Option<Vec<f64>> = roc_period.map(|p| compute_roc(history, p));

    // ── Indent width (computed from max-label length) ──────────────────────
    fn calc_max_label(data: &[f64], as_pct: bool) -> String {
        if data.is_empty() {
            return "0".into();
        }
        let mx = data.iter().copied().fold(f64::NEG_INFINITY, f64::max);
        fmt_label(mx, as_pct)
    }
    let max_lbl_main = calc_max_label(history, as_percentage);
    let indent_width = max_lbl_main.len() + 2;
    let indent: String = (0..indent_width).map(|_| ' ').collect();

    // ── Building the title ────────────────────────────────────────────────
    let mut title_parts: Vec<String> = Vec::new();

    for (&p, sma) in sma_periods.iter().zip(smas.iter()) {
        let v = sma.last().copied().unwrap_or(0.0);
        title_parts.push(format!("SMA{}={}", p, fmt_val(v, as_percentage)));
    }
    for (&p, ema) in ema_periods.iter().zip(emas.iter()) {
        let v = ema.last().copied().unwrap_or(0.0);
        title_parts.push(format!("EMA{}={}", p, fmt_val(v, as_percentage)));
    }
    if let Some(ref roc_data) = roc {
        if let Some(&last) = roc_data.last() {
            title_parts.push(format!("ROC{}={:+.1}%", roc_period.unwrap_or(20), last));
        }
    }

    let crossover_char = if show_crossovers {
        let i3 = sma_periods.iter().position(|&p| p == 3);
        let i5 = sma_periods.iter().position(|&p| p == 5);
        match (i3, i5) {
            (Some(i3), Some(i5)) => match detect_sma_crossover(&smas[i3], &smas[i5]) {
                Some(true) => " ▲",
                Some(false) => " ▼",
                None => "",
            },
            _ => "",
        }
    } else {
        ""
    };

    let title_str = format!("{}  {}{}", label, title_parts.join(" "), crossover_char);
    let block = Block::default()
        .title(Span::styled(&title_str, Style::default().fg(THEME.border)))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(THEME.border));
    let inner = block.inner(area);
    f.render_widget(block, area);

    if history.len() < 2 {
        let para = Paragraph::new(Line::from(Span::styled(
            "waiting for data...",
            Style::default().fg(THEME.muted),
        )));
        f.render_widget(para, inner);
        return;
    }

    // ── Shared Y-axis for main + SMAs + EMAs ──────────────────────────────
    let shared_series: Vec<&[f64]> = std::iter::once(history)
        .chain(smas.iter().map(|v| v.as_slice()))
        .chain(emas.iter().map(|v| v.as_slice()))
        .collect();
    let (all_min, all_max, range) = series_min_max(shared_series.into_iter());
    let main_spark = sparkline_bars(history, all_min, range);
    let avg_main = history.iter().sum::<f64>() / history.len().max(1) as f64;
    let min_lbl = fmt_label(all_min, as_percentage);
    let max_lbl = fmt_label(all_max, as_percentage);

    // Colors for SMAs / EMAs
    let overlay_colors = [THEME.warning, THEME.info, Color::Magenta, Color::Blue];

    // ── Layout rows ───────────────────────────────────────────────────────
    let n_extra = roc
        .as_ref()
        .map(|r| if r.is_empty() { 0 } else { 1 })
        .unwrap_or(0);
    let n_rows = 1 + sma_periods.len() + ema_periods.len() + n_extra;
    let constraints: Vec<Constraint> = (0..n_rows).map(|_| Constraint::Length(1)).collect();
    let inner_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints(constraints)
        .split(inner);

    // ── Row 0: Main series ────────────────────────────────────────────────
    let color_main = if as_percentage {
        if avg_main >= 0.6 {
            THEME.positive
        } else if avg_main >= 0.3 {
            THEME.warning
        } else {
            THEME.negative
        }
    } else {
        if avg_main > 0.0 {
            THEME.positive
        } else if avg_main > -100.0 {
            THEME.warning
        } else {
            THEME.negative
        }
    };
    let main_line = Line::from(vec![
        Span::styled(
            &max_lbl,
            Style::default().fg(color_main).add_modifier(Modifier::BOLD),
        ),
        Span::styled("  ", Style::default().fg(THEME.muted)),
        Span::styled(
            &main_spark,
            Style::default().fg(color_main).add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!("  avg {}", fmt_val(avg_main, as_percentage)),
            Style::default().fg(THEME.muted),
        ),
    ]);
    f.render_widget(Paragraph::new(main_line), inner_chunks[0]);

    // ── SMA rows ──────────────────────────────────────────────────────────
    for (i, sma_period) in sma_periods.iter().enumerate() {
        let color = overlay_colors[i % overlay_colors.len()];
        let sp = sparkline_bars(&smas[i], all_min, range);
        let tag = format!("({})", sma_period);
        let mut spans = vec![
            Span::styled(&indent, Style::default().fg(THEME.muted)),
            Span::styled(&sp, Style::default().fg(color)),
            Span::styled(
                format!("  {}", tag),
                Style::default().fg(color).add_modifier(Modifier::DIM),
            ),
        ];
        let is_last = i == sma_periods.len() - 1 && ema_periods.is_empty() && n_extra == 0;
        if is_last {
            spans.push(Span::styled(
                format!("  {}", min_lbl),
                Style::default().fg(THEME.muted),
            ));
        }
        f.render_widget(Paragraph::new(Line::from(spans)), inner_chunks[1 + i]);
    }

    // ── EMA rows ──────────────────────────────────────────────────────────
    let ema_offset = 1 + sma_periods.len();
    for (j, ema_period) in ema_periods.iter().enumerate() {
        let color = overlay_colors[(sma_periods.len() + j) % overlay_colors.len()];
        let ep = sparkline_bars(&emas[j], all_min, range);
        let tag = format!("E({})", ema_period);
        let mut spans = vec![
            Span::styled(&indent, Style::default().fg(THEME.muted)),
            Span::styled(&ep, Style::default().fg(color)),
            Span::styled(
                format!("  {}", tag),
                Style::default().fg(color).add_modifier(Modifier::DIM),
            ),
        ];
        let is_last = j == ema_periods.len().saturating_sub(1) && n_extra == 0;
        if is_last {
            spans.push(Span::styled(
                format!("  {}", min_lbl),
                Style::default().fg(THEME.muted),
            ));
        }
        f.render_widget(
            Paragraph::new(Line::from(spans)),
            inner_chunks[ema_offset + j],
        );
    }

    // ── ROC row (separate Y-axis) ─────────────────────────────────────────
    if let Some(ref roc_data) = roc {
        if !roc_data.is_empty() {
            let roc_offset = ema_offset + ema_periods.len();
            let (roc_min, roc_max, roc_range) =
                series_min_max(std::iter::once(roc_data.as_slice()));
            let roc_spark = sparkline_bars(roc_data, roc_min, roc_range);
            let roc_max_lbl = format!("{:+.1}%", roc_max);
            let roc_min_lbl = format!("{:+.1}%", roc_min);
            let roc_color = if roc_data.last().copied().unwrap_or(0.0) >= 0.0 {
                THEME.positive
            } else {
                THEME.negative
            };

            let roc_line = Line::from(vec![
                Span::styled(
                    &roc_max_lbl,
                    Style::default().fg(roc_color).add_modifier(Modifier::BOLD),
                ),
                Span::styled("  ", Style::default().fg(THEME.muted)),
                Span::styled(&roc_spark, Style::default().fg(roc_color)),
                Span::styled(
                    format!("  {}", roc_min_lbl),
                    Style::default().fg(THEME.muted),
                ),
            ]);
            f.render_widget(Paragraph::new(roc_line), inner_chunks[roc_offset]);
        }
    }
}

// ── Gauge Card Renderer ─────────────────────────────────────────────────────

#[allow(clippy::too_many_arguments)]
fn render_metric_card(
    f: &mut Frame,
    area: Rect,
    title: &str,
    value: &str,
    percent: u16,
    accent: Color,
    subtitle: &str,
    trend_data: &[f64],
) {
    let block = Block::default()
        .title(Span::styled(
            title,
            Style::default().fg(accent).add_modifier(Modifier::BOLD),
        ))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(accent));

    let inner = block.inner(area);
    f.render_widget(block, area);

    let trend_height: u16 = if trend_data.len() >= 2 { 1 } else { 0 };
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2),            // value
            Constraint::Min(1),               // gauge
            Constraint::Length(trend_height), // mini trendline
            Constraint::Length(1),            // subtitle
        ])
        .split(inner);

    let val_para = Paragraph::new(Line::from(Span::styled(
        value,
        Style::default()
            .fg(THEME.highlight)
            .add_modifier(Modifier::BOLD),
    )))
    .alignment(Alignment::Center);
    f.render_widget(val_para, chunks[0]);

    let gauge_color = match percent {
        0..=30 => THEME.positive,
        31..=70 => THEME.warning,
        _ => THEME.negative,
    };
    let clamped = percent.clamp(0, 100);
    let gauge = Gauge::default()
        .gauge_style(Style::default().fg(gauge_color).bg(THEME.border))
        .label(format!("{}%", clamped))
        .percent(clamped);
    f.render_widget(gauge, chunks[1]);

    if trend_height > 0 {
        let min_v = trend_data.iter().copied().fold(f64::INFINITY, f64::min);
        let max_v = trend_data.iter().copied().fold(f64::NEG_INFINITY, f64::max);
        let rng = (max_v - min_v).max(0.001);
        let bars: [&str; 8] = ["▁", "▂", "▃", "▄", "▅", "▆", "▇", "█"];
        let line: String = trend_data
            .iter()
            .map(|v| {
                let idx = ((v - min_v) / rng * 7.0).round().clamp(0.0, 7.0) as usize;
                bars[idx]
            })
            .collect();
        let trend_para = Paragraph::new(Line::from(Span::styled(
            line,
            Style::default().fg(accent).add_modifier(Modifier::DIM),
        )))
        .alignment(Alignment::Center);
        f.render_widget(trend_para, chunks[2]);
    }

    let sub_para = Paragraph::new(Line::from(Span::styled(
        subtitle,
        Style::default().fg(THEME.muted),
    )))
    .alignment(Alignment::Center);
    f.render_widget(sub_para, chunks[3]);
}

/// Render a panel showing key technical indicators for the active symbol.
fn render_indicators_panel(f: &mut Frame, area: Rect, app: &AppState) {
    let symbol = &app.trade_entry_symbol;
    let block = Block::default()
        .title(Span::styled(
            format!("📊 INDICATORS ({})", symbol),
            Style::default()
                .fg(THEME.brand)
                .add_modifier(Modifier::BOLD),
        ))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(THEME.border));

    let inner = block.inner(area);
    f.render_widget(block, area);

    let mut lines = vec![];

    if let Some(metrics_map) = app.latest_metrics.get(symbol) {
        // Extract indicators
        let rsi = metrics_map
            .get("rsi_14")
            .and_then(|v| v.as_f64())
            .unwrap_or(50.0);
        let macd_hist = metrics_map
            .get("macd_hist")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0);
        let atr_pct = metrics_map
            .get("atr_pct")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.01);
        let regime = metrics_map
            .get("regime_hint")
            .and_then(|v| v.as_str())
            .unwrap_or("ranging");
        let confluence = metrics_map
            .get("confluence_hint")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.5);
        let adx = metrics_map
            .get("adx")
            .and_then(|v| v.as_f64())
            .unwrap_or(25.0);
        let obv_dir = metrics_map
            .get("obv_direction")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0);
        let mfi = metrics_map
            .get("mfi")
            .and_then(|v| v.as_f64())
            .unwrap_or(50.0);
        let cmf = metrics_map
            .get("cmf")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0);

        let regime_display = match regime {
            "trending_bull" => "Bull (🟢)",
            "trending_bear" => "Bear (🔴)",
            "volatile" => "Volatile (🟡)",
            _ => "Ranging (⚪)",
        };

        let confluence_color = if confluence > 0.65 {
            THEME.positive
        } else if confluence < 0.35 {
            THEME.negative
        } else {
            THEME.neutral
        };

        lines.push(Line::from(vec![
            Span::styled("Regime: ", Style::default().fg(THEME.muted)),
            Span::styled(
                regime_display,
                Style::default()
                    .fg(THEME.highlight)
                    .add_modifier(Modifier::BOLD),
            ),
        ]));
        lines.push(Line::from(vec![
            Span::styled("Edge:   ", Style::default().fg(THEME.muted)),
            Span::styled(
                format!("{:.0}%", confluence * 100.0),
                Style::default()
                    .fg(confluence_color)
                    .add_modifier(Modifier::BOLD),
            ),
        ]));
        lines.push(Line::from(vec![
            Span::styled("RSI:    ", Style::default().fg(THEME.muted)),
            Span::styled(
                format!("{:.1}", rsi),
                Style::default().fg(if !(30.0..=70.0).contains(&rsi) {
                    THEME.warning
                } else {
                    THEME.highlight
                }),
            ),
        ]));
        lines.push(Line::from(vec![
            Span::styled("MACD:   ", Style::default().fg(THEME.muted)),
            Span::styled(
                format!("{:+.4}", macd_hist),
                Style::default().fg(if macd_hist > 0.0 {
                    THEME.positive
                } else {
                    THEME.negative
                }),
            ),
        ]));
        lines.push(Line::from(vec![
            Span::styled("ATR %:  ", Style::default().fg(THEME.muted)),
            Span::styled(
                format!("{:.2}%", atr_pct * 100.0),
                Style::default().fg(THEME.highlight),
            ),
        ]));
        lines.push(Line::from(vec![
            Span::styled("ADX:    ", Style::default().fg(THEME.muted)),
            Span::styled(
                format!("{:.1}", adx),
                Style::default().fg(if adx > 25.0 {
                    THEME.positive
                } else {
                    THEME.muted
                }),
            ),
        ]));
        lines.push(Line::from(vec![
            Span::styled("OBV:    ", Style::default().fg(THEME.muted)),
            Span::styled(
                if obv_dir > 0.0 {
                    "Bullish"
                } else if obv_dir < 0.0 {
                    "Bearish"
                } else {
                    "Neutral"
                },
                Style::default().fg(if obv_dir > 0.0 {
                    THEME.positive
                } else if obv_dir < 0.0 {
                    THEME.negative
                } else {
                    THEME.muted
                }),
            ),
        ]));
        lines.push(Line::from(vec![
            Span::styled("MFI:    ", Style::default().fg(THEME.muted)),
            Span::styled(
                format!("{:.1}", mfi),
                Style::default().fg(if !(20.0..=80.0).contains(&mfi) {
                    THEME.warning
                } else {
                    THEME.highlight
                }),
            ),
        ]));
        lines.push(Line::from(vec![
            Span::styled("CMF:    ", Style::default().fg(THEME.muted)),
            Span::styled(
                format!("{:+.2}", cmf),
                Style::default().fg(if cmf > 0.15 {
                    THEME.positive
                } else if cmf < -0.15 {
                    THEME.negative
                } else {
                    THEME.muted
                }),
            ),
        ]));
    } else {
        lines.push(Line::from(Span::styled(
            "  No indicator data.",
            Style::default().fg(THEME.muted),
        )));
        lines.push(Line::from(Span::styled(
            "  Ensure orchestrator is",
            Style::default().fg(THEME.muted),
        )));
        lines.push(Line::from(Span::styled(
            "  running & computing.",
            Style::default().fg(THEME.muted),
        )));
    }

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "  ↑↓ cycle symbol",
        Style::default().fg(THEME.muted),
    )));

    let p = Paragraph::new(lines).wrap(Wrap { trim: true });
    f.render_widget(p, inner);
}
