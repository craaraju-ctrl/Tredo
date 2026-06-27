//! Help tab — Keyboard shortcuts and philosophy.

use crate::prelude::*;

pub fn render_help(f: &mut Frame, area: Rect) {
    let help = r"
tredo Terminal UI — Trading Real-time Edge Decision Optimisation

This is the primary way to observe and lightly control the autonomous system.

The real brain (temporal loops + Tredo agents + debate + skills) runs in `tredo-orchestrator`.

Key commands (from any terminal):
  tredo                 # start everything (or launch this UI)
  tredo tui             # explicitly launch this full Terminal UI
  tredo health / status / cot / portfolio / tree
  tredo watchlist add TSLA
  tredo rules min_confluence_score=0.72

In this UI:
  Tab / Shift+Tab or 1-8   Change view
  r                        Refresh now
  ↑ ↓                      Scroll / Select model
  ← →                      Navigate action buttons
  Enter                    Activate focused button / Select model
  q / Ctrl-C               Quit (backend keeps running)

Action Buttons:
  ▶ Run Pipeline     Execute a full pipeline cycle for the watchlist symbol
  🔄 Refresh         Force refresh all data from the backend
  ⚡ Trigger Cycle   Trigger a complete pipeline with LLM decision

Everything stays paper-only until you are 100% confident.
";

    let p = Paragraph::new(help)
        .block(
            Block::default()
                .title("❓ Help & Philosophy")
                .borders(Borders::ALL),
        )
        .wrap(Wrap { trim: true });
    f.render_widget(p, area);
}
