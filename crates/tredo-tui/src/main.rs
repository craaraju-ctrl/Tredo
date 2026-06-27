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
use std::collections::{HashMap, VecDeque};
use std::io::{self, IsTerminal};
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

/// Derive the API base URL from the same env vars the orchestrator uses,
/// so the TUI always connects to the correct port.
/// Falls back to port 8080 if no env var is set.
fn api_base() -> String {
    let port: u16 = std::env::var("PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .or_else(|| {
            std::env::var("WEB_API_ADDR")
                .ok()
                .and_then(|a| a.split(':').next_back().and_then(|p| p.parse().ok()))
        })
        .or_else(|| {
            if let Ok(content) = std::fs::read_to_string("config/tredo.env") {
                for line in content.lines() {
                    let parts: Vec<&str> = line.split('=').collect();
                    if parts.len() == 2 {
                        let key = parts[0].trim().trim_matches('"').trim_matches('\'');
                        let val = parts[1].trim().trim_matches('"').trim_matches('\'');
                        if key == "PORT" {
                            if let Ok(p) = val.parse() {
                                return Some(p);
                            }
                        } else if key == "WEB_API_ADDR" {
                            if let Some(p_str) = val.split(':').next_back() {
                                if let Ok(p) = p_str.parse() {
                                    return Some(p);
                                }
                            }
                        }
                    }
                }
            }
            None
        })
        .unwrap_or(8080);
    format!("http://localhost:{}/api", port)
}

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
            ButtonAction::RunPipeline => "Run full pipeline for all watchlist symbols (sequential)",
            ButtonAction::ForceRefresh => "Refresh all data from backend",
            ButtonAction::TriggerCycle => "Trigger one agentic cycle for the active symbol",
        }
    }
}

pub(crate) const ALL_BUTTONS: &[ButtonAction] = &[
    ButtonAction::RunPipeline,
    ButtonAction::ForceRefresh,
    ButtonAction::TriggerCycle,
];

// ── Background Polling Types ──────────────────────────────────────────────

#[derive(Debug)]
pub(crate) enum PollResult {
    Status(serde_json::Value),
    Health(serde_json::Value),
    Cot(Vec<serde_json::Value>),
    Agents(serde_json::Value),
    Watchlist(Vec<String>),
    Models(serde_json::Value),
    PolicyCache(serde_json::Value),
    Skills(serde_json::Value),
    CryptoPrices(serde_json::Value),
    LatestMetrics(HashMap<String, serde_json::Value>),
    BacktestResult(serde_json::Value),
    Error(String),
}

// ── Application State ─────────────────────────────────────────────────────

#[allow(dead_code)]
#[derive(Debug, Default)]
pub(crate) struct AppState {
    pub(crate) poll_rx: Option<mpsc::Receiver<PollResult>>,
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
    // (confirm_action field removed — pipeline actions now execute immediately)
    pub(crate) policy_cache_filter: String,
    pub(crate) policy_cache_filter_active: bool,
    pub(crate) sparkline_card_areas: Vec<Rect>,
    pub(crate) crypto_prices: HashMap<String, serde_json::Value>,
    pub(crate) latest_metrics: HashMap<String, serde_json::Value>,
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

    /// Service status from ServiceManager (LLM, Kronos connection health)
    pub(crate) service_status: Option<serde_json::Value>,
    /// Previous service_status snapshot for detecting Healthy→Down transitions
    pub(crate) previous_service_status: Option<serde_json::Value>,
    /// Active service alerts: (alert_message, triggered_at)
    pub(crate) service_alerts: Vec<(String, Instant)>,
    /// Live inter-agent communication log: (from_agent, to_agent, message, timestamp)
    /// Ring-buffer of last 200 messages for display on Models/Tree tabs.
    pub(crate) live_comm_log: VecDeque<(String, String, String, Instant)>,
    /// Selected agent index in Models tab comm panel (0 = show all)
    pub(crate) models_comm_agent_filter: usize,
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
    if !io::stdout().is_terminal() {
        anyhow::bail!(
            "TUI requires an interactive terminal.\n\
             Start headless: tredo start --headless\n\
             Attach TUI later from a real terminal: tredo tui"
        );
    }
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
        ws_rx: Some(start_ws_client(&api_base())),
        ws_connected: false,
        ws_cot_count: 0,
        equity_history: Vec::new(),
        pnl_history: Vec::new(),
        win_rate_history: Vec::new(),
        consecutive_losses_history: Vec::new(),
        ws_last_seen: None,
        force_poll: true,
        // Settings page defaults: all 7 agents enabled
        agent_enabled: vec![true; 7],
        settings_panel_focus: 0,
        settings_row_focus: 0,
        risk_editing: false,
        settings_confirm: None,
        settings_message: None,
        latest_metrics: HashMap::new(),
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
                        KeyCode::Tab
                            if !app.policy_cache_filter_active
                                && !app.cot_filter_active
                                && !app.trade_entry_visible =>
                        {
                            reset_settings_if_leaving(app);
                            app.selected_tab = (app.selected_tab + 1) % NUM_TABS;
                            app.scroll_offset = 0;
                        }
                        KeyCode::BackTab
                            if !app.policy_cache_filter_active
                                && !app.cot_filter_active
                                && !app.trade_entry_visible =>
                        {
                            reset_settings_if_leaving(app);
                            app.selected_tab = if app.selected_tab == 0 {
                                NUM_TABS - 1
                            } else {
                                app.selected_tab - 1
                            };
                            app.scroll_offset = 0;
                        }
                        KeyCode::Char(c @ '1'..='9')
                            if !app.policy_cache_filter_active
                                && !app.cot_filter_active
                                && !app.show_overlay
                                && !app.trade_entry_visible =>
                        {
                            reset_settings_if_leaving(app);
                            let idx = (c as usize - '1' as usize).min(NUM_TABS - 1);
                            app.selected_tab = idx;
                            app.scroll_offset = 0;
                        }
                        KeyCode::Char('0')
                            if !app.policy_cache_filter_active
                                && !app.cot_filter_active
                                && !app.show_overlay
                                && !app.trade_entry_visible =>
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
                            if !app.policy_cache_filter_active
                                && !app.cot_filter_active
                                && !app.show_overlay
                                && !app.trade_entry_visible =>
                        {
                            reset_settings_if_leaving(app);
                            app.selected_tab = Tab::Broker as usize;
                            app.scroll_offset = 0;
                        }
                        KeyCode::Char('S')
                            if !app.policy_cache_filter_active
                                && !app.cot_filter_active
                                && !app.show_overlay
                                && !app.trade_entry_visible =>
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
                            if app.selected_tab == Tab::Dashboard as usize
                                && !app.show_overlay
                                && !app.trade_entry_visible =>
                        {
                            let symbols = &app.watchlist;
                            if !symbols.is_empty() {
                                if let Some(pos) = symbols
                                    .iter()
                                    .position(|s| s == &app.trade_entry_symbol)
                                {
                                    let next = if pos == 0 { symbols.len() - 1 } else { pos - 1 };
                                    if let Some(s) = symbols.get(next) {
                                        app.trade_entry_symbol = s.clone();
                                    }
                                } else {
                                    app.trade_entry_symbol = symbols[0].clone();
                                }
                            }
                        }
                        KeyCode::Down
                            if app.selected_tab == Tab::Dashboard as usize
                                && !app.show_overlay
                                && !app.trade_entry_visible =>
                        {
                            let symbols = &app.watchlist;
                            if !symbols.is_empty() {
                                let last = symbols.len() - 1;
                                if let Some(pos) = symbols
                                    .iter()
                                    .position(|s| s == &app.trade_entry_symbol)
                                {
                                    let next = if pos >= last { 0 } else { pos + 1 };
                                    if let Some(s) = symbols.get(next) {
                                        app.trade_entry_symbol = s.clone();
                                    }
                                } else {
                                    app.trade_entry_symbol = symbols[0].clone();
                                }
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
                        KeyCode::Char('+' | '=')
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
                                            if let Some(enabled) =
                                                app.agent_enabled.get_mut(app.settings_row_focus)
                                            {
                                                *enabled = !*enabled;
                                                let name = AGENT_NAMES
                                                    .get(app.settings_row_focus)
                                                    .unwrap_or(&"?");
                                                let state =
                                                    if *enabled { "enabled" } else { "disabled" };
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
                                                if let Some(name) =
                                                    model.get("name").and_then(|n| n.as_str())
                                                {
                                                    let body = serde_json::json!({ "model": name });
                                                    if let Ok(resp) = client
                                                        .post(format!("{}/models/set", api_base()))
                                                        .json(&body)
                                                        .send()
                                                    {
                                                        if let Ok(json) =
                                                            resp.json::<serde_json::Value>()
                                                        {
                                                            if json
                                                                .get("success")
                                                                .and_then(|s| s.as_bool())
                                                                .unwrap_or(false)
                                                            {
                                                                app.current_model =
                                                                    Some(name.to_string());
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
                                // Submit trade (API expects camelCase field names)
                                app.trade_entry_visible = false;
                                let body = serde_json::json!({
                                    "symbol": app.trade_entry_symbol,
                                    "directionStr": app.trade_entry_direction,
                                    "entryPrice": app.trade_entry_price,
                                    "stopLoss": app.trade_entry_sl,
                                    "takeProfit": app.trade_entry_tp,
                                });
                                let (tx, rx) = mpsc::channel();
                                app.action_result_rx = Some(rx);
                                app.action_message = Some((
                                    format!("Submitting {} trade...", app.trade_entry_symbol),
                                    Instant::now(),
                                ));
                                let base_url = api_base();
                                std::thread::spawn(move || {
                                    let client = reqwest::blocking::Client::builder()
                                        .timeout(Duration::from_secs(10))
                                        .build();
                                    let summary = match client {
                                        Ok(c) => match c
                                            .post(format!("{}/trade", base_url))
                                            .json(&body)
                                            .send()
                                        {
                                            Ok(r) => {
                                                let status = r.status();
                                                let text = r.text().unwrap_or_default();
                                                if status.is_success() {
                                                    if let Ok(json) =
                                                        serde_json::from_str::<serde_json::Value>(
                                                            &text,
                                                        )
                                                    {
                                                        let msg = json
                                                            .get("message")
                                                            .and_then(|m| m.as_str())
                                                            .unwrap_or("Trade executed");
                                                        format!("✅ {}", msg)
                                                    } else {
                                                        "✅ Trade submitted".to_string()
                                                    }
                                                } else if let Ok(json) =
                                                    serde_json::from_str::<serde_json::Value>(&text)
                                                {
                                                    let err = json
                                                        .get("error")
                                                        .and_then(|e| e.as_str())
                                                        .unwrap_or(&text);
                                                    format!("❌ Trade failed: {}", err)
                                                } else {
                                                    format!("❌ Trade failed: HTTP {}", status)
                                                }
                                            }
                                            Err(e) => format!("❌ Trade request failed: {}", e),
                                        },
                                        Err(e) => format!("❌ HTTP client error: {}", e),
                                    };
                                    let _ = tx.send(summary);
                                });
                                app.force_poll = true;
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
                                                .post(format!("{}/models/set", api_base()))
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
                                app.settings_message =
                                    Some(("Edit cancelled.".to_string(), Instant::now()));
                            } else if app.trade_entry_visible {
                                app.trade_entry_visible = false;
                            } else if app.policy_cache_filter_active {
                                app.policy_cache_filter_active = false;
                                app.policy_cache_filter.clear();
                            } else if app.cot_filter_active {
                                app.cot_filter_active = false;
                                app.cot_filter.clear();
                            }
                            app.scroll_offset = 0;
                            app.selected_model_index = 0;
                        }
                        KeyCode::Char('y' | 'Y') => {
                            if app.settings_confirm.is_some() {
                                handle_settings_confirm(app);
                            } else if app.selected_tab == Tab::Positions as usize {
                                let opening = !app.trade_entry_visible;
                                app.trade_entry_visible = opening;
                                if opening {
                                    sync_trade_form_from_market(app);
                                }
                            }
                        }
                        KeyCode::Char('n' | 'N') => {
                            if app.settings_confirm.is_some() {
                                app.settings_confirm = None;
                                app.risk_editing = false;
                                app.settings_message =
                                    Some(("Change cancelled.".to_string(), Instant::now()));
                            }
                            app.scroll_offset = 0;
                            app.selected_model_index = 0;
                        }
                        KeyCode::Char('t')
                            if !app.trade_entry_visible
                                && app.selected_tab == Tab::Positions as usize =>
                        {
                            app.trade_entry_visible = true;
                            sync_trade_form_from_market(app);
                        }
                        KeyCode::Up
                            if app.trade_entry_visible
                                && app.selected_tab == Tab::Positions as usize =>
                        {
                            match app.trade_entry_focus {
                                // Symbol: cycle through watchlist (already ordered)
                                0 => {
                                    let symbols = &app.watchlist;
                                    if !symbols.is_empty() {
                                        if let Some(pos) = symbols
                                            .iter()
                                            .position(|s| s == &app.trade_entry_symbol)
                                        {
                                            let next =
                                                if pos == 0 { symbols.len() - 1 } else { pos - 1 };
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
                                    app.trade_entry_direction =
                                        if app.trade_entry_direction == "Long" {
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
                                    let step =
                                        price_step(app.trade_entry_sl.max(app.trade_entry_price));
                                    app.trade_entry_sl += step;
                                }
                                // Take Profit: increase
                                4 => {
                                    let step =
                                        price_step(app.trade_entry_tp.max(app.trade_entry_price));
                                    app.trade_entry_tp += step;
                                }
                                _ => {}
                            }
                        }
                        KeyCode::Down
                            if app.trade_entry_visible
                                && app.selected_tab == Tab::Positions as usize =>
                        {
                            match app.trade_entry_focus {
                                // Symbol: cycle through watchlist (already ordered)
                                0 => {
                                    let symbols = &app.watchlist;
                                    if !symbols.is_empty() {
                                        let last = symbols.len() - 1;
                                        if let Some(pos) = symbols
                                            .iter()
                                            .position(|s| s == &app.trade_entry_symbol)
                                        {
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
                                    app.trade_entry_direction =
                                        if app.trade_entry_direction == "Long" {
                                            "Short".to_string()
                                        } else {
                                            "Long".to_string()
                                        };
                                }
                                // Entry price: decrease by adaptive step
                                2 => {
                                    let step = price_step(app.trade_entry_price);
                                    app.trade_entry_price =
                                        (app.trade_entry_price - step).max(0.01);
                                }
                                // Stop Loss: decrease
                                3 => {
                                    let step =
                                        price_step(app.trade_entry_sl.max(app.trade_entry_price));
                                    app.trade_entry_sl = (app.trade_entry_sl - step).max(0.01);
                                }
                                // Take Profit: decrease
                                4 => {
                                    let step =
                                        price_step(app.trade_entry_tp.max(app.trade_entry_price));
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
                    let clicked_layer = app
                        .pipeline_layer_areas
                        .iter()
                        .enumerate()
                        .find(|(_, area)| {
                            row >= area.y
                                && row < area.y + area.height
                                && col >= area.x
                                && col < area.x + area.width
                        })
                        .map(|(i, _)| i);
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
        let ws_msgs: Vec<String> = app
            .ws_rx
            .as_ref()
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
                app.action_message = Some((
                    "WebSocket connected — real-time updates active".to_string(),
                    Instant::now(),
                ));
            }
            // Backend is alive (WS is receiving data) — clear any stale connection error
            app.error = None;
        }
        // If WS data hasn't arrived in 30s, mark disconnected and re-enable HTTP fallback
        if app.ws_connected
            && app
                .ws_last_seen
                .map(|t| t.elapsed() > Duration::from_secs(45))
                .unwrap_or(false)
        {
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
            app.force_poll = true;
        }

        // ── Process background poll results ──────────────────────────────────────
        let mut poll_results = Vec::new();
        if let Some(ref rx) = app.poll_rx {
            while let Ok(result) = rx.try_recv() {
                poll_results.push(result);
            }
        }
        for result in poll_results {
            match result {
                PollResult::Status(json) => {
                    match &mut app.status {
                        Some(existing) => merge_status_snapshot(existing, json),
                        None => app.status = Some(json),
                    }
                    app.error = None;
                }
                PollResult::Health(json) => app.health = Some(json),
                PollResult::Cot(json) => {
                    app.cot = json;
                    let mut index = HashMap::new();
                    for entry in app.cot.iter().rev() {
                        if let Some(agent) = entry.get("agent").and_then(|a| a.as_str()) {
                            index.entry(agent.to_string()).or_insert_with(|| entry.clone());
                        }
                    }
                    app.cot_by_agent = index;
                }
                PollResult::Agents(json) => app.agents = Some(json),
                PollResult::Watchlist(json) => app.watchlist = json,
                PollResult::Models(json) => {
                    if let Some(models) = json.get("models").and_then(|m| m.as_array()) {
                        app.models = models.clone();
                    }
                    if let Some(current) = json.get("current_model").and_then(|m| m.as_str()) {
                        app.current_model = Some(current.to_string());
                    }
                }
                PollResult::PolicyCache(json) => {
                    app.policy_cache = Some(json);
                    app.policy_cache_loaded_at = Some(Instant::now());
                }
                PollResult::Skills(json) => {
                    if let Some(votes) = json.get("votes").and_then(|v| v.as_array()) {
                        let mut idx = HashMap::new();
                        for vote in votes {
                            if let Some(name) = vote.get("skill_name").and_then(|n| n.as_str()) {
                                idx.entry(name.to_string()).or_insert_with(|| vote.clone());
                            }
                        }
                        app.skill_votes = idx;
                    }
                    app.aggregated_signal = json.get("aggregated").cloned();
                }
                PollResult::CryptoPrices(json) => {
                    if let Some(obj) = json.as_object() {
                        for (sym, data) in obj {
                            if data.get("error").is_some() {
                                continue;
                            }
                            app.crypto_prices.insert(sym.clone(), data.clone());
                        }
                    }
                }
                PollResult::LatestMetrics(json) => app.latest_metrics = json,
                PollResult::BacktestResult(json) => app.backtest_result = Some(json),
                PollResult::Error(msg) => {
                    if app.error.is_none() && !app.ws_connected {
                        app.error = Some(msg);
                    }
                }
            }
        }

        if last_tick.elapsed() >= POLL_INTERVAL || app.force_poll {
            let api = api_base();
            let selected_tab = app.selected_tab;
            let (tx, rx) = mpsc::channel();
            app.poll_rx = Some(rx);
            std::thread::spawn(move || {
                poll_backend_bg(api, selected_tab, tx);
            });
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
            // Autonomous: execute immediately — no y/N confirmation.
            // The backend's medium_loop already runs every 30s, but manual
            // trigger lets the user kick off an immediate cycle on-demand.
            if app.action_running.is_some() {
                app.action_message =
                    Some(("Pipeline already running...".to_string(), Instant::now()));
            } else {
                run_pipeline_action(app, action);
            }
        }
        ButtonAction::ForceRefresh => {
            app.force_poll = true;
            app.action_message = Some(("Refreshing data...".to_string(), Instant::now()));
        }
    }
}

fn run_pipeline_action(app: &mut AppState, action: ButtonAction) {
    app.action_running = Some(action);

    let trigger_symbol_preview = if !app.trade_entry_symbol.is_empty() {
        app.trade_entry_symbol.clone()
    } else if let Some(s) = app.watchlist.first() {
        s.clone()
    } else {
        "BTC".to_string()
    };

    let status_msg = match action {
        ButtonAction::RunPipeline => format!(
            "{} Running pipeline for watchlist (sequential)...",
            action.icon()
        ),
        ButtonAction::TriggerCycle => format!(
            "{} Triggering cycle for {}...",
            action.icon(),
            trigger_symbol_preview
        ),
        _ => format!("{} Working...", action.icon()),
    };
    app.action_message = Some((status_msg, Instant::now()));

    let (tx, rx) = mpsc::channel();
    app.action_result_rx = Some(rx);

    let watchlist: Vec<String> = if app.watchlist.is_empty() {
        vec!["BTC".to_string(), "ETH".to_string()]
    } else {
        app.watchlist.clone()
    };

    let trigger_symbol = if !app.trade_entry_symbol.is_empty() {
        app.trade_entry_symbol.clone()
    } else {
        watchlist
            .first()
            .cloned()
            .unwrap_or_else(|| "BTC".to_string())
    };

    let base_url = api_base();
    std::thread::spawn(move || {
        let timeout_secs = if action == ButtonAction::RunPipeline {
            300
        } else {
            180
        };
        let client = match reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(timeout_secs))
            .build()
        {
            Ok(c) => c,
            Err(_) => {
                let _ = tx.send("Failed to create HTTP client".to_string());
                return;
            }
        };

        let summary = match action {
            ButtonAction::RunPipeline => {
                let url = format!("{}/pipeline/run", base_url);
                let body = serde_json::json!({ "symbols": watchlist });
                match client.post(&url).json(&body).send() {
                    Ok(resp) => {
                        let status = resp.status().as_u16();
                        match resp.json::<serde_json::Value>() {
                            Ok(json) => {
                                let run = json
                                    .get("symbols_run")
                                    .and_then(|v| v.as_u64())
                                    .unwrap_or(0);
                                let trades = json
                                    .get("trades_executed")
                                    .and_then(|v| v.as_u64())
                                    .unwrap_or(0);
                                let ms = json
                                    .get("total_duration_ms")
                                    .and_then(|v| v.as_u64())
                                    .unwrap_or(0);
                                let mut parts = Vec::new();
                                if let Some(results) =
                                    json.get("results").and_then(|v| v.as_array())
                                {
                                    for r in results {
                                        let sym =
                                            r.get("symbol").and_then(|v| v.as_str()).unwrap_or("?");
                                        let act =
                                            r.get("action").and_then(|v| v.as_str()).unwrap_or("?");
                                        let ok = r
                                            .get("success")
                                            .and_then(|v| v.as_bool())
                                            .unwrap_or(false);
                                        let icon = if ok { "✅" } else { "❌" };
                                        parts.push(format!("{icon} {sym}:{act}"));
                                    }
                                }
                                format!(
                                    "Pipeline done: {run} symbols, {trades} trades, {ms}ms | {}",
                                    parts.join(" | ")
                                )
                            }
                            Err(_) => format!("Pipeline HTTP {status} — bad response"),
                        }
                    }
                    Err(e) => format!("Pipeline failed: {e}"),
                }
            }
            ButtonAction::TriggerCycle => {
                let url = format!("{}/trigger_cycle", base_url);
                let body = serde_json::json!({ "symbol": trigger_symbol });
                match client.post(&url).json(&body).send() {
                    Ok(resp) => {
                        let status_ok = resp.status().is_success();
                        let status = resp.status().as_u16();
                        match resp.json::<serde_json::Value>() {
                            Ok(json) => {
                                let sym = json
                                    .get("symbol")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or(&trigger_symbol);
                                let act =
                                    json.get("action").and_then(|v| v.as_str()).unwrap_or("?");
                                let reason =
                                    json.get("reason").and_then(|v| v.as_str()).unwrap_or("");
                                let ok = json
                                    .get("success")
                                    .and_then(|v| v.as_bool())
                                    .unwrap_or(status_ok);
                                let icon = if ok { "✅" } else { "❌" };
                                format!("{icon} {sym} → {act} | {reason}")
                            }
                            Err(_) => format!("Trigger HTTP {status} for {trigger_symbol}"),
                        }
                    }
                    Err(e) => format!("Trigger failed for {trigger_symbol}: {e}"),
                }
            }
            _ => "Unknown action".to_string(),
        };

        let _ = tx.send(summary);
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
            let agent_name = json.get("agent").and_then(|a| a.as_str()).unwrap_or("?").to_string();
            app.cot_by_agent.insert(agent_name.clone(), json.clone());
            // Extract action/reason for the live comm log
            let action = json.get("action").and_then(|a| a.as_str()).unwrap_or("").to_string();
            let reason = json
                .get("reason")
                .and_then(|r| r.as_str())
                .or_else(|| json.get("message").and_then(|m| m.as_str()))
                .unwrap_or("")
                .chars()
                .take(120)
                .collect::<String>();
            // Determine next agent in pipeline for "to" label
            let to_agent = if let Some(to) = json.get("to").and_then(|t| t.as_str()) {
                to.to_string()
            } else {
                match agent_name.as_str() {
                    n if n.contains("Tredo") || n.contains("Meta") => "Identifier".to_string(),
                    n if n.contains("Identifier") => "Verifier".to_string(),
                    n if n.contains("Verifier") => "Executer".to_string(),
                    n if n.contains("Executer") => "Guardian".to_string(),
                    n if n.contains("Guardian") => "System".to_string(),
                    _ => "System".to_string(),
                }
            };
            let msg = if action.is_empty() {
                reason.clone()
            } else {
                format!("[{}] {}", action, reason)
            };
            if !msg.trim().is_empty() {
                app.live_comm_log.push_back((agent_name, to_agent, msg, Instant::now()));
                if app.live_comm_log.len() > 200 {
                    app.live_comm_log.pop_front();
                }
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
            // Merge portfolio snapshot into status (dashboard + positions tab)
            let status = app.status.get_or_insert_with(|| serde_json::json!({}));
            merge_portfolio_into_status(status, &json);

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
        "service_status" => {
            // Detect Healthy→Down transitions and push alerts
            let prev = app.previous_service_status.take();
            if let (Some(prev_services), Some(new_services)) = (
                prev.as_ref().and_then(|p| p.get("services")),
                json.get("services"),
            ) {
                if let Some(prev_obj) = prev_services.as_object() {
                    if let Some(new_obj) = new_services.as_object() {
                        for (key, new_svc) in new_obj {
                            let new_status =
                                new_svc.get("status").and_then(|s| s.as_str()).unwrap_or("");
                            let prev_status = prev_obj
                                .get(key)
                                .and_then(|s| s.get("status"))
                                .and_then(|s| s.as_str())
                                .unwrap_or("");
                            // Transition: Healthy/Degraded → Down
                            if (prev_status == "Healthy" || prev_status == "Degraded")
                                && new_status == "Down"
                            {
                                let label = match key.as_str() {
                                    k if k.contains("llm") => "LLM",
                                    "kronos" => "Kronos",
                                    _ => key.as_str(),
                                };
                                let msg = format!("⚠ {} SERVICE DOWN — Health check failed", label);
                                app.service_alerts.push((msg, Instant::now()));
                                // Keep max 5 alerts, remove oldest
                                while app.service_alerts.len() > 5 {
                                    app.service_alerts.remove(0);
                                }
                            }
                        }
                    }
                }
            }
            // Store current as previous for next comparison
            app.previous_service_status = app.service_status.clone();
            app.service_status = Some(json);
        }
        // "ping" messages are keepalives — no action needed
        _ => {}
    }
}

/// Seed the inline trade form from live market prices (crypto feed or last poll).
fn sync_trade_form_from_market(app: &mut AppState) {
    let price = app
        .crypto_prices
        .get(&app.trade_entry_symbol)
        .and_then(|p| p.get("price").and_then(|v| v.as_f64()))
        .filter(|p| *p > 0.0)
        .or_else(|| {
            app.status.as_ref().and_then(|s| {
                s.get("open_positions")
                    .and_then(|v| v.as_array())
                    .and_then(|arr| {
                        arr.iter().find(|p| {
                            p.get("symbol").and_then(|s| s.as_str())
                                == Some(app.trade_entry_symbol.as_str())
                        })
                    })
                    .and_then(|p| p.get("current_price").and_then(|v| v.as_f64()))
            })
        });

    if let Some(price) = price {
        app.trade_entry_price = price;
        let stop_pct = 0.015;
        let tp_pct = 0.03;
        if app.trade_entry_direction == "Long" {
            app.trade_entry_sl = price * (1.0 - stop_pct);
            app.trade_entry_tp = price * (1.0 + tp_pct);
        } else {
            app.trade_entry_sl = price * (1.0 + stop_pct);
            app.trade_entry_tp = price * (1.0 - tp_pct);
        }
    }
}

/// Merge portfolio fields from a WS or HTTP snapshot into app.status.
fn merge_portfolio_into_status(status: &mut serde_json::Value, json: &serde_json::Value) {
    if let Some(obj) = status.as_object_mut() {
        for key in [
            "total_equity",
            "cash_balance",
            "daily_pnl",
            "daily_pnl_pct",
            "open_positions",
            "open_positions_count",
            "total_trades_today",
            "trades_today",
            "winning_trades_today",
            "losing_trades_today",
            "consecutive_losses",
            "win_rate",
            "max_drawdown_today",
            "trading_enabled",
        ] {
            if let Some(v) = json.get(key) {
                obj.insert(key.to_string(), v.clone());
            }
        }
        if obj.get("total_trades_today").is_none() {
            if let Some(v) = json.get("trades_today") {
                obj.insert("total_trades_today".to_string(), v.clone());
            }
        }
    }
}

/// Merge top-level fields from a poll response into existing status without wiping WS data.
fn merge_status_snapshot(existing: &mut serde_json::Value, update: serde_json::Value) {
    if let Some(update_obj) = update.as_object() {
        let target = existing.as_object_mut().expect("status must be an object");
        for (k, v) in update_obj {
            target.insert(k.clone(), v.clone());
        }
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

fn poll_backend_bg(api: String, selected_tab: usize, tx: mpsc::Sender<PollResult>) {
    let client = match reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(3))
        .build()
    {
        Ok(c) => c,
        Err(_) => {
            let _ = tx.send(PollResult::Error("Failed to create HTTP client.".into()));
            return;
        }
    };

    // 1. Status
    match client.get(format!("{}/status", api)).send() {
        Ok(resp) if resp.status().is_success() => {
            if let Ok(json) = resp.json::<serde_json::Value>() {
                let _ = tx.send(PollResult::Status(json));
            }
        }
        Ok(_) | Err(_) => {
            let _ = tx.send(PollResult::Error("Backend not responding. Retrying...".into()));
        }
    }

    // 2. Health
    if let Ok(resp) = client.get(format!("{}/health", api)).send() {
        if let Ok(json) = resp.json::<serde_json::Value>() {
            let _ = tx.send(PollResult::Health(json));
        }
    }

    // 3. COT
    if let Ok(resp) = client.get(format!("{}/cot", api)).send() {
        if let Ok(json) = resp.json::<Vec<serde_json::Value>>() {
            let _ = tx.send(PollResult::Cot(json));
        }
    }

    // 4. Agents
    if let Ok(resp) = client.get(format!("{}/agents", api)).send() {
        if let Ok(json) = resp.json::<serde_json::Value>() {
            let _ = tx.send(PollResult::Agents(json));
        }
    }

    // 5. Watchlist
    if let Ok(resp) = client.get(format!("{}/watchlist", api)).send() {
        if let Ok(json) = resp.json::<Vec<String>>() {
            let _ = tx.send(PollResult::Watchlist(json));
        }
    }

    // 6. Models
    if let Ok(resp) = client.get(format!("{}/models", api)).send() {
        if let Ok(json) = resp.json::<serde_json::Value>() {
            let _ = tx.send(PollResult::Models(json));
        }
    }

    // 7. Policy Cache
    if let Ok(resp) = client.get(format!("{}/policy-cache", api)).send() {
        if let Ok(json) = resp.json::<serde_json::Value>() {
            let _ = tx.send(PollResult::PolicyCache(json));
        }
    }

    // 8. Skill scores
    if let Ok(resp) = client.get(format!("{}/skills", api)).send() {
        if let Ok(json) = resp.json::<serde_json::Value>() {
            let _ = tx.send(PollResult::Skills(json));
        }
    }

    // 9. Crypto prices
    if let Ok(resp) = client
        .get(format!(
            "{}/crypto/prices?symbols=BTC,ETH,SOL,BNB,XRP,ADA,DOGE,AVAX,MATIC,LINK,DOT,ATOM,LTC,UNI,AAVE,NEAR,APT,ARB,OP,SUI,INJ,TON,TRX,XLM,PEPE,SHIB",
            &api
        ))
        .send()
    {
        if let Ok(json) = resp.json::<serde_json::Value>() {
            let _ = tx.send(PollResult::CryptoPrices(json));
        }
    }

    // 10. Technical Indicators / Metrics
    if let Ok(resp) = client.get(format!("{}/metrics", api)).send() {
        if let Ok(json) = resp.json::<HashMap<String, serde_json::Value>>() {
            let _ = tx.send(PollResult::LatestMetrics(json));
        }
    }

    // 11. Backtest results
    if selected_tab == Tab::Backtest as usize {
        if let Ok(resp) = client.get(format!("{}/backtest/results", api)).send() {
            if let Ok(json) = resp.json::<serde_json::Value>() {
                let _ = tx.send(PollResult::BacktestResult(json));
            }
        }
    }
}
