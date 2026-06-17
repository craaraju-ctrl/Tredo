//! tredo-tui — The full beautiful Terminal UI for
//! Trading Real-time Edge Decision Optimisation (tredo).
//!
//! This is the PRIMARY interface. The autonomous brain (tredo-orchestrator)
//! runs in the background. This TUI is pure observer + light control.
//!
//! Keys:
//!   q / Ctrl-C   Quit
//!   Tab / 1-0    Switch tabs
//!   b            Jump to Broker page
//!   S            Jump to Settings page
//!   r            Force refresh
//!   /            Search/filter (COT Log, Policy Cache)
//!   s            Sort current table by next column
//!   ?            Toggle keyboard shortcuts overlay
//!   ↑ / ↓        Scroll / Navigate
//!   ← / →        Navigate action buttons
//!   Enter        Activate / Confirm
//!   Esc          Back / Close / Cancel
//!
//! Run via: `tredo tui`

// ── Module declarations ───────────────────────────────────────────────────
mod backtest;
mod broker;
mod cot;
mod dashboard;
mod health;
mod help;
mod models;
mod performance;
mod policy_cache;
mod positions;
mod prelude;
mod rules;
mod scanner;
mod settings;
mod tree;
mod ui;
mod watchlist;
mod ws_client;

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

pub(crate) use crate::backtest::render_backtest;
pub(crate) use crate::broker::render_broker;
pub(crate) use crate::cot::render_cot;
pub(crate) use crate::dashboard::render_dashboard;
pub(crate) use crate::health::render_health;
pub(crate) use crate::help::render_help;
pub(crate) use crate::models::render_models;
pub(crate) use crate::performance::render_performance;
pub(crate) use crate::policy_cache::render_policy_cache;
pub(crate) use crate::positions::render_positions;
pub(crate) use crate::rules::render_rules;
pub(crate) use crate::scanner::render_scanner;
pub(crate) use crate::settings::render_settings;
use crate::settings::{AGENT_NAMES, PANEL_AGENTS, PANEL_MODELS, PANEL_RISK};
pub(crate) use crate::tree::render_tree;
pub(crate) use crate::ui::ui;
pub(crate) use crate::watchlist::render_watchlist;
pub(crate) use crate::ws_client::start_ws_client;

// ── Constants ─────────────────────────────────────────────────────────────

const API_BASE: &str = "http://localhost:8082/api";

/// HTTP polling interval for data that isn't streamed via WebSocket.
/// WebSocket now handles real-time COT, prices, health, and portfolio.
/// HTTP polling is kept as a fallback for non-streamed snapshots
/// (models, watchlist, agents tree, policy cache, backtest).
const POLL_INTERVAL: Duration = Duration::from_secs(10);
const NUM_TABS: usize = 15;

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
    pub(crate) policy_cache: Option<serde_json::Value>,
    pub(crate) policy_cache_loaded_at: Option<Instant>,
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
    pub(crate) button_focus_offset: usize,
    pub(crate) action_running: Option<ButtonAction>,
    pub(crate) action_message: Option<(String, Instant)>,
    pub(crate) action_result_rx: Option<mpsc::Receiver<String>>,
    pub(crate) force_poll: bool,
    pub(crate) button_areas: Vec<Rect>,
    pub(crate) tab_areas: Vec<Rect>,
    pub(crate) confirm_action: Option<ButtonAction>,
    pub(crate) policy_cache_filter: String,
    pub(crate) policy_cache_filter_active: bool,
    pub(crate) sparkline_card_areas: Vec<Rect>,
    pub(crate) crypto_prices: HashMap<String, serde_json::Value>,
    pub(crate) cot_filter: String,
    pub(crate) cot_filter_active: bool,
    pub(crate) show_overlay: bool,
    /// Sort state: (column_index, direction_asc)
    pub(crate) sort_column: usize,
    pub(crate) sort_ascending: bool,
    /// Backtest results (structured JSON from /api/backtest/results)
    pub(crate) backtest_result: Option<serde_json::Value>,
    /// Trade execution form state
    pub(crate) trade_entry_visible: bool,
    pub(crate) trade_entry_symbol: String,
    pub(crate) trade_entry_direction: String, // "Long" or "Short"
    pub(crate) trade_entry_price: f64,
    pub(crate) trade_entry_sl: f64,
    pub(crate) trade_entry_tp: f64,
    /// Which field in the trade form is focused (0-4)
    pub(crate) trade_entry_focus: usize,
    /// WebSocket receiver — raw JSON strings from backend
    pub(crate) ws_rx: Option<mpsc::Receiver<String>>,
    /// Live trend history accumulated from WebSocket portfolio messages
    pub(crate) equity_history: Vec<f64>,
    pub(crate) pnl_history: Vec<f64>,
    pub(crate) win_rate_history: Vec<f64>,
    pub(crate) consecutive_losses_history: Vec<f64>,
    /// WebSocket connection status (displayed in footer)
    pub(crate) ws_connected: bool,
    /// Timestamp of last COT entry received via WS (for dedup)
    pub(crate) ws_cot_count: usize,
    /// Last time any WS message was received (for disconnect detection)
    pub(crate) ws_last_seen: Option<Instant>,
    // ── Settings page interaction state ─────────────────────────────────
    /// Which settings panel is focused (0=LLM Models, 1=Agents, 2=Skills, 3=Risk Params)
    pub(crate) settings_panel_focus: usize,
    /// Which row within the focused panel is selected (e.g., which agent or risk param)
    pub(crate) settings_row_focus: usize,
    /// Agent enable/disable states (agent_index -> enabled)
    pub(crate) agent_enabled: Vec<bool>,
    /// Whether we're editing a risk parameter (shows increment/decrement UI)
    pub(crate) risk_editing: bool,
    /// Confirmation dialog state for risk changes
    pub(crate) settings_confirm: Option<String>,
    /// Message displayed after a settings action
    pub(crate) settings_message: Option<(String, Instant)>,
    /// Clickable areas for the 5-Layer Pipeline Flow boxes on Dashboard
    pub(crate) pipeline_layer_areas: Vec<Rect>,
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
    PolicyCache = 7,
    Scanner = 8,
    Health = 9,
    Performance = 10,
    Backtest = 11,
    Broker = 12,
    Settings = 13,
    Help = 14,
}

impl Tab {
    pub(crate) fn title(self) -> &'static str {
        match self {
            Tab::Dashboard => "Dashboard",
            Tab::Cot => "COT Log",
            Tab::Positions => "Positions",
            Tab::Watchlist => "Watchlist",
            Tab::Models => "🤖 Models",
            Tab::Tree => "Agent Tree",
            Tab::Rules => "Rules",
            Tab::PolicyCache => "🧠 Policy Cache",
            Tab::Scanner => "🔍 Scanner",
            Tab::Health => "🔷 Health",
            Tab::Performance => "📈 Perf",
            Tab::Backtest => "🔬 Backtest",
            Tab::Broker => "📡 Broker",
            Tab::Settings => "⚙️ Settings",
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

    let mut app = AppState {
        trade_entry_symbol: "BTC".to_string(),
        trade_entry_direction: "Long".to_string(),
        trade_entry_price: 42000.0,
        trade_entry_sl: 41000.0,
        trade_entry_tp: 44000.0,
        trade_entry_focus: 0,
        ws_rx: Some(start_ws_client(API_BASE)),
        ws_connected: false,
        ws_cot_count: 0,
        equity_history: Vec::new(),
        pnl_history: Vec::new(),
        win_rate_history: Vec::new(),
        consecutive_losses_history: Vec::new(),
        ws_last_seen: None,
        // Settings page defaults: all 7 agents enabled
        agent_enabled: vec![true; 7],
        settings_panel_focus: 0,
        settings_row_focus: 0,
        risk_editing: false,
        settings_confirm: None,
        settings_message: None,
        ..AppState::default()
    };
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
                        KeyCode::Char('q') if !app.show_overlay => return Ok(()),
                        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                            return Ok(());
                        }
                        KeyCode::Tab if !app.policy_cache_filter_active && !app.cot_filter_active && !app.trade_entry_visible => {
                            reset_settings_if_leaving(app);
                            app.selected_tab = (app.selected_tab + 1) % NUM_TABS;
                            app.scroll_offset = 0;
                        }
                        KeyCode::BackTab if !app.policy_cache_filter_active && !app.cot_filter_active && !app.trade_entry_visible => {
                            reset_settings_if_leaving(app);
                            app.selected_tab = if app.selected_tab == 0 {
                                NUM_TABS - 1
                            } else {
                                app.selected_tab - 1
                            };
                            app.scroll_offset = 0;
                        }
                        KeyCode::Char(c @ '1'..='9')
                            if !app.policy_cache_filter_active && !app.cot_filter_active && !app.show_overlay && !app.trade_entry_visible =>
                        {
                            reset_settings_if_leaving(app);
                            let idx = (c as usize - '1' as usize).min(NUM_TABS - 1);
                            app.selected_tab = idx;
                            app.scroll_offset = 0;
                        }
                        KeyCode::Char('0')
                            if !app.policy_cache_filter_active && !app.cot_filter_active && !app.show_overlay && !app.trade_entry_visible =>
                        {
                            reset_settings_if_leaving(app);
                            app.selected_tab = Tab::Help as usize;
                            app.scroll_offset = 0;
                        }
                        KeyCode::Char('r') => {
                            app.force_poll = true;
                        }
                        KeyCode::Char('s')
                            if app.selected_tab == Tab::PolicyCache as usize
                                && !app.policy_cache_filter_active
                                && !app.cot_filter_active =>
                        {
                            // Cycle sort column: 0-6 for policy cache columns
                            app.sort_column = (app.sort_column + 1) % 7;
                            app.sort_ascending = !app.sort_ascending;
                        }
                        KeyCode::Char('b')
                            if !app.policy_cache_filter_active && !app.cot_filter_active && !app.show_overlay && !app.trade_entry_visible =>
                        {
                            reset_settings_if_leaving(app);
                            app.selected_tab = Tab::Broker as usize;
                            app.scroll_offset = 0;
                        }
                        KeyCode::Char('S')
                            if !app.policy_cache_filter_active && !app.cot_filter_active && !app.show_overlay && !app.trade_entry_visible =>
                        {
                            reset_settings_if_leaving(app);
                            app.selected_tab = Tab::Settings as usize;
                            app.scroll_offset = 0;
                            app.settings_panel_focus = 0;
                            app.settings_row_focus = 0;
                        }
                        // ── Settings page navigation ──────────────────────────
                        KeyCode::Left
                            if app.selected_tab == Tab::Settings as usize
                                && !app.policy_cache_filter_active
                                && !app.cot_filter_active
                                && !app.show_overlay
                                && !app.trade_entry_visible =>
                        {
                            if app.settings_confirm.is_some() {
                                // Confirm dialog: Left = cancel (consistent with y/n/Esc)
                                app.settings_confirm = None;
                            } else if app.risk_editing {
                                // In risk editing mode, Left decreases value
                                adjust_risk_param(app, -1);
                            } else {
                                // Switch panel left
                                app.settings_panel_focus = if app.settings_panel_focus == 0 {
                                    3
                                } else {
                                    app.settings_panel_focus - 1
                                };
                                app.settings_row_focus = 0;
                            }
                        }
                        KeyCode::Right
                            if app.selected_tab == Tab::Settings as usize
                                && !app.policy_cache_filter_active
                                && !app.cot_filter_active
                                && !app.show_overlay
                                && !app.trade_entry_visible =>
                        {
                            if app.settings_confirm.is_some() {
                                // Confirm dialog: Right = cancel
                                app.settings_confirm = None;
                            } else if app.risk_editing {
                                // In risk editing mode, Right increases value
                                adjust_risk_param(app, 1);
                            } else {
                                // Switch panel right
                                app.settings_panel_focus = (app.settings_panel_focus + 1) % 4;
                                app.settings_row_focus = 0;
                            }
                        }
                        KeyCode::Up
                            if app.selected_tab == Tab::Settings as usize
                                && !app.policy_cache_filter_active
                                && !app.cot_filter_active
                                && !app.show_overlay
                                && !app.trade_entry_visible
                                && app.settings_confirm.is_none() =>
                        {
                            if app.risk_editing {
                                // In risk editing mode, Up decreases value
                                adjust_risk_param(app, -1);
                            } else {
                                // Navigate row up within focused panel
                                let max_row = match app.settings_panel_focus {
                                    0 => app.models.len().saturating_sub(1),
                                    1 => 6, // 7 agents (0-6)
                                    2 => app.skill_votes.len().saturating_sub(1),
                                    3 => 4, // 5 risk params (0-4)
                                    _ => 0,
                                };
                                if app.settings_row_focus > 0 {
                                    app.settings_row_focus -= 1;
                                } else {
                                    app.settings_row_focus = max_row;
                                }
                            }
                        }
                        KeyCode::Down
                            if app.selected_tab == Tab::Settings as usize
                                && !app.policy_cache_filter_active
                                && !app.cot_filter_active
                                && !app.show_overlay
                                && !app.trade_entry_visible
                                && app.settings_confirm.is_none() =>
                        {
                            if app.risk_editing {
                                // In risk editing mode, Down increases value
                                adjust_risk_param(app, 1);
                            } else {
                                // Navigate row down within focused panel
                                let max_row = match app.settings_panel_focus {
                                    0 => app.models.len().saturating_sub(1),
                                    1 => 6, // 7 agents (0-6)
                                    2 => app.skill_votes.len().saturating_sub(1),
                                    3 => 4, // 5 risk params (0-4)
                                    _ => 0,
                                };
                                if app.settings_row_focus < max_row {
                                    app.settings_row_focus += 1;
                                } else {
                                    app.settings_row_focus = 0;
                                }
                            }
                        }
                        KeyCode::Char('+') | KeyCode::Char('=')
                            if app.selected_tab == Tab::Settings as usize
                                && !app.policy_cache_filter_active
                                && !app.cot_filter_active
                                && !app.show_overlay
                                && !app.trade_entry_visible
                                && app.settings_confirm.is_none() =>
                        {
                            if app.risk_editing {
                                adjust_risk_param(app, 1);
                            }
                        }
                        KeyCode::Char('-')
                            if app.selected_tab == Tab::Settings as usize
                                && !app.policy_cache_filter_active
                                && !app.cot_filter_active
                                && !app.show_overlay
                                && !app.trade_entry_visible
                                && app.settings_confirm.is_none() =>
                        {
                            if app.risk_editing {
                                adjust_risk_param(app, -1);
                            }
                        }
                        KeyCode::Char('?')
                            if !app.policy_cache_filter_active && !app.cot_filter_active =>
                        {
                            app.show_overlay = !app.show_overlay;
                        }
                        KeyCode::Char('/') => {
                            if app.selected_tab == Tab::PolicyCache as usize {
                                app.policy_cache_filter_active = true;
                                app.policy_cache_filter.clear();
                            } else if app.selected_tab == Tab::Cot as usize {
                                app.cot_filter_active = true;
                                app.cot_filter.clear();
                            }
                        }
                        KeyCode::Left => {
                            if app.trade_entry_visible {
                                // Move to previous field (wrap around)
                                app.trade_entry_focus = if app.trade_entry_focus == 0 {
                                    4
                                } else {
                                    app.trade_entry_focus - 1
                                };
                            } else if app.button_focus_offset > 0 {
                                app.button_focus_offset -= 1;
                            }
                        }
                        KeyCode::Right => {
                            if app.trade_entry_visible {
                                // Move to next field (wrap around)
                                app.trade_entry_focus = (app.trade_entry_focus + 1) % 5;
                            } else if app.button_focus_offset < ALL_BUTTONS.len() - 1 {
                                app.button_focus_offset += 1;
                            }
                        }
                        KeyCode::Enter => {
                            if app.show_overlay {
                                app.show_overlay = false;
                            } else if app.selected_tab == Tab::Settings as usize
                                && !app.policy_cache_filter_active
                                && !app.cot_filter_active
                            {
                                // Settings-specific Enter
                                if app.settings_confirm.is_some() {
                                    handle_settings_confirm(app);
                                } else if app.risk_editing {
                                    // Confirm risk edit → show dialog
                                    adjust_risk_param(app, 0);
                                } else {
                                    match app.settings_panel_focus {
                                        PANEL_AGENTS => {
                                            // Toggle agent enable/disable
                                            if let Some(enabled) = app.agent_enabled.get_mut(app.settings_row_focus) {
                                                *enabled = !*enabled;
                                                let name = AGENT_NAMES.get(app.settings_row_focus).unwrap_or(&"?");
                                                let state = if *enabled { "enabled" } else { "disabled" };
                                                app.settings_message = Some((
                                                    format!("{} {}", name, state),
                                                    Instant::now(),
                                                ));
                                            }
                                        }
                                        PANEL_RISK => {
                                            // Start risk parameter editing
                                            app.risk_editing = true;
                                            app.settings_message = Some((
                                                "Editing risk parameter — use ↑↓ or +/- to adjust, Enter to confirm".to_string(),
                                                Instant::now(),
                                            ));
                                        }
                                        PANEL_MODELS => {
                                            // Switch model (same logic as Models tab)
                                            let client = reqwest::blocking::Client::builder()
                                                .timeout(Duration::from_secs(5))
                                                .build();
                                            if let (Some(client), Some(model)) = (
                                                client.as_ref().ok(),
                                                app.models.get(app.settings_row_focus),
                                            ) {
                                                if let Some(name) = model.get("name").and_then(|n| n.as_str()) {
                                                    let body = serde_json::json!({ "model": name });
                                                    if let Ok(resp) = client
                                                        .post(format!("{}/models/set", API_BASE))
                                                        .json(&body)
                                                        .send()
                                                    {
                                                        if let Ok(json) = resp.json::<serde_json::Value>() {
                                                            if json.get("success").and_then(|s| s.as_bool()).unwrap_or(false) {
                                                                app.current_model = Some(name.to_string());
                                                                app.settings_message = Some((
                                                                    format!("Switched to {}", name),
                                                                    Instant::now(),
                                                                ));
                                                            }
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                        _ => {} // Skills panel: read-only
                                    }
                                }
                            } else if app.trade_entry_visible {
                                // Submit trade
                                app.trade_entry_visible = false;
                                let body = serde_json::json!({
                                    "symbol": app.trade_entry_symbol,
                                    "direction_str": app.trade_entry_direction,
                                    "entry_price": app.trade_entry_price,
                                    "stop_loss": app.trade_entry_sl,
                                    "take_profit": app.trade_entry_tp,
                                });
                                let client = reqwest::blocking::Client::builder()
                                    .timeout(Duration::from_secs(5))
                                    .build();
                                let api_base = API_BASE.to_string();
                                std::thread::spawn(move || {
                                    if let Ok(c) = client {
                                        let resp = c
                                            .post(format!("{}/trade", api_base))
                                            .json(&body)
                                            .send();
                                        match resp {
                                            Ok(r) => {
                                                if r.status().is_success() {
                                                    println!("[TradePanel] Trade submitted successfully");
                                                } else {
                                                    eprintln!("[TradePanel] Trade failed: HTTP {}", r.status());
                                                }
                                            }
                                            Err(e) => {
                                                eprintln!("[TradePanel] Trade request failed: {}", e);
                                            }
                                        }
                                    }
                                });
                            } else if app.confirm_action.is_some() {
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
                                                            format!("Switched to {}", name),
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
                        // Filter text input
                        KeyCode::Char(c)
                            if app.policy_cache_filter_active
                                && app.selected_tab == Tab::PolicyCache as usize =>
                        {
                            app.policy_cache_filter.push(c);
                        }
                        KeyCode::Char(c)
                            if app.cot_filter_active && app.selected_tab == Tab::Cot as usize =>
                        {
                            app.cot_filter.push(c);
                        }
                        KeyCode::Backspace if app.policy_cache_filter_active => {
                            app.policy_cache_filter.pop();
                        }
                        KeyCode::Backspace if app.cot_filter_active => {
                            app.cot_filter.pop();
                        }
                        KeyCode::Esc => {
                            if app.show_overlay {
                                app.show_overlay = false;
                            } else if app.settings_confirm.is_some() {
                                app.settings_confirm = None;
                            } else if app.risk_editing {
                                app.risk_editing = false;
                                app.settings_message = Some(("Edit cancelled.".to_string(), Instant::now()));
                            } else if app.trade_entry_visible {
                                app.trade_entry_visible = false;
                            } else if app.policy_cache_filter_active {
                                app.policy_cache_filter_active = false;
                                app.policy_cache_filter.clear();
                            } else if app.cot_filter_active {
                                app.cot_filter_active = false;
                                app.cot_filter.clear();
                            } else if app.confirm_action.take().is_some() {
                                app.action_message =
                                    Some(("Cancelled.".to_string(), Instant::now()));
                            }
                            app.scroll_offset = 0;
                            app.selected_model_index = 0;
                        }
                        KeyCode::Char('y') | KeyCode::Char('Y') => {
                            if let Some(action) = app.confirm_action.take() {
                                run_pipeline_action(app, action);
                            } else if app.settings_confirm.is_some() {
                                handle_settings_confirm(app);
                            } else if app.selected_tab == Tab::Positions as usize {
                                app.trade_entry_visible = !app.trade_entry_visible;
                            }
                        }
                        KeyCode::Char('n') | KeyCode::Char('N') => {
                            if app.confirm_action.take().is_some() {
                                app.action_message =
                                    Some(("Cancelled.".to_string(), Instant::now()));
                            } else if app.settings_confirm.is_some() {
                                app.settings_confirm = None;
                                app.risk_editing = false;
                                app.settings_message = Some(("Change cancelled.".to_string(), Instant::now()));
                            }
                            app.scroll_offset = 0;
                            app.selected_model_index = 0;
                        }
                        KeyCode::Char('t') if !app.trade_entry_visible && app.selected_tab == Tab::Positions as usize => {
                            app.trade_entry_visible = !app.trade_entry_visible;
                        }
                        KeyCode::Up if app.trade_entry_visible && app.selected_tab == Tab::Positions as usize => {
                            match app.trade_entry_focus {
                                // Symbol: cycle through watchlist (already ordered)
                                0 => {
                                    let symbols = &app.watchlist;
                                    if !symbols.is_empty() {
                                        if let Some(pos) = symbols.iter().position(|s| s == &app.trade_entry_symbol) {
                                            let next = if pos == 0 { symbols.len() - 1 } else { pos - 1 };
                                            if let Some(s) = symbols.get(next) {
                                                app.trade_entry_symbol = s.clone();
                                            }
                                        } else {
                                            app.trade_entry_symbol = symbols[0].clone();
                                        }
                                    }
                                }
                                // Direction: toggle
                                1 => {
                                    app.trade_entry_direction = if app.trade_entry_direction == "Long" {
                                        "Short".to_string()
                                    } else {
                                        "Long".to_string()
                                    };
                                }
                                // Entry price: increase by adaptive step
                                2 => {
                                    let step = price_step(app.trade_entry_price);
                                    app.trade_entry_price += step;
                                }
                                // Stop Loss: increase
                                3 => {
                                    let step = price_step(app.trade_entry_sl.max(app.trade_entry_price));
                                    app.trade_entry_sl += step;
                                }
                                // Take Profit: increase
                                4 => {
                                    let step = price_step(app.trade_entry_tp.max(app.trade_entry_price));
                                    app.trade_entry_tp += step;
                                }
                                _ => {}
                            }
                        }
                        KeyCode::Down if app.trade_entry_visible && app.selected_tab == Tab::Positions as usize => {
                            match app.trade_entry_focus {
                                // Symbol: cycle through watchlist (already ordered)
                                0 => {
                                    let symbols = &app.watchlist;
                                    if !symbols.is_empty() {
                                        let last = symbols.len() - 1;
                                        if let Some(pos) = symbols.iter().position(|s| s == &app.trade_entry_symbol) {
                                            let next = if pos >= last { 0 } else { pos + 1 };
                                            if let Some(s) = symbols.get(next) {
                                                app.trade_entry_symbol = s.clone();
                                            }
                                        } else {
                                            app.trade_entry_symbol = symbols[0].clone();
                                        }
                                    }
                                }
                                // Direction: toggle
                                1 => {
                                    app.trade_entry_direction = if app.trade_entry_direction == "Long" {
                                        "Short".to_string()
                                    } else {
                                        "Long".to_string()
                                    };
                                }
                                // Entry price: decrease by adaptive step
                                2 => {
                                    let step = price_step(app.trade_entry_price);
                                    app.trade_entry_price = (app.trade_entry_price - step).max(0.01);
                                }
                                // Stop Loss: decrease
                                3 => {
                                    let step = price_step(app.trade_entry_sl.max(app.trade_entry_price));
                                    app.trade_entry_sl = (app.trade_entry_sl - step).max(0.01);
                                }
                                // Take Profit: decrease
                                4 => {
                                    let step = price_step(app.trade_entry_tp.max(app.trade_entry_price));
                                    app.trade_entry_tp = (app.trade_entry_tp - step).max(0.01);
                                }
                                _ => {}
                            }
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
                    if app.confirm_action.take().is_some() {
                        app.action_message = Some(("Cancelled.".to_string(), Instant::now()));
                    }

                    let col = mouse.column;
                    let row = mouse.row;

                    for (i, area) in app.tab_areas.iter().enumerate() {
                        if row >= area.y
                            && row < area.y + area.height
                            && col >= area.x
                            && col < area.x + area.width
                        {
                            reset_settings_if_leaving(app);
                            app.selected_tab = i;
                            app.scroll_offset = 0;
                            break;
                        }
                    }

                    // Find which pipeline layer box was clicked (if any)
                    let clicked_layer = app.pipeline_layer_areas.iter().enumerate().find(|(_, area)| {
                        row >= area.y && row < area.y + area.height
                            && col >= area.x && col < area.x + area.width
                    }).map(|(i, _)| i);
                    if let Some(layer_index) = clicked_layer {
                        reset_settings_if_leaving(app);
                        // L1=Rules, L2=COT, L3=AgentTree, L4=COT, L5=Positions
                        app.selected_tab = match layer_index {
                            0 => Tab::Rules as usize,     // L1 Gate → Rules
                            1 => Tab::Cot as usize,       // L2 Ident → COT Log
                            2 => Tab::Tree as usize,      // L3 Debate → Agent Tree
                            3 => Tab::Settings as usize,  // L4 Judge → Settings (agent config)
                            4 => Tab::Positions as usize, // L5 Exec → Positions
                            _ => Tab::Dashboard as usize,
                        };
                        app.scroll_offset = 0;
                    }

                    for area in &app.sparkline_card_areas {
                        if row >= area.y
                            && row < area.y + area.height
                            && col >= area.x
                            && col < area.x + area.width
                        {
                            app.selected_tab = 7;
                            app.scroll_offset = 0;
                            break;
                        }
                    }

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

        // ── Process incoming WebSocket messages ──────────────────────────
        // Collect messages first (without holding a borrow on `app`) to avoid
        // borrow conflicts when calling handle_ws_message(&mut app).
        let ws_msgs: Vec<String> = app.ws_rx.as_ref()
            .map(|rx| std::iter::from_fn(|| rx.try_recv().ok()).collect())
            .unwrap_or_default();
        let ws_received = !ws_msgs.is_empty();
        for msg in &ws_msgs {
            if let Ok(json) = serde_json::from_str::<serde_json::Value>(msg) {
                handle_ws_message(app, json);
            }
        }
        if ws_received {
            app.ws_last_seen = Some(Instant::now());
            if !app.ws_connected {
                app.ws_connected = true;
                app.action_message = Some(("WebSocket connected — real-time updates active".to_string(), Instant::now()));
            }
        }
        // If WS data hasn't arrived in 30s, mark disconnected and re-enable HTTP fallback
        if app.ws_connected && app.ws_last_seen.map(|t| t.elapsed() > Duration::from_secs(30)).unwrap_or(false) {
            app.ws_connected = false;
            // Clear WS-fed data so HTTP fallback re-fetches it on next poll cycle
            app.health = None;
            app.cot.clear();
            app.cot_by_agent.clear();
            app.aggregated_signal = None;
            app.crypto_prices.clear();
            // Clear live trend history so it re-accumulates fresh on reconnect
            app.equity_history.clear();
            app.pnl_history.clear();
            app.win_rate_history.clear();
            app.consecutive_losses_history.clear();
            app.error = Some("WebSocket disconnected. HTTP fallback active.".into());
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

fn execute_button_action(app: &mut AppState, action: ButtonAction) {
    match action {
        ButtonAction::RunPipeline | ButtonAction::TriggerCycle => {
            app.confirm_action = Some(action);
            app.action_message = Some((
                format!("{} {}? (y/N)", action.icon(), action.label().trim()),
                Instant::now(),
            ));
        }
        ButtonAction::ForceRefresh => {
            app.force_poll = true;
            app.action_message = Some(("Refreshing data...".to_string(), Instant::now()));
        }
    }
}

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
                                format!("Pipeline done: {}", decision)
                            } else {
                                "Pipeline cycle completed".to_string()
                            }
                        } else {
                            format!("Pipeline failed (HTTP {})", resp.status().as_u16())
                        }
                    }
                    Err(e) => format!("Request failed: {}", e),
                }
            }
            Err(_) => "Failed to create HTTP client".to_string(),
        };
        let _ = tx.send(result);
    });
}

// ── WebSocket Message Handler ───────────────────────────────────────────

/// Parse an incoming WebSocket JSON message and update AppState accordingly.
/// Supports: cot, price, health, portfolio, signal, ping.
fn handle_ws_message(app: &mut AppState, json: serde_json::Value) {
    let msg_type = match json.get("type").and_then(|t| t.as_str()) {
        Some(t) => t,
        None => return,
    };

    match msg_type {
        "cot" => {
            // Append COT entry to the log
            app.cot.push(json.clone());
            app.ws_cot_count += 1;
            // Update COT-by-agent index
            if let Some(agent) = json.get("agent").and_then(|a| a.as_str()) {
                app.cot_by_agent
                    .insert(agent.to_string(), json.clone());
            }
            // Cap in-memory COT to prevent unbounded growth
            if app.cot.len() > 500 {
                app.cot.drain(0..100);
            }
        }
        "price" => {
            // Update crypto_prices with incoming price data
            if let Some(symbol) = json.get("symbol").and_then(|s| s.as_str()) {
                let price = json.get("price").and_then(|p| p.as_f64()).unwrap_or(0.0);
                let entry = app
                    .crypto_prices
                    .entry(symbol.to_string())
                    .or_insert_with(|| serde_json::json!({}));
                if let Some(obj) = entry.as_object_mut() {
                    obj.insert("price".to_string(), serde_json::json!(price));
                }
            }
        }
        "health" => {
            app.health = Some(json);
        }
        "portfolio" => {
            // Update status with all portfolio fields (dashboard gauge cards)
            let status = app.status.get_or_insert_with(|| serde_json::json!({}));
            if let Some(obj) = status.as_object_mut() {
                if let Some(v) = json.get("total_equity").and_then(|v| v.as_f64()) {
                    obj.insert("total_equity".to_string(), serde_json::json!(v));
                }
                if let Some(v) = json.get("cash_balance").and_then(|v| v.as_f64()) {
                    obj.insert("cash_balance".to_string(), serde_json::json!(v));
                }
                if let Some(v) = json.get("daily_pnl").and_then(|v| v.as_f64()) {
                    obj.insert("daily_pnl".to_string(), serde_json::json!(v));
                }
                if let Some(v) = json.get("trades_today").and_then(|v| v.as_u64()) {
                    obj.insert("total_trades_today".to_string(), serde_json::json!(v));
                }
                if let Some(v) = json.get("winning_trades_today").and_then(|v| v.as_u64()) {
                    obj.insert("winning_trades_today".to_string(), serde_json::json!(v));
                }
                if let Some(v) = json.get("losing_trades_today").and_then(|v| v.as_u64()) {
                    obj.insert("losing_trades_today".to_string(), serde_json::json!(v));
                }
            }

            // Accumulate live trend history (capped at 500)
            if let Some(equity) = json.get("total_equity").and_then(|v| v.as_f64()) {
                app.equity_history.push(equity);
                if app.equity_history.len() > 500 {
                    app.equity_history.remove(0);
                }
            }
            if let Some(pnl) = json.get("daily_pnl").and_then(|v| v.as_f64()) {
                app.pnl_history.push(pnl);
                if app.pnl_history.len() > 500 {
                    app.pnl_history.remove(0);
                }
            }
            if let Some(wr) = json.get("win_rate").and_then(|v| v.as_f64()) {
                app.win_rate_history.push(wr);
                if app.win_rate_history.len() > 500 {
                    app.win_rate_history.remove(0);
                }
            }
            if let Some(cls) = json.get("consecutive_losses").and_then(|v| v.as_f64()) {
                app.consecutive_losses_history.push(cls);
                if app.consecutive_losses_history.len() > 500 {
                    app.consecutive_losses_history.remove(0);
                }
            }
        }
        "signal" => {
            // Update aggregated signal
            app.aggregated_signal = Some(json);
        }
        // "ping" messages are keepalives — no action needed
        _ => {}
    }
}

/// Compute an adaptive step size for price fields based on the magnitude.
fn price_step(price: f64) -> f64 {
    if price > 10000.0 {
        100.0
    } else if price > 1000.0 {
        10.0
    } else if price > 100.0 {
        1.0
    } else if price > 10.0 {
        0.5
    } else {
        0.05
    }
}

// ── Settings Page Interaction Helpers ───────────────────────────────────────

/// Reset transient settings state when leaving the Settings tab.
fn reset_settings_if_leaving(app: &mut AppState) {
    if app.selected_tab == Tab::Settings as usize {
        app.risk_editing = false;
        app.settings_confirm = None;
    }
}

/// Handle confirming a risk parameter change from the settings confirmation dialog.
fn handle_settings_confirm(app: &mut AppState) {
    if app.settings_confirm.is_some() {
        app.settings_confirm = None;
        app.risk_editing = false;
        app.settings_message = Some((
            "Risk parameter updated (demo mode — backend sync pending)".to_string(),
            Instant::now(),
        ));
    }
}

/// Adjust the currently selected risk parameter by a delta.
/// In a real implementation this would POST to the backend API.
fn adjust_risk_param(app: &mut AppState, delta: i32) {
    // For now we show a confirmation dialog since risk changes require validation
    let param_name = match app.settings_row_focus {
        0 => "Max Risk/Trade",
        1 => "Max Daily Drawdown",
        2 => "Max Portfolio Heat",
        3 => "Max Daily Trades",
        4 => "Max Consecutive Losses",
        _ => "Unknown",
    };
    let message = if delta == 0 {
        format!("Confirm change to {}?", param_name)
    } else if delta > 0 {
        format!("Increase {}?", param_name)
    } else {
        format!("Decrease {}?", param_name)
    };
    app.settings_confirm = Some(message);
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

    // Status — still polled for initial connection detection
    match client.get(format!("{}/status", API_BASE)).send() {
        Ok(resp) if resp.status().is_success() => {
            if let Ok(json) = resp.json::<serde_json::Value>() {
                app.status = Some(json);
                app.error = None;
            }
        }
        Ok(_) | Err(_) => {
            app.error = Some("Backend not responding.".into());
        }
    }

    // Health — now streamed via WebSocket; keep as fallback
    if app.health.is_none() {
        if let Ok(resp) = client.get(format!("{}/health", API_BASE)).send() {
            if let Ok(json) = resp.json::<serde_json::Value>() {
                app.health = Some(json);
            }
        }
    }

    // COT — now streamed via WebSocket; only poll on first load if empty
    if app.cot.is_empty() {
        if let Ok(resp) = client.get(format!("{}/cot", API_BASE)).send() {
            if let Ok(json) = resp.json::<Vec<serde_json::Value>>() {
                app.cot = json;
                let mut index: HashMap<String, serde_json::Value> = HashMap::new();
                for entry in app.cot.iter().rev() {
                    if let Some(agent) = entry.get("agent").and_then(|a| a.as_str()) {
                        index.entry(agent.to_string()).or_insert_with(|| entry.clone());
                    }
                }
                app.cot_by_agent = index;
            }
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

    // Policy Cache
    if let Ok(resp) = client.get(format!("{}/policy-cache", API_BASE)).send() {
        if let Ok(json) = resp.json::<serde_json::Value>() {
            app.policy_cache = Some(json);
            app.policy_cache_loaded_at = Some(Instant::now());
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

    // Crypto prices — now streamed via WebSocket; only initial seed if empty
    if app.crypto_prices.is_empty() {
        if let Ok(resp) = client
            .get(format!("{}/crypto/prices?symbols=BTC,ETH,SOL,XRP,ADA,DOGE,AVAX", API_BASE))
            .send()
        {
            if let Ok(json) = resp.json::<serde_json::Value>() {
                if let Some(obj) = json.as_object() {
                    let mut prices: HashMap<String, serde_json::Value> = HashMap::new();
                    for (sym, data) in obj {
                        prices.insert(sym.clone(), data.clone());
                    }
                    app.crypto_prices = prices;
                }
            }
        }
    }

    // Backtest results — only poll when user is viewing the Backtest tab to avoid overhead
    if app.selected_tab == Tab::Backtest as usize {
        if let Ok(resp) = client.get(format!("{}/backtest/results", API_BASE)).send() {
            if let Ok(json) = resp.json::<serde_json::Value>() {
                app.backtest_result = Some(json);
            }
        }
    }

    app.last_poll = Some(Instant::now());
}
