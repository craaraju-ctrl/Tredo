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
//!   ← / →        Navigate action buttons
//!   Enter        Activate focused action button / Select model
//!   Esc          Reset scroll or go back
//!
//! Run via: `tredo tui`

// ── Module declarations ───────────────────────────────────────────────────
mod cot;
mod dashboard;
mod help;
mod models;
mod positions;
mod prelude;
mod rules;
mod tree;
mod ui;
mod watchlist;

// ── Imports ───────────────────────────────────────────────────────────────
use std::collections::HashMap;
use std::io;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use crossterm::{
    event::{
        self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyModifiers, MouseButton,
        MouseEventKind,
    },
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{backend::CrosstermBackend, layout::Rect, Terminal};

pub(crate) use crate::cot::render_cot;
pub(crate) use crate::dashboard::render_dashboard;
pub(crate) use crate::help::render_help;
pub(crate) use crate::models::render_models;
pub(crate) use crate::positions::render_positions;
pub(crate) use crate::rules::render_rules;
pub(crate) use crate::tree::render_tree;
pub(crate) use crate::ui::ui;
pub(crate) use crate::watchlist::render_watchlist;

// ── Constants ─────────────────────────────────────────────────────────────

const API_BASE: &str = "http://localhost:8082/api";
const POLL_INTERVAL: Duration = Duration::from_secs(2);

// ── Action Buttons ────────────────────────────────────────────────────────

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum ButtonAction {
    RunPipeline,
    ForceRefresh,
    TriggerCycle,
}

impl ButtonAction {
    pub(crate) fn icon(&self) -> &'static str {
        match self {
            ButtonAction::RunPipeline => "▶",
            ButtonAction::ForceRefresh => "🔄",
            ButtonAction::TriggerCycle => "⚡",
        }
    }
    pub(crate) fn label(&self) -> &'static str {
        match self {
            ButtonAction::RunPipeline => " Run Pipeline",
            ButtonAction::ForceRefresh => " Refresh",
            ButtonAction::TriggerCycle => " Trigger Cycle",
        }
    }
    pub(crate) fn description(&self) -> &'static str {
        match self {
            ButtonAction::RunPipeline => "Execute pipeline cycle for watchlist",
            ButtonAction::ForceRefresh => "Refresh all data from backend",
            ButtonAction::TriggerCycle => "Trigger complete pipeline with LLM",
        }
    }
}

pub(crate) const ALL_BUTTONS: &[ButtonAction] = &[
    ButtonAction::RunPipeline,
    ButtonAction::ForceRefresh,
    ButtonAction::TriggerCycle,
];

// ── Application State ─────────────────────────────────────────────────────

#[derive(Debug, Default)]
pub(crate) struct AppState {
    pub(crate) status: Option<serde_json::Value>,
    pub(crate) health: Option<serde_json::Value>,
    pub(crate) cot: Vec<serde_json::Value>,
    pub(crate) cot_by_agent: HashMap<String, serde_json::Value>,
    pub(crate) agents: Option<serde_json::Value>,
    pub(crate) skill_votes: HashMap<String, serde_json::Value>,
    pub(crate) aggregated_signal: Option<serde_json::Value>,
    pub(crate) watchlist: Vec<String>,
    pub(crate) models: Vec<serde_json::Value>,
    pub(crate) current_model: Option<String>,
    pub(crate) last_poll: Option<Instant>,
    pub(crate) scroll_offset: usize,
    pub(crate) tree_scroll: usize,
    pub(crate) selected_tab: usize,
    pub(crate) selected_model_index: usize,
    pub(crate) error: Option<String>,
    // Interactive button state
    pub(crate) button_focus_offset: usize,
    pub(crate) action_running: Option<ButtonAction>,
    pub(crate) action_message: Option<(String, Instant)>,
    pub(crate) action_result_rx: Option<mpsc::Receiver<String>>,
    pub(crate) force_poll: bool,
    pub(crate) button_areas: Vec<Rect>,
    pub(crate) tab_areas: Vec<Rect>,
    /// When set, a confirmation dialog is active (y/N)
    pub(crate) confirm_action: Option<ButtonAction>,
}

// ── Tabs ──────────────────────────────────────────────────────────────────

#[derive(PartialEq, Clone, Copy)]
pub(crate) enum Tab {
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
    pub(crate) fn title(self) -> &'static str {
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

// ── Main Entry Point ──────────────────────────────────────────────────────

fn main() -> anyhow::Result<()> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let mut app = AppState::default();
    let res = run_app(&mut terminal, &mut app);

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

// ── Main Event Loop ───────────────────────────────────────────────────────

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
            match event::read()? {
                Event::Key(key) => {
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
                            app.force_poll = true;
                        }
                        KeyCode::Left => {
                            if app.button_focus_offset > 0 {
                                app.button_focus_offset -= 1;
                            }
                        }
                        KeyCode::Right => {
                            if app.button_focus_offset < ALL_BUTTONS.len() - 1 {
                                app.button_focus_offset += 1;
                            }
                        }
                        KeyCode::Enter => {
                            // If a confirmation is active, treat Enter as 'y'
                            if app.confirm_action.is_some() {
                                let action = app.confirm_action.take().unwrap();
                                run_pipeline_action(app, action);
                            } else {
                                let client = reqwest::blocking::Client::builder()
                                    .timeout(Duration::from_secs(5))
                                    .build();

                                if app.selected_tab == Tab::Models as usize {
                                    if let (Some(client), Some(model)) = (
                                        client.as_ref().ok(),
                                        app.models.get(app.selected_model_index),
                                    ) {
                                        if let Some(name) =
                                            model.get("name").and_then(|n| n.as_str())
                                        {
                                            let body = serde_json::json!({ "model": name });
                                            if let Ok(resp) = client
                                                .post(format!("{}/models/set", API_BASE))
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
                                                        app.action_message = Some((
                                                            format!("✅ Switched to {}", name),
                                                            Instant::now(),
                                                        ));
                                                    }
                                                }
                                            }
                                        }
                                    }
                                } else if app.action_running.is_none() {
                                    if let Some(action) = ALL_BUTTONS.get(app.button_focus_offset) {
                                        execute_button_action(app, *action);
                                    }
                                }
                            }
                        }
                        KeyCode::Char('y') | KeyCode::Char('Y') => {
                            if let Some(action) = app.confirm_action.take() {
                                run_pipeline_action(app, action);
                            }
                        }
                        KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
                            if app.confirm_action.take().is_some() {
                                app.action_message =
                                    Some(("❌ Cancelled.".to_string(), Instant::now()));
                            }
                            app.scroll_offset = 0;
                            app.selected_model_index = 0;
                        }
                        KeyCode::Up => match app.selected_tab {
                            t if t == Tab::Models as usize => {
                                if app.selected_model_index > 0 {
                                    app.selected_model_index -= 1;
                                }
                            }
                            t if t == Tab::Tree as usize => {
                                if app.tree_scroll > 0 {
                                    app.tree_scroll -= 1;
                                }
                            }
                            _ => {
                                if app.scroll_offset > 0 {
                                    app.scroll_offset -= 1;
                                }
                            }
                        },
                        KeyCode::Down => match app.selected_tab {
                            t if t == Tab::Models as usize => {
                                if app.selected_model_index < app.models.len().saturating_sub(1) {
                                    app.selected_model_index += 1;
                                }
                            }
                            t if t == Tab::Tree as usize => {
                                app.tree_scroll += 1;
                            }
                            _ => {
                                app.scroll_offset += 1;
                            }
                        },
                        _ => {}
                    }
                }
                Event::Mouse(mouse) if mouse.kind == MouseEventKind::Down(MouseButton::Left) => {
                    // Cancel any pending confirmation on mouse click
                    if app.confirm_action.take().is_some() {
                        app.action_message = Some(("❌ Cancelled.".to_string(), Instant::now()));
                    }

                    let col = mouse.column;
                    let row = mouse.row;

                    // Check tab clicks first (higher priority)
                    for (i, area) in app.tab_areas.iter().enumerate() {
                        if row >= area.y
                            && row < area.y + area.height
                            && col >= area.x
                            && col < area.x + area.width
                        {
                            app.selected_tab = i;
                            app.scroll_offset = 0;
                            break;
                        }
                    }

                    // Then check button clicks
                    for (i, area) in app.button_areas.iter().enumerate() {
                        if row >= area.y
                            && row < area.y + area.height
                            && col >= area.x
                            && col < area.x + area.width
                        {
                            app.button_focus_offset = i;
                            if app.action_running.is_none() {
                                if let Some(action) = ALL_BUTTONS.get(i) {
                                    execute_button_action(app, *action);
                                }
                            }
                            break;
                        }
                    }
                }
                _ => {}
            }
        }

        let action_result = app
            .action_result_rx
            .as_ref()
            .and_then(|rx| rx.try_recv().ok());
        if let Some(msg) = action_result {
            app.action_message = Some((msg, Instant::now()));
            app.action_running = None;
            app.action_result_rx = None;
        }

        if last_tick.elapsed() >= POLL_INTERVAL || app.force_poll {
            poll_backend(app);
            last_tick = Instant::now();
            app.force_poll = false;
        }

        if let Some((_, msg_time)) = &app.action_message {
            if msg_time.elapsed() > Duration::from_secs(3) {
                app.action_message = None;
            }
        }
    }
}

// ── Button Action Execution ──────────────────────────────────────────────

/// Start a confirmation dialog for pipeline actions.
/// Returns immediately — the pipeline runs only after the user confirms with 'y'.
fn execute_button_action(app: &mut AppState, action: ButtonAction) {
    match action {
        ButtonAction::RunPipeline | ButtonAction::TriggerCycle => {
            // Show confirmation prompt instead of running immediately
            app.confirm_action = Some(action);
            app.action_message = Some((
                format!("{} {}? (y/N)", action.icon(), action.label().trim()),
                Instant::now(),
            ));
        }
        ButtonAction::ForceRefresh => {
            app.force_poll = true;
            app.action_message = Some(("🔄 Refreshing data...".to_string(), Instant::now()));
        }
    }
}

/// Actually run the pipeline action after the user confirmed.
fn run_pipeline_action(app: &mut AppState, action: ButtonAction) {
    app.action_running = Some(action);

    let (tx, rx) = mpsc::channel();
    app.action_result_rx = Some(rx);

    let api_base = API_BASE.to_string();
    std::thread::spawn(move || {
        let result = match reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
        {
            Ok(client) => {
                let body = serde_json::json!({ "symbol": "BTC" });
                match client
                    .post(format!("{}/trigger_cycle", api_base))
                    .json(&body)
                    .send()
                {
                    Ok(resp) => {
                        if resp.status().is_success() {
                            if let Ok(json) = resp.json::<serde_json::Value>() {
                                let decision =
                                    json.get("decision").and_then(|d| d.as_str()).unwrap_or("?");
                                format!("✅ Pipeline done → {}", decision)
                            } else {
                                "✅ Pipeline cycle completed".to_string()
                            }
                        } else {
                            format!("❌ Pipeline failed (HTTP {})", resp.status().as_u16())
                        }
                    }
                    Err(e) => format!("❌ Request failed: {}", e),
                }
            }
            Err(_) => "❌ Failed to create HTTP client".to_string(),
        };
        let _ = tx.send(result);
    });
}

// ── Backend Polling ───────────────────────────────────────────────────────

fn poll_backend(app: &mut AppState) {
    let client = match reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(3))
        .build()
    {
        Ok(c) => c,
        Err(_) => {
            app.error = Some("Failed to create HTTP client.".into());
            return;
        }
    };

    // Status
    match client.get(format!("{}/status", API_BASE)).send() {
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

    // Health
    match client.get(format!("{}/health", API_BASE)).send() {
        Ok(resp) if resp.status().is_success() => {
            if let Ok(json) = resp.json::<serde_json::Value>() {
                app.health = Some(json);
            }
        }
        Ok(_) | Err(_) => {}
    }

    // COT
    if let Ok(resp) = client.get(format!("{}/cot", API_BASE)).send() {
        if let Ok(json) = resp.json::<Vec<serde_json::Value>>() {
            app.cot = json;
            let mut index: HashMap<String, serde_json::Value> = HashMap::new();
            for entry in app.cot.iter().rev() {
                if let Some(agent) = entry.get("agent").and_then(|a| a.as_str()) {
                    index
                        .entry(agent.to_string())
                        .or_insert_with(|| entry.clone());
                }
            }
            app.cot_by_agent = index;
        }
    }

    // Agents tree
    if let Ok(resp) = client.get(format!("{}/agents", API_BASE)).send() {
        if let Ok(json) = resp.json::<serde_json::Value>() {
            app.agents = Some(json);
        }
    }

    // Watchlist
    if let Ok(resp) = client.get(format!("{}/watchlist", API_BASE)).send() {
        if let Ok(json) = resp.json::<Vec<String>>() {
            app.watchlist = json;
        }
    }

    // Models
    if let Ok(resp) = client.get(format!("{}/models", API_BASE)).send() {
        if let Ok(json) = resp.json::<serde_json::Value>() {
            if let Some(models) = json.get("models").and_then(|m| m.as_array()) {
                app.models = models.clone();
            }
            if let Some(current) = json.get("current_model").and_then(|m| m.as_str()) {
                app.current_model = Some(current.to_string());
            }
        }
    }

    // Skill scores
    if let Ok(resp) = client.get(format!("{}/skills", API_BASE)).send() {
        if let Ok(json) = resp.json::<serde_json::Value>() {
            if let Some(votes) = json.get("votes").and_then(|v| v.as_array()) {
                let mut idx: HashMap<String, serde_json::Value> = HashMap::new();
                for vote in votes {
                    if let Some(name) = vote.get("skill_name").and_then(|n| n.as_str()) {
                        idx.entry(name.to_string()).or_insert_with(|| vote.clone());
                    }
                }
                app.skill_votes = idx;
            }
            app.aggregated_signal = json.get("aggregated").cloned();
        }
    }

    app.last_poll = Some(Instant::now());
}
