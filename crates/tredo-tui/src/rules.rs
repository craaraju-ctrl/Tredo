//! Rules tab — Display current discipline rules from the backend.

use crate::prelude::*;
use crate::AppState;

pub fn render_rules(f: &mut Frame, area: Rect, app: &AppState) {
    let status = app.status.as_ref();
    let rules = status
        .and_then(|s| s.get("rules"))
        .cloned()
        .unwrap_or(serde_json::json!({}));

    let p = Paragraph::new(format!("Current rules (from backend):\n\n{:#}", rules))
        .block(
            Block::default()
                .title("⚖️ Discipline Rules (use `tredo rules k=v` to change)")
                .borders(Borders::ALL),
        )
        .wrap(Wrap { trim: true });
    f.render_widget(p, area);
}
