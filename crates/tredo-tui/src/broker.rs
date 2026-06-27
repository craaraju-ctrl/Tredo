//! Broker & Data tab — Broker connection status, account balances, live data feeds.
//!
//! Shows real-time status of all configured brokers (Alpaca, Zerodha, Binance),
//! their connection state, account balances, margin usage, and active orders.
//! Users can view which brokers are active and their live data feed status.

use crate::prelude::*;
use crate::AppState;

pub fn render_broker(f: &mut Frame, area: Rect, app: &AppState) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // Title
            Constraint::Length(8), // Broker cards row
            Constraint::Length(1), // gap
            Constraint::Min(5),    // Data feeds / Orders
        ])
        .split(area);

    // ── Title ──────────────────────────────────────────────────────────────
    let title = Paragraph::new(Line::from(Span::styled(
        "📡 BROKER & DATA FEEDS",
        Style::default()
            .fg(THEME.brand)
            .add_modifier(Modifier::BOLD),
    )));
    f.render_widget(title, chunks[0]);

    // ── Broker Status Cards ────────────────────────────────────────────────
    let broker_cards = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Ratio(1, 3),
            Constraint::Ratio(1, 3),
            Constraint::Ratio(1, 3),
        ])
        .split(chunks[1]);

    // Alpaca broker
    render_broker_card(
        f,
        broker_cards[0],
        "Alpaca Markets",
        "US Equities / Crypto",
        app,
        "alpaca",
    );

    // Zerodha broker
    render_broker_card(
        f,
        broker_cards[1],
        "Zerodha Kite",
        "India Equities / Derivatives",
        app,
        "zerodha",
    );

    // Binance broker
    render_broker_card(
        f,
        broker_cards[2],
        "Binance",
        "Crypto (Live Data)",
        app,
        "binance",
    );

    // ── Data Feeds & Orders Panel ──────────────────────────────────────────
    let bottom = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Ratio(3, 5), Constraint::Ratio(2, 5)])
        .split(chunks[3]);

    // Data Feeds
    render_data_feeds(f, bottom[0], app);

    // Active Orders
    render_active_orders(f, bottom[1], app);
}

fn render_broker_card(
    f: &mut Frame,
    area: Rect,
    name: &str,
    description: &str,
    app: &AppState,
    broker_id: &str,
) {
    // Check if this broker is connected from health/status data
    let connected = app
        .health
        .as_ref()
        .and_then(|h| h.get(broker_id))
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    let status_color = if connected {
        THEME.positive
    } else {
        THEME.muted
    };
    let status_text = if connected {
        "● CONNECTED"
    } else {
        "○ OFFLINE"
    };

    // Get balance from status if available
    let balance = app
        .status
        .as_ref()
        .and_then(|s| s.get("total_equity"))
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0);

    let block = Block::default()
        .title(Span::styled(
            name,
            Style::default()
                .fg(THEME.highlight)
                .add_modifier(Modifier::BOLD),
        ))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(status_color));

    let inner = block.inner(area);
    f.render_widget(block, area);

    let lines = vec![
        Line::from(Span::styled(description, Style::default().fg(THEME.muted))),
        Line::from(""),
        Line::from(vec![
            Span::styled("Status: ", Style::default().fg(THEME.muted)),
            Span::styled(
                status_text,
                Style::default()
                    .fg(status_color)
                    .add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(vec![
            Span::styled("Balance: ", Style::default().fg(THEME.muted)),
            Span::styled(
                format!("${:.2}", balance),
                Style::default().fg(THEME.highlight),
            ),
        ]),
        Line::from(""),
        Line::from(Span::styled(
            if connected {
                "Paper mode active"
            } else {
                "Run: tredo configure <broker>"
            },
            Style::default().fg(THEME.info),
        )),
    ];

    let p = Paragraph::new(lines).wrap(Wrap { trim: true });
    f.render_widget(p, inner);
}

fn render_data_feeds(f: &mut Frame, area: Rect, app: &AppState) {
    let block = Block::default()
        .title(Span::styled(
            "📊 Live Data Feeds",
            Style::default()
                .fg(THEME.brand)
                .add_modifier(Modifier::BOLD),
        ))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(THEME.border));

    let inner = block.inner(area);
    f.render_widget(block, area);

    let mut lines = vec![];

    // WebSocket status
    let ws_status = if app.ws_connected {
        Span::styled(
            "● WebSocket Connected",
            Style::default()
                .fg(THEME.positive)
                .add_modifier(Modifier::BOLD),
        )
    } else {
        Span::styled(
            "○ WebSocket Disconnected",
            Style::default()
                .fg(THEME.negative)
                .add_modifier(Modifier::BOLD),
        )
    };
    lines.push(Line::from(ws_status));
    lines.push(Line::from(""));

    // Price feeds
    lines.push(Line::from(Span::styled(
        "Price Feeds:",
        Style::default()
            .fg(THEME.highlight)
            .add_modifier(Modifier::BOLD),
    )));

    let symbols = [
        "BTC", "ETH", "SOL", "BNB", "XRP", "ADA", "DOGE", "AVAX", "MATIC", "LINK", "DOT", "ATOM",
        "LTC", "UNI", "AAVE", "NEAR", "APT", "ARB", "OP", "SUI", "INJ", "TON", "TRX", "XLM",
        "PEPE", "SHIB",
    ];
    for sym in &symbols {
        if let Some(data) = app.crypto_prices.get(*sym) {
            let price = data.get("price").and_then(|v| v.as_f64()).unwrap_or(0.0);
            let change = data
                .get("binance")
                .and_then(|b| b.get("change_pct_24h"))
                .and_then(|v| v.as_f64())
                .unwrap_or(0.0);
            let arrow = if change > 0.0 { "▲" } else { "▼" };
            let change_color = if change > 0.0 {
                THEME.positive
            } else if change < 0.0 {
                THEME.negative
            } else {
                THEME.muted
            };

            lines.push(Line::from(vec![
                Span::styled(
                    format!("  {} ", sym),
                    Style::default()
                        .fg(THEME.highlight)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    format!("{:.2}", price),
                    Style::default().fg(THEME.highlight),
                ),
                Span::styled(
                    format!(" {} {:.1}%", arrow, change.abs()),
                    Style::default().fg(change_color),
                ),
            ]));
        }
    }

    if app.crypto_prices.is_empty() {
        lines.push(Line::from(Span::styled(
            "  Waiting for price data...",
            Style::default().fg(THEME.muted),
        )));
    }

    // COT streaming status
    lines.push(Line::from(""));
    lines.push(Line::from(vec![
        Span::styled("COT Entries: ", Style::default().fg(THEME.muted)),
        Span::styled(
            format!("{}", app.ws_cot_count),
            Style::default().fg(THEME.info),
        ),
        Span::styled(" (via WebSocket)", Style::default().fg(THEME.muted)),
    ]));

    let p = Paragraph::new(lines).wrap(Wrap { trim: true });
    f.render_widget(p, inner);
}

fn render_active_orders(f: &mut Frame, area: Rect, app: &AppState) {
    let block = Block::default()
        .title(Span::styled(
            "📋 Open Positions",
            Style::default()
                .fg(THEME.brand)
                .add_modifier(Modifier::BOLD),
        ))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(THEME.border));

    let inner = block.inner(area);
    f.render_widget(block, area);

    // Try to get positions from status
    let positions = app
        .status
        .as_ref()
        .and_then(|s| s.get("open_positions"))
        .and_then(|v| v.as_array());

    let mut lines = vec![];

    // Header
    lines.push(Line::from(vec![Span::styled(
        "  Symbol    Dir    Entry      P&L       Status",
        Style::default()
            .fg(THEME.highlight)
            .add_modifier(Modifier::BOLD),
    )]));
    lines.push(Line::from(Span::styled(
        "  ─────────────────────────────────────────────────",
        Style::default().fg(THEME.border),
    )));

    if let Some(pos_arr) = positions {
        if pos_arr.is_empty() {
            lines.push(Line::from(Span::styled(
                "  No open positions",
                Style::default().fg(THEME.muted),
            )));
        } else {
            for pos in pos_arr.iter().take(10) {
                let sym = pos.get("symbol").and_then(|s| s.as_str()).unwrap_or("???");
                let dir = match pos.get("direction").and_then(|d| d.as_str()) {
                    Some("Long") | Some("long") | Some("BUY") => "Long",
                    Some("Short") | Some("short") | Some("SELL") => "Short",
                    Some(other) => other,
                    None => "?",
                };
                let entry = pos
                    .get("entry_price")
                    .and_then(|v| v.as_f64())
                    .unwrap_or(0.0);
                let pnl = pos
                    .get("unrealized_pnl")
                    .and_then(|v| v.as_f64())
                    .unwrap_or(0.0);
                let qty = pos.get("quantity").and_then(|v| v.as_f64()).unwrap_or(0.0);

                let pnl_color = if pnl >= 0.0 {
                    THEME.positive
                } else {
                    THEME.negative
                };
                let dir_color = if dir == "Long" {
                    THEME.positive
                } else {
                    THEME.negative
                };

                lines.push(Line::from(vec![
                    Span::styled(
                        format!("  {:<8}", sym),
                        Style::default()
                            .fg(THEME.highlight)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(format!("{:<6}", dir), Style::default().fg(dir_color)),
                    Span::styled(
                        format!("{:<9.2}", entry),
                        Style::default().fg(THEME.highlight),
                    ),
                    Span::styled(format!("{:<9.2}", pnl), Style::default().fg(pnl_color)),
                    Span::styled(format!("qty {:.4}", qty), Style::default().fg(THEME.muted)),
                ]));
            }
        }
    } else {
        lines.push(Line::from(Span::styled(
            "  No position data available",
            Style::default().fg(THEME.muted),
        )));
    }

    let p = Paragraph::new(lines).wrap(Wrap { trim: true });
    f.render_widget(p, inner);
}
