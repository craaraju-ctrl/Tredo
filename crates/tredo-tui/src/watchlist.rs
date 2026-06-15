//! Watchlist tab — Display watched symbols.

use crate::prelude::*;
use crate::AppState;

pub fn render_watchlist(f: &mut Frame, area: Rect, app: &AppState) {
    let items: Vec<ListItem> = app
        .watchlist
        .iter()
        .enumerate()
        .skip(app.scroll_offset)
        .take(area.height as usize - 2)
        .map(|(i, s)| {
            let label = if i == 0 { "★" } else { "○" };
            ListItem::new(Line::from(vec![
                Span::styled(format!("{} ", label), Style::default().fg(Color::Yellow)),
                Span::styled(s, Style::default().fg(Color::Cyan)),
            ]))
        })
        .collect();

    let list = List::new(items).block(
        Block::default()
            .title("👁️ Watchlist (live via /api/watchlist)")
            .borders(Borders::ALL),
    );
    f.render_widget(list, area);
}
