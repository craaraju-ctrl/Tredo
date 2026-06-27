//! Policy Cache tab — Shows the agent's learned trading memory.
//!
//! Displays a table of (features → action → outcome) entries that the
//! system has cached to skip expensive Ollama calls on repeated market
//! conditions. Each row shows symbol, recommended action, win rate,
//! sample count, confidence score, and average regret.

use crate::prelude::*;
use crate::AppState;
use std::time::Instant;

/// Render a small spinning loading indicator.
fn loading_spinner(now: Instant) -> &'static str {
    let phase = (now.elapsed().as_millis() / 250) % 4;
    match phase {
        0 => "◴",
        1 => "◷",
        2 => "◶",
        _ => "◵",
    }
}

/// Build a mini sparkline chart from data using Unicode block chars.
///
/// Maps [min, max] of the data into 8 levels (▁▂▃▄▅▆▇█) and returns a
/// `Line` with the sparkline flanked by min/max axis labels and avg label,
/// all color-coded by the average value.
/// If fewer than 2 data points, returns a dim "waiting for data..." line.
pub(crate) fn build_sparkline_line(
    history: &[f64],
    label: &str,
    as_percentage: bool,
) -> Line<'static> {
    if history.len() < 2 {
        return Line::from(Span::styled(
            format!("  {}: waiting for more data...", label),
            Style::default().fg(Color::DarkGray),
        ));
    }
    let min_val = history.iter().copied().fold(f64::INFINITY, f64::min);
    let max_val = history.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    let range = (max_val - min_val).max(0.001);
    let avg = history.iter().sum::<f64>() / history.len() as f64;

    let bars: [&str; 8] = ["▁", "▂", "▃", "▄", "▅", "▆", "▇", "█"];
    let line: String = history
        .iter()
        .map(|v| {
            let idx = ((v - min_val) / range * 7.0).round().clamp(0.0, 7.0) as usize;
            bars[idx]
        })
        .collect();

    // Color threshold: for percentages (0-1 scale) use 0.6/0.3;
    // for raw P&L values, positive avg is green, near-zero is yellow, negative is red.
    let color = if as_percentage {
        if avg >= 0.6 {
            Color::Green
        } else if avg >= 0.3 {
            Color::Yellow
        } else {
            Color::Red
        }
    } else {
        if avg > 0.0 {
            Color::Green
        } else if avg > -100.0 {
            Color::Yellow
        } else {
            Color::Red
        }
    };

    // Build the mini chart: label |max▁▂▃▄▅▆▇█ min| avg
    let (max_label, min_label, avg_label) = if as_percentage {
        (
            format!("{:.0}%", max_val * 100.0),
            format!("{:.0}%", min_val * 100.0),
            format!("avg {:.0}%", avg * 100.0),
        )
    } else {
        (
            format!("{:+.1}", max_val),
            format!("{:+.1}", min_val),
            format!("avg {:+.1}", avg),
        )
    };

    Line::from(vec![
        Span::styled("  ", Style::default().fg(Color::DarkGray)),
        Span::styled(format!("{}: ", label), Style::default().fg(Color::DarkGray)),
        // Max label on the left (like top of Y-axis)
        Span::styled(
            max_label,
            Style::default().fg(color).add_modifier(Modifier::BOLD),
        ),
        Span::styled(" ", Style::default().fg(Color::DarkGray)),
        // Sparkline characters
        Span::styled(
            line,
            Style::default().fg(color).add_modifier(Modifier::BOLD),
        ),
        // Min label on the right (like bottom of Y-axis)
        Span::styled(format!(" {} ", min_label), Style::default().fg(color)),
        // Avg label
        Span::styled(avg_label, Style::default().fg(Color::DarkGray)),
    ])
}

pub fn render_policy_cache(f: &mut Frame, area: Rect, app: &AppState) {
    let data = app.policy_cache.as_ref();
    let now = Instant::now();

    // ── Layout ────────────────────────────────────────────────────────────
    let show_search = app.policy_cache_filter_active || !app.policy_cache_filter.is_empty();
    let search_height: u16 = if show_search { 3 } else { 0 };
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),             // title
            Constraint::Length(8),             // summary stats + timestamp + 4 sparklines
            Constraint::Length(search_height), // search bar (1 or 3)
            Constraint::Min(3),                // table
        ])
        .split(area);

    // ── Title with loading indicator ──────────────────────────────────────
    let is_loading = data.is_none();
    let spinner = if is_loading {
        loading_spinner(now)
    } else {
        "✓"
    };
    let title_color = if is_loading {
        Color::Yellow
    } else {
        THEME.brand
    };
    let ws_dot = if app.ws_connected { "🟢" } else { "🔴" };
    let comm_count = app.live_comm_log.len();
    let title = Paragraph::new(Line::from(vec![
        Span::styled(
            format!("🧠 POLICY CACHE — Learned Trading Memory  {}", spinner),
            Style::default()
                .fg(title_color)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!("   {} WS  |  {} live msgs", ws_dot, comm_count),
            Style::default().fg(Color::DarkGray),
        ),
    ]));
    f.render_widget(title, chunks[0]);

    // ── Summary stats + timestamp ─────────────────────────────────────────
    if let Some(cache) = data {
        let total_entries = cache
            .get("total_entries")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        let total_samples = cache
            .get("total_samples")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        let min_samples = cache
            .get("config")
            .and_then(|c| c.get("min_samples"))
            .and_then(|v| v.as_u64())
            .unwrap_or(5);
        let min_win_rate = cache
            .get("config")
            .and_then(|c| c.get("min_win_rate"))
            .and_then(|v| v.as_f64())
            .unwrap_or(0.55);
        let min_confidence = cache
            .get("config")
            .and_then(|c| c.get("min_confidence"))
            .and_then(|v| v.as_f64())
            .unwrap_or(0.6);

        // Cache hit rate stats
        let hit_rate = cache
            .get("hit_stats")
            .and_then(|h| h.get("hit_rate"))
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0);
        let total_lookups = cache
            .get("hit_stats")
            .and_then(|h| h.get("total_lookups"))
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        let cache_hits_count = cache
            .get("hit_stats")
            .and_then(|h| h.get("cache_hits"))
            .and_then(|v| v.as_u64())
            .unwrap_or(0);

        // Hit-rate history for sparkline
        let hit_history: Vec<f64> = cache
            .get("hit_rate_history")
            .and_then(|h| h.as_array())
            .map(|a| a.iter().filter_map(|v| v.as_f64()).collect())
            .unwrap_or_default();

        // Top-performers win-rate history for sparkline
        let wr_history: Vec<f64> = cache
            .get("top_win_rate_history")
            .and_then(|h| h.as_array())
            .map(|a| a.iter().filter_map(|v| v.as_f64()).collect())
            .unwrap_or_default();

        // Top-performers confidence history for sparkline
        let conf_history: Vec<f64> = cache
            .get("confidence_history")
            .and_then(|h| h.as_array())
            .map(|a| a.iter().filter_map(|v| v.as_f64()).collect())
            .unwrap_or_default();

        // Consecutive win/loss streak history for sparkline
        let streak_history: Vec<f64> = cache
            .get("streak_history")
            .and_then(|h| h.as_array())
            .map(|a| a.iter().filter_map(|v| v.as_f64()).collect())
            .unwrap_or_default();

        let stats_text = format!(
            "Entries: {}  |  Total Samples: {}  |  Hit Rate: {:.0}% ({}/{}) | Thresholds: min_samples={}, min_win_rate={:.0}%, min_confidence={:.2}",
            total_entries, total_samples, hit_rate * 100.0, cache_hits_count, total_lookups,
            min_samples, min_win_rate * 100.0, min_confidence
        );

        // Build hit-rate sparkline line
        let sparkline_hit_rate = build_sparkline_line(&hit_history, "Hit Rate Trend", true);

        // Build top-performers win-rate sparkline line
        let sparkline_win_rate = build_sparkline_line(&wr_history, "Top Perf WR Trend", true);

        // Build top-performers confidence sparkline line
        let sparkline_conf = build_sparkline_line(&conf_history, "Top Perf Conf Trend", true);

        // Build win/loss streak sparkline line (raw values, not percentage)
        let sparkline_streak = build_sparkline_line(&streak_history, "Win/Loss Streak", false);

        // Build timestamp line with color coding
        let (timestamp_line, ts_color) = match app.policy_cache_loaded_at {
            Some(t) => {
                let secs = t.elapsed().as_secs();
                let color = if secs < 10 {
                    Color::Green
                } else {
                    Color::Yellow
                };
                let text = if secs < 60 {
                    format!("Last updated: {}s ago", secs)
                } else {
                    format!("Last updated: {}m {}s ago", secs / 60, secs % 60)
                };
                (text, color)
            }
            None => ("Last updated: never".to_string(), Color::Red),
        };

        let stats = Paragraph::new(vec![
            Line::from(Span::styled(&stats_text, Style::default().fg(Color::White))),
            Line::from(Span::styled(
                "Top performers shown below (min 3 samples). Entries are sorted by win rate.",
                Style::default().fg(Color::DarkGray),
            )),
            sparkline_hit_rate,
            sparkline_win_rate,
            sparkline_conf,
            sparkline_streak,
            Line::from(vec![
                Span::styled("  ", Style::default().fg(Color::DarkGray)),
                Span::styled(&timestamp_line, Style::default().fg(ts_color)),
            ]),
        ])
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::DarkGray)),
        );
        f.render_widget(stats, chunks[1]);
    } else {
        let spinner = loading_spinner(now);
        let no_data = Paragraph::new(vec![
            Line::from(Span::styled(
                format!("{} Loading policy cache...", spinner),
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            )),
            Line::from(Span::styled(
                "Connecting to orchestrator API. Ensure `tredo` is running.",
                Style::default().fg(Color::DarkGray),
            )),
            Line::from(Span::styled(
                "Once connected, the cache will populate automatically as trades are executed.",
                Style::default().fg(Color::DarkGray),
            )),
        ])
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::DarkGray)),
        );
        f.render_widget(no_data, chunks[1]);
    }

    // ── Search Bar ─────────────────────────────────────────────────────────
    let filter_text = &app.policy_cache_filter;
    let filter_active = app.policy_cache_filter_active;

    if show_search {
        let search_border_color = if filter_active {
            Color::Yellow
        } else {
            Color::DarkGray
        };
        let search_title = if filter_active {
            " 🔍 Filter (Esc to close) "
        } else {
            " 🔍 Filter (press / to search) "
        };
        let display_text = if filter_text.is_empty() {
            if filter_active {
                "Type to filter by symbol..."
            } else {
                "No filter active"
            }
        } else {
            filter_text
        };

        let search_block = Block::default()
            .title(search_title)
            .borders(Borders::ALL)
            .border_style(Style::default().fg(search_border_color));

        let inner = search_block.inner(chunks[2]);
        f.render_widget(search_block, chunks[2]);

        // Show the filter text with a cursor if active
        let cursor_vis = if filter_active { "█" } else { "" };
        let display_str = format!(" {}{}", display_text, cursor_vis);
        let search_content = Paragraph::new(Line::from(vec![Span::styled(
            &display_str,
            Style::default().fg(if filter_active {
                Color::White
            } else {
                Color::DarkGray
            }),
        )]));
        f.render_widget(search_content, inner);
    }

    // ── Table ─────────────────────────────────────────────────────────────
    let all_entries: Vec<serde_json::Value> = data
        .and_then(|c| c.get("top_performers"))
        .and_then(|v| v.as_array())
        .map(|a| a.to_vec())
        .unwrap_or_default();

    // Filter entries by symbol (case-insensitive)
    let top_entries: Vec<&serde_json::Value> = if filter_text.is_empty() {
        all_entries.iter().collect()
    } else {
        let lower = filter_text.to_lowercase();
        all_entries
            .iter()
            .filter(|e| {
                e.get("features")
                    .and_then(|f| f.get("symbol"))
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_lowercase().contains(&lower))
                    .unwrap_or(false)
            })
            .collect()
    };

    if top_entries.is_empty() {
        let msg = if filter_text.is_empty() {
            "No entries yet. Trades will populate the cache automatically."
        } else {
            "No entries match your filter. Try a different symbol."
        };
        let empty = Paragraph::new(Line::from(Span::styled(
            msg,
            Style::default().fg(Color::DarkGray),
        )))
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title("Top Performers")
                .border_style(Style::default().fg(Color::DarkGray)),
        );
        f.render_widget(empty, chunks[3]);
        return;
    }

    // Build header row
    let header_cells = [
        "Symbol",
        "Action",
        "Win Rate",
        "Samples",
        "Confidence",
        "Avg P&L%",
        "Avg Regret",
    ];
    let header = ListItem::new(Line::from(
        header_cells
            .iter()
            .map(|h| {
                Span::styled(
                    format!(" {:>8} ", h),
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                )
            })
            .collect::<Vec<_>>(),
    ));

    let mut rows: Vec<ListItem> = vec![header];

    for entry in &top_entries {
        let symbol = entry
            .get("features")
            .and_then(|f| f.get("symbol"))
            .and_then(|v| v.as_str())
            .unwrap_or("?");
        let action = entry
            .get("recommended_action")
            .and_then(|v| v.as_str())
            .unwrap_or("?");
        let win_rate = entry
            .get("win_rate")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.5);
        let samples = entry
            .get("sample_size")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        let confidence = entry
            .get("confidence")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0);
        let avg_pnl = entry
            .get("avg_pnl_pct")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0);
        let avg_regret = entry
            .get("avg_regret")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0);

        let wr_color = if win_rate >= 0.65 {
            Color::Green
        } else if win_rate >= 0.55 {
            Color::Yellow
        } else {
            Color::Red
        };
        let conf_color = if confidence >= 0.7 {
            Color::Green
        } else if confidence >= 0.5 {
            Color::Yellow
        } else {
            Color::DarkGray
        };
        let pnl_color = if avg_pnl >= 0.0 {
            Color::Green
        } else {
            Color::Red
        };
        let regret_color = if avg_regret <= 0.3 {
            Color::Green
        } else if avg_regret <= 0.6 {
            Color::Yellow
        } else {
            Color::Red
        };

        rows.push(ListItem::new(Line::from(vec![
            Span::styled(
                format!(" {:>8} ", symbol),
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!(" {:>6} ", action),
                Style::default().fg(Color::Yellow),
            ),
            Span::styled(
                format!(" {:>8.1}% ", win_rate * 100.0),
                Style::default().fg(wr_color),
            ),
            Span::styled(
                format!(" {:>7} ", samples),
                Style::default().fg(Color::White),
            ),
            Span::styled(
                format!(" {:>10.2} ", confidence),
                Style::default().fg(conf_color),
            ),
            Span::styled(
                format!(" {:>8.2}% ", avg_pnl),
                Style::default().fg(pnl_color),
            ),
            Span::styled(
                format!(" {:>9.3} ", avg_regret),
                Style::default().fg(regret_color),
            ),
        ])));
    }

    let list = List::new(rows).block(
        Block::default()
            .borders(Borders::ALL)
            .title("Top Performers")
            .border_style(Style::default().fg(Color::DarkGray)),
    );
    f.render_widget(list, chunks[3]);
}
