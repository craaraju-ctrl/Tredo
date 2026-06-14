//! tredo-tui — The full beautiful Terminal UI for
//! Trading Real-time Edge Decision Optimisation (tredo).
//!
//! This is the PRIMARY interface. The autonomous brain (tredo-orchestrator)
//! runs in the background. This TUI is pure observer + light control.
//!
//! Keys:
//!   q / Ctrl-C   Quit
//!   Tab / 1-8    Switch tabs
//!   r            Force refresh (poll APIs now)
//!   ↑ / ↓        Scroll logs / lists
//!   Enter        Select model / action
//!   Esc          Reset scroll or go back
//!
//! Run via the launcher: `tredo tui` (it ensures the backend is up).

use std::io;
use std::time::{Duration, Instant};

use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, Paragraph, Tabs, Wrap},
    Terminal,
};

const API_BASE: &str = "http://localhost:8082/api";
const POLL_INTERVAL: Duration = Duration::from_secs(2);

#[derive(Clone, Debug, Default)]
struct AppState {
    status: Option<serde_json::Value>,
    cot: Vec<serde_json::Value>,
    agents: Option<serde_json::Value>,
    watchlist: Vec<String>,
    models: Vec<serde_json::Value>,
    current_model: Option<String>,
    last_poll: Option<Instant>,
    scroll_offset: usize,
    selected_tab: usize,
    selected_model_index: usize,
    error: Option<String>,
}

#[derive(PartialEq, Clone, Copy)]
enum Tab {
    Dashboard = 0,
    Cot = 1,
    Positions = 2,
    Watchlist = 3,
    Models = 4,
    Tree = 5,
    Rules = 6,
    Help = 7,
}

impl Tab {
    fn title(self) -> &'static str {
        match self {
            Tab::Dashboard => "Dashboard",
            Tab::Cot => "COT Log",
            Tab::Positions => "Positions",
            Tab::Watchlist => "Watchlist",
            Tab::Models => "🤖 LLM Models",
            Tab::Tree => "Agent Tree",
            Tab::Rules => "Rules",
            Tab::Help => "Help",
        }
    }
}

fn main() -> anyhow::Result<()> {
    // Setup terminal
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let mut app = AppState::default();
    let res = run_app(&mut terminal, &mut app);

    // Restore terminal
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;

    if let Err(err) = res {
        println!("{err:?}");
    }
    Ok(())
}

fn run_app<B: ratatui::backend::Backend>(
    terminal: &mut Terminal<B>,
    app: &mut AppState,
) -> anyhow::Result<()> {
    let mut last_tick = Instant::now();

    loop {
        terminal.draw(|f| ui(f, app))?;

        let timeout = POLL_INTERVAL
            .checked_sub(last_tick.elapsed())
            .unwrap_or_else(|| Duration::from_millis(50));

        if crossterm::event::poll(timeout)? {
            if let Event::Key(key) = event::read()? {
                match key.code {
                    KeyCode::Char('q') => return Ok(()),
                    KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        return Ok(());
                    }
                    KeyCode::Tab => {
                        app.selected_tab = (app.selected_tab + 1) % 8;
                        app.scroll_offset = 0;
                    }
                    KeyCode::BackTab => {
                        app.selected_tab = if app.selected_tab == 0 {
                            7
                        } else {
                            app.selected_tab - 1
                        };
                        app.scroll_offset = 0;
                    }
                    KeyCode::Char(c @ '1'..='8') => {
                        app.selected_tab = (c as usize - '1' as usize).min(7);
                        app.scroll_offset = 0;
                    }
                    KeyCode::Char('r') => {
                        // Force immediate poll
                        last_tick = Instant::now() - POLL_INTERVAL;
                    }
                    KeyCode::Up => {
                        if app.selected_tab == Tab::Models as usize {
                            if app.selected_model_index > 0 {
                                app.selected_model_index -= 1;
                            }
                        } else if app.scroll_offset > 0 {
                            app.scroll_offset -= 1;
                        }
                    }
                    KeyCode::Down => {
                        if app.selected_tab == Tab::Models as usize {
                            if app.selected_model_index < app.models.len().saturating_sub(1) {
                                app.selected_model_index += 1;
                            }
                        } else {
                            app.scroll_offset += 1;
                        }
                    }
                    KeyCode::Enter => {
                        if app.selected_tab == Tab::Models as usize && !app.models.is_empty() {
                            if let Some(model) = app.models.get(app.selected_model_index) {
                                if let Some(name) = model.get("name").and_then(|n| n.as_str()) {
                                    // Switch model
                                    if let Ok(client) = reqwest::blocking::Client::builder()
                                        .timeout(Duration::from_secs(5))
                                        .build()
                                    {
                                        let body = serde_json::json!({ "model": name });
                                        if let Ok(resp) = client
                                            .post(format!("{}/api/models/set", API_BASE))
                                            .json(&body)
                                            .send()
                                        {
                                            if let Ok(json) = resp.json::<serde_json::Value>() {
                                                if json
                                                    .get("success")
                                                    .and_then(|s| s.as_bool())
                                                    .unwrap_or(false)
                                                {
                                                    app.current_model = Some(name.to_string());
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                    KeyCode::Esc => {
                        app.scroll_offset = 0;
                        app.selected_model_index = 0;
                    }
                    _ => {}
                }
            }
        }

        if last_tick.elapsed() >= POLL_INTERVAL {
            poll_backend(app);
            last_tick = Instant::now();
        }
    }
}

fn poll_backend(app: &mut AppState) {
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(3))
        .build()
        .unwrap_or_default();

    // Status
    match client.get(format!("{}/api/status", API_BASE)).send() {
        Ok(resp) if resp.status().is_success() => {
            if let Ok(json) = resp.json::<serde_json::Value>() {
                app.status = Some(json);
                app.error = None;
            }
        }
        Ok(_) | Err(_) => {
            app.error = Some("Backend not responding. Run `tredo` or `tredo start` first.".into());
        }
    }

    // COT (most recent first in UI)
    if let Ok(resp) = client.get(format!("{}/api/cot", API_BASE)).send() {
        if let Ok(json) = resp.json::<Vec<serde_json::Value>>() {
            app.cot = json;
        }
    }

    // Agents tree
    if let Ok(resp) = client.get(format!("{}/api/agents", API_BASE)).send() {
        if let Ok(json) = resp.json::<serde_json::Value>() {
            app.agents = Some(json);
        }
    }

    // Watchlist
    if let Ok(resp) = client.get(format!("{}/api/watchlist", API_BASE)).send() {
        if let Ok(json) = resp.json::<Vec<String>>() {
            app.watchlist = json;
        }
    }

    // Models
    if let Ok(resp) = client.get(format!("{}/api/models", API_BASE)).send() {
        if let Ok(json) = resp.json::<serde_json::Value>() {
            if let Some(models) = json.get("models").and_then(|m| m.as_array()) {
                app.models = models.clone();
            }
            if let Some(current) = json.get("current_model").and_then(|m| m.as_str()) {
                app.current_model = Some(current.to_string());
            }
        }
    }

    app.last_poll = Some(Instant::now());
}

fn ui(f: &mut ratatui::Frame, app: &mut AppState) {
    let size = f.area();

    // Header with gradient-like effect using colors
    let header = Paragraph::new(vec![
        Line::from(vec![
            Span::styled("⚡ tredo", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
            Span::raw("  —  "),
            Span::styled("Trading Real-time Edge Decision Optimisation", Style::default().fg(Color::White)),
        ]),
        Line::from(Span::styled(
            "Full Terminal UI  •  Autonomous • Paper Only  •  Press q to quit, Tab/1-8 to navigate, r to refresh",
            Style::default().fg(Color::DarkGray),
        )),
    ])
    .block(Block::default().borders(Borders::BOTTOM).border_style(Style::default().fg(Color::Cyan)));

    // Tabs
    let tab_titles: Vec<Line> = (0..8)
        .map(|i| {
            let tab = match i {
                0 => Tab::Dashboard,
                1 => Tab::Cot,
                2 => Tab::Positions,
                3 => Tab::Watchlist,
                4 => Tab::Models,
                5 => Tab::Tree,
                6 => Tab::Rules,
                _ => Tab::Help,
            };
            if i == app.selected_tab {
                Line::from(Span::styled(
                    tab.title(),
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD),
                ))
            } else {
                Line::from(Span::styled(tab.title(), Style::default().fg(Color::White)))
            }
        })
        .collect();

    let tabs = Tabs::new(tab_titles)
        .block(Block::default().borders(Borders::BOTTOM))
        .select(app.selected_tab);

    // Main content area
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // header
            Constraint::Length(3), // tabs
            Constraint::Min(3),    // content
            Constraint::Length(2), // footer
        ])
        .split(size);

    f.render_widget(header, chunks[0]);
    f.render_widget(tabs, chunks[1]);

    // Content by tab
    match app.selected_tab {
        0 => render_dashboard(f, chunks[2], app),
        1 => render_cot(f, chunks[2], app),
        2 => render_positions(f, chunks[2], app),
        3 => render_watchlist(f, chunks[2], app),
        4 => render_models(f, chunks[2], app),
        5 => render_tree(f, chunks[2], app),
        6 => render_rules(f, chunks[2], app),
        _ => render_help(f, chunks[2]),
    }

    // Footer / status
    let health = if let Some(s) = &app.status {
        let k = if s.get("kronos").and_then(|v| v.as_bool()).unwrap_or(false) {
            "🔷"
        } else {
            "❌"
        };
        let o = if s
            .get("orchestrator")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
        {
            "⚙️"
        } else {
            "❌"
        };
        let l = if s.get("llm").and_then(|v| v.as_bool()).unwrap_or(false) {
            "🤖"
        } else {
            "❌"
        };
        let m = app.current_model.as_deref().unwrap_or("unknown");
        format!(
            "K:{} O:{} L:{} | Model: {} | Last: {:?}s ago",
            k,
            o,
            l,
            m,
            app.last_poll.map(|t| t.elapsed().as_secs()).unwrap_or(0)
        )
    } else {
        "Connecting to backend... (run `tredo` or `tredo start`)".to_string()
    };

    let footer = Paragraph::new(health)
        .style(Style::default().fg(Color::DarkGray))
        .block(Block::default().borders(Borders::TOP));
    f.render_widget(footer, chunks[3]);

    if let Some(err) = &app.error {
        let err_p = Paragraph::new(err.as_str())
            .style(Style::default().fg(Color::Red))
            .wrap(Wrap { trim: true });
        let err_area = Rect {
            x: 2,
            y: size.height.saturating_sub(4),
            width: size.width - 4,
            height: 2,
        };
        f.render_widget(err_p, err_area);
    }
}

fn render_dashboard(f: &mut ratatui::Frame, area: Rect, app: &AppState) {
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

    let pnl_color = if pnl >= 0.0 { Color::Green } else { Color::Red };
    let win_rate = if trades > 0 {
        wins as f64 / trades as f64 * 100.0
    } else {
        0.0
    };

    let text = vec![
        Line::from(vec![Span::styled(
            "📊 PORTFOLIO",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )]),
        Line::from(""),
        Line::from(vec![
            Span::styled("  EQUITY     ", Style::default().fg(Color::Cyan)),
            Span::styled(
                format!("₹{:>12.2}", equity),
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(vec![
            Span::styled("  CASH       ", Style::default().fg(Color::Cyan)),
            Span::styled(
                format!("₹{:>12.2}", cash),
                Style::default().fg(Color::White),
            ),
        ]),
        Line::from(vec![
            Span::styled("  DAILY P&L  ", Style::default().fg(Color::Cyan)),
            Span::styled(
                format!("{:>+12.2}", pnl),
                Style::default().fg(pnl_color).add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(""),
        Line::from(vec![Span::styled(
            "📈 STATISTICS",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )]),
        Line::from(""),
        Line::from(vec![
            Span::styled("  TRADES     ", Style::default().fg(Color::Cyan)),
            Span::styled(format!("{}", trades), Style::default().fg(Color::White)),
            Span::raw("  │  Wins: "),
            Span::styled(format!("{}", wins), Style::default().fg(Color::Green)),
            Span::raw("  │  Losses: "),
            Span::styled(format!("{}", losses), Style::default().fg(Color::Red)),
        ]),
        Line::from(vec![
            Span::styled("  WIN RATE   ", Style::default().fg(Color::Cyan)),
            Span::styled(
                format!("{:>5.1}%", win_rate),
                Style::default().fg(if win_rate >= 50.0 {
                    Color::Green
                } else {
                    Color::Red
                }),
            ),
        ]),
        Line::from(""),
        Line::from(Span::styled(
            "tredo is running autonomously in the background.",
            Style::default().fg(Color::DarkGray),
        )),
        Line::from(Span::styled(
            "Switch tabs with Tab or number keys. Press r to force a refresh.",
            Style::default().fg(Color::DarkGray),
        )),
    ];

    let p = Paragraph::new(text)
        .block(
            Block::default()
                .title("📊 Dashboard")
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Cyan)),
        )
        .wrap(Wrap { trim: true });
    f.render_widget(p, area);
}

fn render_cot(f: &mut ratatui::Frame, area: Rect, app: &AppState) {
    let items: Vec<ListItem> = app
        .cot
        .iter()
        .rev()
        .skip(app.scroll_offset)
        .take(area.height as usize - 2)
        .map(|entry| {
            let ts = entry
                .get("timestamp")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let agent = entry.get("agent").and_then(|v| v.as_str()).unwrap_or("?");
            let action = entry.get("action").and_then(|v| v.as_str()).unwrap_or("");
            let reason = entry.get("reason").and_then(|v| v.as_str()).unwrap_or("");

            let color = match agent {
                a if a.contains("Identifier") || a.contains("Market") => Color::Green,
                a if a.contains("Verifier") || a.contains("Risk") => Color::Yellow,
                a if a.contains("Executer") || a.contains("Execution") => Color::Magenta,
                a if a.contains("Guardian") || a.contains("Drawdown") => Color::Red,
                a if a.contains("MetaControl") => Color::Cyan,
                _ => Color::White,
            };

            ListItem::new(Line::from(vec![
                Span::styled(
                    format!("[{}] ", &ts[..ts.len().min(19)]),
                    Style::default().fg(Color::DarkGray),
                ),
                Span::styled(
                    format!("{}: ", agent),
                    Style::default().fg(color).add_modifier(Modifier::BOLD),
                ),
                Span::styled(action, Style::default().fg(Color::White)),
                Span::raw("  "),
                Span::styled(reason, Style::default().fg(Color::Gray)),
            ]))
        })
        .collect();

    let list = List::new(items).block(
        Block::default()
            .title("🔀 Live COT — Chain of Thought")
            .borders(Borders::ALL),
    );
    f.render_widget(list, area);
}

fn render_positions(f: &mut ratatui::Frame, area: Rect, app: &AppState) {
    let status = app.status.as_ref();
    let positions = status
        .and_then(|s| s.get("open_positions"))
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    let lines: Vec<Line> = if positions.is_empty() {
        vec![Line::from(Span::styled(
            "No open positions (paper).",
            Style::default().fg(Color::DarkGray),
        ))]
    } else {
        let mut lines = vec![
            Line::from(vec![
                Span::styled(
                    "SYMBOL",
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw("  "),
                Span::styled(
                    "DIR",
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw("  "),
                Span::styled(
                    "ENTRY",
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw("  "),
                Span::styled(
                    "CURRENT",
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw("  "),
                Span::styled(
                    "P&L",
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                ),
            ]),
            Line::from(""),
        ];

        for p in positions.iter() {
            let sym = p.get("symbol").and_then(|v| v.as_str()).unwrap_or("?");
            let dir = p.get("direction").and_then(|v| v.as_str()).unwrap_or("?");
            let entry = p.get("entry_price").and_then(|v| v.as_f64()).unwrap_or(0.0);
            let current = p
                .get("current_price")
                .and_then(|v| v.as_f64())
                .unwrap_or(entry);
            let pnl_pct = if entry > 0.0 {
                (current - entry) / entry * 100.0
            } else {
                0.0
            };
            let pnl_color = if pnl_pct >= 0.0 {
                Color::Green
            } else {
                Color::Red
            };

            lines.push(Line::from(vec![
                Span::styled(sym.to_string(), Style::default().fg(Color::White)),
                Span::raw("  "),
                Span::styled(
                    dir.to_string(),
                    Style::default().fg(if dir == "Long" {
                        Color::Green
                    } else {
                        Color::Red
                    }),
                ),
                Span::raw("  "),
                Span::styled(format!("{:.2}", entry), Style::default().fg(Color::White)),
                Span::raw("  "),
                Span::styled(format!("{:.2}", current), Style::default().fg(Color::White)),
                Span::raw("  "),
                Span::styled(format!("{:+.2}%", pnl_pct), Style::default().fg(pnl_color)),
            ]));
        }
        lines
    };

    let p = Paragraph::new(lines)
        .block(
            Block::default()
                .title("📋 Open Positions")
                .borders(Borders::ALL),
        )
        .wrap(Wrap { trim: true });
    f.render_widget(p, area);
}

fn render_watchlist(f: &mut ratatui::Frame, area: Rect, app: &AppState) {
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

fn render_models(f: &mut ratatui::Frame, area: Rect, app: &AppState) {
    let current = app.current_model.as_deref().unwrap_or("unknown");

    let header_text = vec![
        Line::from(vec![Span::styled(
            "🤖 LLM Model Selection",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )]),
        Line::from(""),
        Line::from(vec![
            Span::styled("Current: ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                current,
                Style::default()
                    .fg(Color::Green)
                    .add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(""),
        Line::from(Span::styled(
            "Use ↑/↓ to select, Enter to switch model",
            Style::default().fg(Color::DarkGray),
        )),
    ];

    let header = Paragraph::new(header_text)
        .block(Block::default().borders(Borders::BOTTOM))
        .wrap(Wrap { trim: true });
    f.render_widget(
        header,
        Rect {
            x: area.x,
            y: area.y,
            width: area.width,
            height: 6,
        },
    );

    let list_area = Rect {
        x: area.x,
        y: area.y + 6,
        width: area.width,
        height: area.height.saturating_sub(7),
    };

    let items: Vec<ListItem> = app
        .models
        .iter()
        .map(|m| {
            let name = m.get("name").and_then(|n| n.as_str()).unwrap_or("unknown");
            let size = m.get("size").and_then(|s| s.as_str()).unwrap_or("-");
            let is_selected = app
                .models
                .get(app.selected_model_index)
                .and_then(|s| s.get("name"))
                .and_then(|n| n.as_str())
                .map(|n| n == name)
                .unwrap_or(false);

            let prefix = if is_selected { "👉 " } else { "  " };
            let style = if is_selected {
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::White)
            };

            ListItem::new(Line::from(vec![
                Span::styled(prefix, style),
                Span::styled(name, style),
                Span::raw("  "),
                Span::styled(format!("({})", size), Style::default().fg(Color::DarkGray)),
            ]))
        })
        .collect();

    let list = List::new(items).block(
        Block::default()
            .title("Available Models")
            .borders(Borders::ALL),
    );
    f.render_widget(list, list_area);
}

fn render_tree(f: &mut ratatui::Frame, area: Rect, app: &AppState) {
    let text = if let Some(tree) = &app.agents {
        format!("{:#}", tree)
    } else {
        "Agent tree not available yet.\nThe Tredo hierarchy (Identifier → Verifier → Executer → Guardian) runs in the orchestrator.".to_string()
    };

    let p = Paragraph::new(text)
        .block(
            Block::default()
                .title("🌳 Tredo Agent Hierarchy")
                .borders(Borders::ALL),
        )
        .wrap(Wrap { trim: true });
    f.render_widget(p, area);
}

fn render_rules(f: &mut ratatui::Frame, area: Rect, app: &AppState) {
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

fn render_help(f: &mut ratatui::Frame, area: Rect) {
    let help = r#"
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
  Enter                    Switch model (in Models tab)
  q / Ctrl-C               Quit (backend keeps running)

Everything stays paper-only until you are 100% confident.
"#;

    let p = Paragraph::new(help)
        .block(
            Block::default()
                .title("❓ Help & Philosophy")
                .borders(Borders::ALL),
        )
        .wrap(Wrap { trim: true });
    f.render_widget(p, area);
}
