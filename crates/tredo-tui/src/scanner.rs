//! Live Market Scanner tab — Real-time market data across watchlist symbols.
//!
//! Shows price, change, volume, and signal data in a compact table.
//! Reads from crypto_prices and aggregated signals via AppState.

use crate::prelude::*;
use crate::AppState;

pub fn render_scanner(f: &mut Frame, area: Rect, app: &AppState) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(3)])
        .split(area);

    // ── Title with market stats ───────────────────────────────────────────
    let bullish = app
        .aggregated_signal
        .as_ref()
        .and_then(|s| s.get("bullish_count"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let bearish = app
        .aggregated_signal
        .as_ref()
        .and_then(|s| s.get("bearish_count"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let neutral = app
        .aggregated_signal
        .as_ref()
        .and_then(|s| s.get("neutral_count"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0);

    let title = Line::from(vec![
        Span::styled(
            "  🔍 LIVE MARKET SCANNER",
            Style::default()
                .fg(THEME.brand)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!("  │  ▲ {}  ▼ {}  ◆ {}", bullish, bearish, neutral),
            Style::default().fg(THEME.muted),
        ),
    ]);
    f.render_widget(Paragraph::new(title), chunks[0]);

    // ── Build table ──────────────────────────────────────────────────────
    let mut rows: Vec<ListItem> = Vec::new();

    // Header row
    let header = ListItem::new(Line::from(
        [
            " Symbol ", "Price   ", "Change ", "Volume  ", "Signal ", "Score",
        ]
        .iter()
        .map(|h| {
            Span::styled(
                format!(" {:>8} ", h),
                Style::default()
                    .fg(THEME.brand)
                    .add_modifier(Modifier::BOLD),
            )
        })
        .collect::<Vec<_>>(),
    ));
    rows.push(header);
    rows.push(ListItem::new(Line::from(Span::styled(
        "",
        Style::default(),
    ))));

    // Get symbols from watchlist and crypto_prices
    let symbols: Vec<&str> = if !app.watchlist.is_empty() {
        app.watchlist.iter().map(|s| s.as_str()).collect()
    } else {
        vec![
            "BTC", "ETH", "SOL", "BNB", "XRP", "ADA", "DOGE", "AVAX", "MATIC", "LINK", "DOT",
            "ATOM", "LTC", "UNI", "AAVE", "NEAR", "APT", "ARB", "OP", "SUI", "INJ", "TON", "TRX",
            "XLM", "PEPE", "SHIB",
        ]
    };

    for sym in symbols {
        let price = app
            .crypto_prices
            .get(sym)
            .and_then(|d| d.get("price"))
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0);
        let change = app
            .crypto_prices
            .get(sym)
            .and_then(|d| d.get("binance"))
            .and_then(|b| b.get("change_pct_24h"))
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0);
        let volume = app
            .crypto_prices
            .get(sym)
            .and_then(|d| d.get("binance"))
            .and_then(|b| b.get("volume_24h"))
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0);

        // Try to get signal from skill_votes or aggregated_signal
        let signal_str = match app.skill_votes.get(sym) {
            Some(vote) => vote
                .get("direction")
                .and_then(|d| d.as_str())
                .unwrap_or("Neutral"),
            None => "Neutral",
        };
        let score = match app.skill_votes.get(sym) {
            Some(vote) => vote.get("score").and_then(|s| s.as_f64()).unwrap_or(0.0),
            None => 0.0,
        };

        let change_color = if change > 0.0 {
            THEME.positive
        } else if change < 0.0 {
            THEME.negative
        } else {
            THEME.muted
        };
        let signal_color = match signal_str {
            "Bullish" => THEME.positive,
            "Bearish" => THEME.negative,
            _ => THEME.muted,
        };
        let score_color = if score > 0.3 {
            THEME.positive
        } else if score < -0.3 {
            THEME.negative
        } else {
            THEME.muted
        };

        rows.push(ListItem::new(Line::from(vec![
            Span::styled(
                format!(" {:>8} ", sym),
                Style::default()
                    .fg(THEME.highlight)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!(" {:>8.0} ", price),
                Style::default().fg(THEME.highlight),
            ),
            Span::styled(
                format!(" {:>+7.1}% ", change),
                Style::default().fg(change_color),
            ),
            Span::styled(
                format!(" {:>8.0} ", volume),
                Style::default().fg(THEME.muted),
            ),
            Span::styled(
                format!(" {:>8} ", signal_str),
                Style::default().fg(signal_color),
            ),
            Span::styled(
                format!(" {:>+.2} ", score),
                Style::default().fg(score_color),
            ),
        ])));
    }

    if rows.len() <= 2 {
        rows.push(ListItem::new(Line::from(Span::styled(
            "  No market data available yet. Ensure backend is running.",
            Style::default().fg(THEME.muted),
        ))));
    }

    let list = List::new(rows).block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(THEME.border)),
    );
    f.render_widget(list, chunks[1]);
}
