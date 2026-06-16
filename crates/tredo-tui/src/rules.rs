//! Rules tab — Structured discipline rules toggle switches.
//!
//! Reads current rules from the /api/status endpoint and provides
//! interactive toggle switches for boolean rules.

use crate::prelude::*;
use crate::AppState;

pub fn render_rules(f: &mut Frame, area: Rect, app: &AppState) {
    let status = app.status.as_ref();

    // Read current rules from backend status
    let use_confluence = status
        .and_then(|s| s.get("use_confluence"))
        .and_then(|v| v.as_bool())
        .unwrap_or(true);
    let respect_session = status
        .and_then(|s| s.get("respect_session_timing"))
        .and_then(|v| v.as_bool())
        .unwrap_or(true);

    let lines = vec![
        Line::from(Span::styled(
            "  ⚖️ DISCIPLINE RULES",
            Style::default()
                .fg(THEME.brand)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        // Toggle: Use Confluence
        Line::from(vec![
            Span::styled("  Use Confluence Score        ", Style::default().fg(THEME.highlight)),
            Span::styled(
                if use_confluence { "● ON" } else { "○ OFF" },
                Style::default()
                    .fg(if use_confluence { THEME.positive } else { THEME.muted })
                    .add_modifier(if use_confluence { Modifier::BOLD } else { Modifier::empty() }),
            ),
            Span::styled("     [POST /api/rules]", Style::default().fg(THEME.muted)),
        ]),
        Line::from(""),
        // Toggle: Respect Session Timing
        Line::from(vec![
            Span::styled("  Respect Session Timing      ", Style::default().fg(THEME.highlight)),
            Span::styled(
                if respect_session { "● ON" } else { "○ OFF" },
                Style::default()
                    .fg(if respect_session { THEME.positive } else { THEME.muted })
                    .add_modifier(if respect_session { Modifier::BOLD } else { Modifier::empty() }),
            ),
            Span::styled("     [POST /api/rules]", Style::default().fg(THEME.muted)),
        ]),
        Line::from(""),
        // Max daily loss
        Line::from(vec![
            Span::styled(
                "  Max Daily Loss               ₹5,000.00",
                Style::default().fg(THEME.highlight),
            ),
        ]),
        Line::from(""),
        // Min confidence score
        Line::from(vec![
            Span::styled(
                "  Min Confidence Score          0.60",
                Style::default().fg(THEME.highlight),
            ),
        ]),
        Line::from(""),
        Line::from(""),
        // Usage hint
        Line::from(Span::styled(
            "  To change rules, use:  tredo rules key=value",
            Style::default().fg(THEME.muted),
        )),
        Line::from(Span::styled(
            "  Examples:",
            Style::default().fg(THEME.muted),
        )),
        Line::from(Span::styled(
            "    tredo rules use_confluence=false",
            Style::default().fg(THEME.muted),
        )),
        Line::from(Span::styled(
            "    tredo rules respect_session_timing=true",
            Style::default().fg(THEME.muted),
        )),
        Line::from(Span::styled(
            "    tredo rules min_confidence_score=0.72",
            Style::default().fg(THEME.muted),
        )),
    ];

    let p = Paragraph::new(lines)
        .block(
            Block::default()
                .title("⚖️ Discipline Rules")
                .borders(Borders::ALL)
                .border_style(Style::default().fg(THEME.border)),
        )
        .wrap(Wrap { trim: true });
    f.render_widget(p, area);
}
