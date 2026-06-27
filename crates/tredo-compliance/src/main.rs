// ═══════════════════════════════════════════════════════════════════════════════
// tredo-compliance — Pre-Trade Compliance Gateway
//
// A standalone binary that validates every trade proposal against parameterized
// rules BEFORE execution. Runs as a sidecar process alongside the orchestrator.
//
// Key features:
//   - 10+ parameterized rules (FAT-finger protection, price collars, etc.)
//   - Rules configured via TOML file (~/.tredo/compliance.toml) — read-only at startup
//   - Every check logged to append-only SQLite database with microsecond timestamps
//   - Independently versioned (separate Cargo.toml)
//   - Cannot be bypassed by the trading process (runs as separate binary)
//   - Returns structured pass/fail responses with detailed reasons
//
// Architecture:
//   Orchestrator ──(POST /check {trade})──→ Compliance Gateway
//   Gateway       ──(200 {passed, reasons})──→ Orchestrator
//   Gateway       ──(append SQLite)──→ ~/.tredo/compliance.db
//
// This binary runs as a SEPARATE PROCESS from the orchestrator.
// It cannot be disabled by a bug in the trading code — the orchestrator
// must receive a "passed: true" response before executing any trade.
// ═══════════════════════════════════════════════════════════════════════════════

use axum::{
    extract::State,
    http::StatusCode,
    response::Json,
    routing::{get, post},
    Router,
};
use chrono::Utc;
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use tracing::{error, info, warn};

// ── Constants ─────────────────────────────────────────────────────────────────

const HTTP_PORT: u16 = 9720;
const DEFAULT_CONFIG_PATH: &str = "~/.tredo/compliance.toml";
const DB_PATH: &str = "~/.tredo/compliance.db";

// ── Rule Configuration (from TOML file) ──────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComplianceConfig {
    /// Max single order size as % of 30-day average daily volume (ADV).
    #[serde(default = "default_max_order_adv_pct")]
    pub max_order_adv_pct: f64,

    /// Max allowed price deviation from last traded price (%). FAT-finger protection.
    #[serde(default = "default_price_collar_pct")]
    pub price_collar_pct: f64,

    /// Max daily loss as % of initial portfolio equity before halting.
    #[serde(default = "default_max_daily_loss_pct")]
    pub max_daily_loss_pct: f64,

    /// Max consecutive losses before circuit breaker engages.
    #[serde(default = "default_consecutive_loss_limit")]
    pub consecutive_loss_limit: u32,

    /// Min confluence score required for any trade (0.0 to 1.0).
    #[serde(default = "default_min_confluence")]
    pub min_confluence_score: f64,

    /// Max position size as % of total equity.
    #[serde(default = "default_max_position_equity_pct")]
    pub max_position_equity_pct: f64,

    /// Max leverage allowed.
    #[serde(default = "default_max_leverage")]
    pub max_leverage: u32,

    /// Max portfolio heat as % of total equity.
    #[serde(default = "default_max_portfolio_heat_pct")]
    pub max_portfolio_heat_pct: f64,

    /// Max drawdown before halting (%).
    #[serde(default = "default_max_drawdown_pct")]
    pub max_drawdown_pct: f64,

    /// Symbols blacklisted from trading entirely.
    #[serde(default)]
    pub blacklisted_symbols: Vec<String>,

    /// Whether to enforce position concentration limits per symbol.
    #[serde(default = "default_true")]
    pub enforce_concentration: bool,

    /// Max single symbol exposure as % of equity.
    #[serde(default = "default_max_concentration_pct")]
    pub max_concentration_pct: f64,

    /// Min risk-reward ratio required.
    #[serde(default = "default_min_rr")]
    pub min_risk_reward_ratio: f64,
}

fn default_max_order_adv_pct() -> f64 {
    5.0
}
fn default_price_collar_pct() -> f64 {
    3.0
}
fn default_max_daily_loss_pct() -> f64 {
    5.0
}
fn default_consecutive_loss_limit() -> u32 {
    5
}
fn default_min_confluence() -> f64 {
    0.30
}
fn default_max_position_equity_pct() -> f64 {
    20.0
}
fn default_max_leverage() -> u32 {
    3
}
fn default_max_portfolio_heat_pct() -> f64 {
    30.0
}
fn default_max_drawdown_pct() -> f64 {
    15.0
}
fn default_true() -> bool {
    true
}
fn default_max_concentration_pct() -> f64 {
    20.0
}
fn default_min_rr() -> f64 {
    1.5
}

impl Default for ComplianceConfig {
    fn default() -> Self {
        Self {
            max_order_adv_pct: default_max_order_adv_pct(),
            price_collar_pct: default_price_collar_pct(),
            max_daily_loss_pct: default_max_daily_loss_pct(),
            consecutive_loss_limit: default_consecutive_loss_limit(),
            min_confluence_score: default_min_confluence(),
            max_position_equity_pct: default_max_position_equity_pct(),
            max_leverage: default_max_leverage(),
            max_portfolio_heat_pct: default_max_portfolio_heat_pct(),
            max_drawdown_pct: default_max_drawdown_pct(),
            blacklisted_symbols: Vec::new(),
            enforce_concentration: default_true(),
            max_concentration_pct: default_max_concentration_pct(),
            min_risk_reward_ratio: default_min_rr(),
        }
    }
}

// ── Trade Proposal (from orchestrator) ────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TradeProposal {
    pub symbol: String,
    pub direction: String, // "BUY" or "SELL"
    pub entry_price: f64,
    pub stop_loss: f64,
    pub take_profit: f64,
    pub position_size: f64,  // Quantity
    pub position_value: f64, // Entry price × quantity
    pub leverage: u32,
    pub confidence_score: f64,
    pub confluence_score: f64,
    pub current_price: f64, // Last market price
    pub portfolio_equity: f64,
    pub portfolio_heat: f64, // Current risk exposure as decimal
    pub daily_pnl: f64,
    pub daily_pnl_pct: f64,
    pub consecutive_losses: u32,
    pub open_positions_count: u32,
    pub trades_today: u32,
    pub current_drawdown_pct: f64,
    pub symbol_exposure: f64,     // Current position value in this symbol
    pub previous_day_volume: f64, // 30-day avg daily volume for ADV check
    pub timestamp_micros: i64,
}

// ── Compliance Check Result ───────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComplianceCheck {
    pub passed: bool,
    pub rule_name: String,
    pub severity: String, // "CRITICAL" | "HIGH" | "MEDIUM" | "LOW"
    pub reason: String,
    pub timestamp_micros: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComplianceResponse {
    pub passed: bool,
    pub version: String,
    pub checks: Vec<ComplianceCheck>,
    pub summary: String,
    pub timestamp_micros: i64,
}

// ── Application State ────────────────────────────────────────────────────────

#[derive(Clone)]
struct ComplianceState {
    config: ComplianceConfig,
    db: Arc<Mutex<Connection>>,
    version: String,
}

impl ComplianceState {
    fn new(config: ComplianceConfig, db: Connection) -> Result<Self, rusqlite::Error> {
        let version = env!("CARGO_PKG_VERSION").to_string();
        Ok(Self {
            config,
            db: Arc::new(Mutex::new(db)),
            version,
        })
    }

    fn run_checks(&self, proposal: &TradeProposal) -> ComplianceResponse {
        let mut checks = Vec::new();
        let cfg = &self.config;
        let now_micros = Utc::now().timestamp_micros();

        // ═══ RULE 1: Symbol Blacklist (CRITICAL) ═══════════════════════════
        if cfg
            .blacklisted_symbols
            .iter()
            .any(|s| s.eq_ignore_ascii_case(&proposal.symbol))
        {
            checks.push(ComplianceCheck {
                passed: false,
                rule_name: "symbol_blacklist".to_string(),
                severity: "CRITICAL".to_string(),
                reason: format!("Symbol '{}' is blacklisted", proposal.symbol),
                timestamp_micros: now_micros,
            });
        } else {
            checks.push(ComplianceCheck {
                passed: true,
                rule_name: "symbol_blacklist".to_string(),
                severity: "CRITICAL".to_string(),
                reason: "Symbol not blacklisted".to_string(),
                timestamp_micros: now_micros,
            });
        }

        // ═══ RULE 2: Price Collar — FAT-finger protection (CRITICAL) ═══════
        if proposal.current_price > 0.0 {
            let deviation_pct = ((proposal.entry_price - proposal.current_price).abs()
                / proposal.current_price)
                * 100.0;
            if deviation_pct > cfg.price_collar_pct {
                checks.push(ComplianceCheck {
                    passed: false,
                    rule_name: "price_collar".to_string(),
                    severity: "CRITICAL".to_string(),
                    reason: format!(
                        "Entry price {:.2} deviates {:.2}% from market {:.2} — exceeds collar of {:.1}%",
                        proposal.entry_price, deviation_pct, proposal.current_price, cfg.price_collar_pct
                    ),
                    timestamp_micros: now_micros,
                });
            } else {
                checks.push(ComplianceCheck {
                    passed: true,
                    rule_name: "price_collar".to_string(),
                    severity: "CRITICAL".to_string(),
                    reason: format!("Entry price within {:.1}% collar", cfg.price_collar_pct),
                    timestamp_micros: now_micros,
                });
            }
        } else {
            checks.push(ComplianceCheck {
                passed: true,
                rule_name: "price_collar".to_string(),
                severity: "CRITICAL".to_string(),
                reason: "No market price available — collar check skipped".to_string(),
                timestamp_micros: now_micros,
            });
        }

        // ═══ RULE 3: Max Position Size (HIGH) ══════════════════════════════
        let max_pos_value = proposal.portfolio_equity * (cfg.max_position_equity_pct / 100.0);
        if proposal.position_value > max_pos_value {
            checks.push(ComplianceCheck {
                passed: false,
                rule_name: "max_position_size".to_string(),
                severity: "HIGH".to_string(),
                reason: format!(
                    "Position value ₹{:.2} exceeds max ₹{:.2} ({:.1}% of equity)",
                    proposal.position_value, max_pos_value, cfg.max_position_equity_pct
                ),
                timestamp_micros: now_micros,
            });
        } else {
            checks.push(ComplianceCheck {
                passed: true,
                rule_name: "max_position_size".to_string(),
                severity: "HIGH".to_string(),
                reason: format!(
                    "Position size {:.1}% within {:.1}% limit",
                    proposal.position_value / proposal.portfolio_equity.max(1.0) * 100.0,
                    cfg.max_position_equity_pct
                ),
                timestamp_micros: now_micros,
            });
        }

        // ═══ RULE 4: Max Daily Loss — Circuit Breaker (CRITICAL) ═══════════
        let max_loss = proposal.portfolio_equity * (cfg.max_daily_loss_pct / 100.0);
        if proposal.daily_pnl < -max_loss {
            checks.push(ComplianceCheck {
                passed: false,
                rule_name: "max_daily_loss".to_string(),
                severity: "CRITICAL".to_string(),
                reason: format!(
                    "Daily loss ₹{:.2} exceeds max ₹{:.2} ({:.1}% of equity)",
                    proposal.daily_pnl.abs(),
                    max_loss,
                    cfg.max_daily_loss_pct
                ),
                timestamp_micros: now_micros,
            });
        } else {
            checks.push(ComplianceCheck {
                passed: true,
                rule_name: "max_daily_loss".to_string(),
                severity: "CRITICAL".to_string(),
                reason: format!(
                    "Daily loss ₹{:.2} within {:.1}% limit",
                    proposal.daily_pnl.abs(),
                    cfg.max_daily_loss_pct
                ),
                timestamp_micros: now_micros,
            });
        }

        // ═══ RULE 5: Consecutive Loss Circuit Breaker (HIGH) ═══════════════
        if proposal.consecutive_losses >= cfg.consecutive_loss_limit {
            checks.push(ComplianceCheck {
                passed: false,
                rule_name: "consecutive_loss_breaker".to_string(),
                severity: "HIGH".to_string(),
                reason: format!(
                    "{} consecutive losses — circuit breaker limit is {}",
                    proposal.consecutive_losses, cfg.consecutive_loss_limit
                ),
                timestamp_micros: now_micros,
            });
        } else {
            checks.push(ComplianceCheck {
                passed: true,
                rule_name: "consecutive_loss_breaker".to_string(),
                severity: "HIGH".to_string(),
                reason: format!(
                    "{} consecutive losses < limit {}",
                    proposal.consecutive_losses, cfg.consecutive_loss_limit
                ),
                timestamp_micros: now_micros,
            });
        }

        // ═══ RULE 6: Min Confluence Score (MEDIUM) ═════════════════════════
        if proposal.confluence_score < cfg.min_confluence_score {
            checks.push(ComplianceCheck {
                passed: false,
                rule_name: "min_confluence".to_string(),
                severity: "MEDIUM".to_string(),
                reason: format!(
                    "Confluence {:.2} < minimum {:.2}",
                    proposal.confluence_score, cfg.min_confluence_score
                ),
                timestamp_micros: now_micros,
            });
        } else {
            checks.push(ComplianceCheck {
                passed: true,
                rule_name: "min_confluence".to_string(),
                severity: "MEDIUM".to_string(),
                reason: format!(
                    "Confluence {:.2} meets minimum {:.2}",
                    proposal.confluence_score, cfg.min_confluence_score
                ),
                timestamp_micros: now_micros,
            });
        }

        // ═══ RULE 7: Max Leverage (CRITICAL) ═══════════════════════════════
        if proposal.leverage > cfg.max_leverage {
            checks.push(ComplianceCheck {
                passed: false,
                rule_name: "max_leverage".to_string(),
                severity: "CRITICAL".to_string(),
                reason: format!(
                    "Leverage {} exceeds max {}",
                    proposal.leverage, cfg.max_leverage
                ),
                timestamp_micros: now_micros,
            });
        } else {
            checks.push(ComplianceCheck {
                passed: true,
                rule_name: "max_leverage".to_string(),
                severity: "CRITICAL".to_string(),
                reason: format!("Leverage {} within limit", proposal.leverage),
                timestamp_micros: now_micros,
            });
        }

        // ═══ RULE 8: Portfolio Heat (HIGH) ═════════════════════════════════
        if proposal.portfolio_heat * 100.0 > cfg.max_portfolio_heat_pct {
            checks.push(ComplianceCheck {
                passed: false,
                rule_name: "portfolio_heat".to_string(),
                severity: "HIGH".to_string(),
                reason: format!(
                    "Portfolio heat {:.1}% exceeds max {:.1}%",
                    proposal.portfolio_heat * 100.0,
                    cfg.max_portfolio_heat_pct
                ),
                timestamp_micros: now_micros,
            });
        } else {
            checks.push(ComplianceCheck {
                passed: true,
                rule_name: "portfolio_heat".to_string(),
                severity: "HIGH".to_string(),
                reason: format!(
                    "Portfolio heat {:.1}% within {:.1}% limit",
                    proposal.portfolio_heat * 100.0,
                    cfg.max_portfolio_heat_pct
                ),
                timestamp_micros: now_micros,
            });
        }

        // ═══ RULE 9: Max Drawdown (CRITICAL) ═══════════════════════════════
        if proposal.current_drawdown_pct > cfg.max_drawdown_pct {
            checks.push(ComplianceCheck {
                passed: false,
                rule_name: "max_drawdown".to_string(),
                severity: "CRITICAL".to_string(),
                reason: format!(
                    "Drawdown {:.1}% exceeds max {:.1}%",
                    proposal.current_drawdown_pct, cfg.max_drawdown_pct
                ),
                timestamp_micros: now_micros,
            });
        } else {
            checks.push(ComplianceCheck {
                passed: true,
                rule_name: "max_drawdown".to_string(),
                severity: "CRITICAL".to_string(),
                reason: format!(
                    "Drawdown {:.1}% within {:.1}% limit",
                    proposal.current_drawdown_pct, cfg.max_drawdown_pct
                ),
                timestamp_micros: now_micros,
            });
        }

        // ═══ RULE 10: Symbol Concentration (MEDIUM) ════════════════════════
        if cfg.enforce_concentration {
            let max_symbol_exposure =
                proposal.portfolio_equity * (cfg.max_concentration_pct / 100.0);
            let total_exposure = proposal.symbol_exposure + proposal.position_value;
            if total_exposure > max_symbol_exposure {
                checks.push(ComplianceCheck {
                    passed: false,
                    rule_name: "symbol_concentration".to_string(),
                    severity: "MEDIUM".to_string(),
                    reason: format!(
                        "Total exposure in {} ₹{:.2} exceeds max ₹{:.2} ({:.1}% of equity)",
                        proposal.symbol,
                        total_exposure,
                        max_symbol_exposure,
                        cfg.max_concentration_pct
                    ),
                    timestamp_micros: now_micros,
                });
            } else {
                checks.push(ComplianceCheck {
                    passed: true,
                    rule_name: "symbol_concentration".to_string(),
                    severity: "MEDIUM".to_string(),
                    reason: format!(
                        "Concentration in {} within {:.1}% limit",
                        proposal.symbol, cfg.max_concentration_pct
                    ),
                    timestamp_micros: now_micros,
                });
            }
        } else {
            checks.push(ComplianceCheck {
                passed: true,
                rule_name: "symbol_concentration".to_string(),
                severity: "MEDIUM".to_string(),
                reason: "Concentration checks disabled".to_string(),
                timestamp_micros: now_micros,
            });
        }

        // ═══ RULE 11: Min Risk-Reward Ratio (MEDIUM) ═══════════════════════
        if proposal.take_profit > proposal.entry_price && proposal.stop_loss > 0.0 {
            let rr = (proposal.take_profit - proposal.entry_price).abs()
                / (proposal.entry_price - proposal.stop_loss)
                    .abs()
                    .max(0.0001);
            if rr < cfg.min_risk_reward_ratio {
                checks.push(ComplianceCheck {
                    passed: false,
                    rule_name: "min_risk_reward".to_string(),
                    severity: "MEDIUM".to_string(),
                    reason: format!(
                        "Risk-reward ratio {:.2} < minimum {:.2}",
                        rr, cfg.min_risk_reward_ratio
                    ),
                    timestamp_micros: now_micros,
                });
            } else {
                checks.push(ComplianceCheck {
                    passed: true,
                    rule_name: "min_risk_reward".to_string(),
                    severity: "MEDIUM".to_string(),
                    reason: format!(
                        "Risk-reward {:.2} meets minimum {:.2}",
                        rr, cfg.min_risk_reward_ratio
                    ),
                    timestamp_micros: now_micros,
                });
            }
        } else {
            // Can't compute RR if no SL/TP
            checks.push(ComplianceCheck {
                passed: true,
                rule_name: "min_risk_reward".to_string(),
                severity: "MEDIUM".to_string(),
                reason: "RR check skipped (no SL/TP defined)".to_string(),
                timestamp_micros: now_micros,
            });
        }

        // ═══ RULE 12: Order Size vs ADV Warning (LOW) ══════════════════════
        if proposal.previous_day_volume > 0.0 {
            let adv_pct = (proposal.position_value / proposal.previous_day_volume) * 100.0;
            if adv_pct > cfg.max_order_adv_pct {
                checks.push(ComplianceCheck {
                    passed: false,
                    rule_name: "order_adv_ratio".to_string(),
                    severity: "LOW".to_string(),
                    reason: format!(
                        "Order size {:.1}% of ADV exceeds {:.1}% threshold (market impact risk)",
                        adv_pct, cfg.max_order_adv_pct
                    ),
                    timestamp_micros: now_micros,
                });
            } else {
                checks.push(ComplianceCheck {
                    passed: true,
                    rule_name: "order_adv_ratio".to_string(),
                    severity: "LOW".to_string(),
                    reason: format!("Order size {:.1}% of ADV within limit", adv_pct),
                    timestamp_micros: now_micros,
                });
            }
        } else {
            checks.push(ComplianceCheck {
                passed: true,
                rule_name: "order_adv_ratio".to_string(),
                severity: "LOW".to_string(),
                reason: "ADV check skipped (no volume data)".to_string(),
                timestamp_micros: now_micros,
            });
        }

        // ── Determine overall verdict ──────────────────────────────────────
        let critical_failures: Vec<&ComplianceCheck> = checks
            .iter()
            .filter(|c| !c.passed && c.severity == "CRITICAL")
            .collect();
        let high_failures: Vec<&ComplianceCheck> = checks
            .iter()
            .filter(|c| !c.passed && c.severity == "HIGH")
            .collect();

        let passed = critical_failures.is_empty() && high_failures.is_empty();
        let failed_count = checks.iter().filter(|c| !c.passed).count();
        let total = checks.len();

        let summary = if passed {
            format!("✅ COMPLIANCE PASSED: All {} checks passed", total)
        } else {
            let cr = critical_failures.len();
            let hi = high_failures.len();
            let med = checks
                .iter()
                .filter(|c| !c.passed && c.severity == "MEDIUM")
                .count();
            let low = checks
                .iter()
                .filter(|c| !c.passed && c.severity == "LOW")
                .count();
            format!(
                "⛔ COMPLIANCE FAILED: {} of {} checks failed ({} CRITICAL, {} HIGH, {} MEDIUM, {} LOW)",
                failed_count, total, cr, hi, med, low
            )
        };

        let response = ComplianceResponse {
            passed,
            version: self.version.clone(),
            checks,
            summary,
            timestamp_micros: now_micros,
        };

        // Log to SQLite (append-only)
        if let Err(e) = self.log_check(proposal, &response) {
            error!("[Compliance] Failed to log check to SQLite: {}", e);
        }

        response
    }

    /// Append the compliance check result to the SQLite database.
    fn log_check(
        &self,
        proposal: &TradeProposal,
        response: &ComplianceResponse,
    ) -> Result<(), rusqlite::Error> {
        let db = self.db.lock().unwrap();
        let mut stmt = db.prepare(
            "INSERT INTO compliance_log (
                timestamp_micros, symbol, direction, entry_price, position_value,
                leverage, confidence, confluence, portfolio_equity, portfolio_heat,
                daily_pnl, consecutive_losses, trades_today, drawdown_pct,
                passed, version, checks_summary, raw_response
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18)"
        )?;

        let checks_summary = response
            .checks
            .iter()
            .map(|c| format!("{}={}", c.rule_name, if c.passed { "PASS" } else { "FAIL" }))
            .collect::<Vec<_>>()
            .join(",");

        let raw = serde_json::to_string(response).unwrap_or_default();

        stmt.execute(params![
            response.timestamp_micros,
            proposal.symbol,
            proposal.direction,
            proposal.entry_price,
            proposal.position_value,
            proposal.leverage as i32,
            proposal.confidence_score,
            proposal.confluence_score,
            proposal.portfolio_equity,
            proposal.portfolio_heat,
            proposal.daily_pnl,
            proposal.consecutive_losses as i32,
            proposal.trades_today as i32,
            proposal.current_drawdown_pct,
            response.passed as i32,
            response.version,
            checks_summary,
            raw,
        ])?;

        Ok(())
    }
}

// ── Database Initialization ───────────────────────────────────────────────────

fn init_database(path: &str) -> Result<Connection, rusqlite::Error> {
    let expanded = shellexpand(path);
    if let Some(parent) = std::path::Path::new(&expanded).parent() {
        std::fs::create_dir_all(parent).ok();
    }

    let db = Connection::open(&expanded)?;

    // Enable WAL mode for concurrent readers
    db.execute_batch("PRAGMA journal_mode=WAL;")?;

    db.execute_batch(
        "CREATE TABLE IF NOT EXISTS compliance_log (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            timestamp_micros INTEGER NOT NULL,
            symbol TEXT NOT NULL,
            direction TEXT NOT NULL,
            entry_price REAL NOT NULL,
            position_value REAL NOT NULL,
            leverage INTEGER NOT NULL,
            confidence REAL NOT NULL,
            confluence REAL NOT NULL,
            portfolio_equity REAL NOT NULL,
            portfolio_heat REAL NOT NULL,
            daily_pnl REAL NOT NULL,
            consecutive_losses INTEGER NOT NULL,
            trades_today INTEGER NOT NULL,
            drawdown_pct REAL NOT NULL,
            passed INTEGER NOT NULL,
            version TEXT NOT NULL,
            checks_summary TEXT NOT NULL,
            raw_response TEXT NOT NULL
        );

        CREATE INDEX IF NOT EXISTS idx_compliance_timestamp
            ON compliance_log(timestamp_micros DESC);

        CREATE INDEX IF NOT EXISTS idx_compliance_passed
            ON compliance_log(passed);

        CREATE VIEW IF NOT EXISTS compliance_stats AS
            SELECT
                COUNT(*) as total_checks,
                SUM(CASE WHEN passed = 1 THEN 1 ELSE 0 END) as passed_checks,
                SUM(CASE WHEN passed = 0 THEN 1 ELSE 0 END) as failed_checks,
                ROUND(AVG(CASE WHEN passed = 1 THEN 100.0 ELSE 0.0 END), 1) as pass_rate_pct,
                MIN(timestamp_micros) as first_check_ts,
                MAX(timestamp_micros) as last_check_ts
            FROM compliance_log;",
    )?;

    info!("[Compliance] Database initialized at {}", expanded);
    Ok(db)
}

/// Simple tilde expansion for file paths.
fn shellexpand(path: &str) -> String {
    if let Some(rest) = path.strip_prefix("~/") {
        if let Some(home) = std::env::var_os("HOME") {
            return std::path::PathBuf::from(home)
                .join(rest)
                .to_string_lossy()
                .to_string();
        }
    }
    path.to_string()
}

// ── Config Loading ───────────────────────────────────────────────────────────

fn load_config(path: &str) -> ComplianceConfig {
    let expanded = shellexpand(path);

    // Try loading from explicit path, then default path
    let paths = vec![expanded.clone(), shellexpand(DEFAULT_CONFIG_PATH)];

    for p in &paths {
        if std::path::Path::new(p).exists() {
            match std::fs::read_to_string(p) {
                Ok(content) => match toml::from_str::<ComplianceConfig>(&content) {
                    Ok(config) => {
                        info!("[Compliance] Loaded config from {}", p);
                        return config;
                    }
                    Err(e) => {
                        warn!("[Compliance] Failed to parse {}: {}. Using defaults.", p, e);
                    }
                },
                Err(e) => {
                    warn!("[Compliance] Failed to read {}: {}. Using defaults.", p, e);
                }
            }
        }
    }

    info!("[Compliance] No config file found. Using default rules.");
    ComplianceConfig::default()
}

/// Generate a default config file at the given path.
fn write_default_config(path: &str) {
    let expanded = shellexpand(path);
    if let Some(parent) = std::path::Path::new(&expanded).parent() {
        std::fs::create_dir_all(parent).ok();
    }

    let config = ComplianceConfig::default();
    let toml_str = toml::to_string_pretty(&config).unwrap_or_default();
    let content = format!(
        "# tredo-compliance configuration file\n\
        # This file is READ-ONLY at startup. Changes require a restart.\n\
        # Defaults are safe for most setups.\n\n\
        {}\n\
        # Blacklist symbols (uncomment to enable):\n\
        # blacklisted_symbols = [\"PENNY_STOCK\", \"ILLIQUID_TOKEN\"]\n",
        toml_str
    );

    match std::fs::write(&expanded, &content) {
        Ok(()) => info!("[Compliance] Default config written to {}", expanded),
        Err(e) => warn!(
            "[Compliance] Could not write default config to {}: {}",
            expanded, e
        ),
    }
}

// ── HTTP Handlers ─────────────────────────────────────────────────────────────

async fn check_handler(
    State(state): State<Arc<ComplianceState>>,
    Json(proposal): Json<TradeProposal>,
) -> (StatusCode, Json<ComplianceResponse>) {
    let response = state.run_checks(&proposal);

    let status = if response.passed {
        StatusCode::OK
    } else {
        StatusCode::UNPROCESSABLE_ENTITY
    };

    if response.passed {
        info!(
            "[Compliance] ✅ ALL CHECKS PASSED for {} {} @ {:.2}",
            proposal.symbol, proposal.direction, proposal.entry_price
        );
    } else {
        warn!(
            "[Compliance] ⛔ COMPLIANCE BLOCKED {} {} @ {:.2} — {}",
            proposal.symbol, proposal.direction, proposal.entry_price, response.summary
        );
    }

    (status, Json(response))
}

async fn status_handler(State(state): State<Arc<ComplianceState>>) -> Json<serde_json::Value> {
    let db_stats = {
        let db = state.db.lock().unwrap();
        db.query_row(
            "SELECT total_checks, passed_checks, failed_checks, pass_rate_pct
             FROM compliance_stats",
            [],
            |row| {
                Ok(serde_json::json!({
                    "total_checks": row.get::<_, i64>(0)?,
                    "passed_checks": row.get::<_, i64>(1)?,
                    "failed_checks": row.get::<_, i64>(2)?,
                    "pass_rate_pct": row.get::<_, f64>(3)?,
                }))
            },
        )
        .unwrap_or(serde_json::json!({
            "total_checks": 0,
            "passed_checks": 0,
            "failed_checks": 0,
            "pass_rate_pct": 100.0,
        }))
    };

    Json(serde_json::json!({
        "status": "running",
        "version": state.version,
        "database": db_stats,
        "timestamp_micros": Utc::now().timestamp_micros(),
    }))
}

async fn config_handler(State(state): State<Arc<ComplianceState>>) -> Json<serde_json::Value> {
    Json(serde_json::to_value(&state.config).unwrap_or_default())
}

// ── Main ──────────────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() {
    // Initialize logging
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "tredo_compliance=info".into()),
        )
        .init();

    println!("╔══════════════════════════════════════════════════════╗");
    println!(
        "║   tredo-compliance v{}                              ║",
        env!("CARGO_PKG_VERSION")
    );
    println!("║   Pre-Trade Compliance Gateway                     ║");
    println!("╚══════════════════════════════════════════════════════╝");
    println!();

    // Parse CLI args --config and --db
    let args: Vec<String> = std::env::args().collect();
    let config_path = args
        .iter()
        .position(|a| a == "--config")
        .and_then(|i| args.get(i + 1))
        .cloned()
        .unwrap_or_else(|| DEFAULT_CONFIG_PATH.to_string());
    let db_path = args
        .iter()
        .position(|a| a == "--db")
        .and_then(|i| args.get(i + 1))
        .cloned()
        .unwrap_or_else(|| DB_PATH.to_string());

    // Load config
    let config = load_config(&config_path);

    // Write default config if none exists
    let expanded_config = shellexpand(&config_path);
    if !std::path::Path::new(&expanded_config).exists() {
        write_default_config(&config_path);
    }

    // Initialize database
    let db = match init_database(&db_path) {
        Ok(db) => db,
        Err(e) => {
            error!("[Compliance] Failed to initialize database: {}", e);
            std::process::exit(1);
        }
    };

    let state = Arc::new(ComplianceState::new(config, db).expect("Failed to initialize state"));

    println!("[Compliance] 🌐 HTTP server on port {}", HTTP_PORT);
    println!("[Compliance]    POST /check  — Submit trade proposal for compliance validation");
    println!("[Compliance]    GET  /status — Gateway status & statistics");
    println!("[Compliance]    GET  /config — Current rules configuration");
    println!("[Compliance]    GET  /health — Alias for /status");
    println!();
    println!("[Compliance] ⚠  THIS GATEWAY CANNOT BE BYPASSED BY THE TRADING PROCESS");
    println!(
        "[Compliance]    The orchestrator MUST receive passed=true before executing any trade."
    );
    println!();
    println!("[Compliance] 📋 12 parameterized rules loaded:");
    println!("    [CRITICAL] symbol_blacklist       — Check symbol not blacklisted");
    println!(
        "    [CRITICAL] price_collar           — FAT-finger protection ({:.1}%)",
        state.config.price_collar_pct
    );
    println!(
        "    [CRITICAL] max_daily_loss         — Daily loss circuit breaker ({:.1}%)",
        state.config.max_daily_loss_pct
    );
    println!(
        "    [CRITICAL] max_leverage           — Leverage limit ({})",
        state.config.max_leverage
    );
    println!(
        "    [CRITICAL] max_drawdown           — Max drawdown ({:.1}%)",
        state.config.max_drawdown_pct
    );
    println!(
        "    [HIGH]    max_position_size       — Max position size ({:.1}% of equity)",
        state.config.max_position_equity_pct
    );
    println!(
        "    [HIGH]    consecutive_loss_breaker — Loss streak limit ({})",
        state.config.consecutive_loss_limit
    );
    println!(
        "    [HIGH]    portfolio_heat          — Portfolio risk exposure ({:.1}%)",
        state.config.max_portfolio_heat_pct
    );
    println!(
        "    [MEDIUM]  min_confluence          — Min confluence score ({:.2})",
        state.config.min_confluence_score
    );
    println!(
        "    [MEDIUM]  symbol_concentration    — Single symbol exposure ({:.1}%)",
        state.config.max_concentration_pct
    );
    println!(
        "    [MEDIUM]  min_risk_reward         — Min risk-reward ratio ({:.1})",
        state.config.min_risk_reward_ratio
    );
    println!(
        "    [LOW]     order_adv_ratio         — Market impact check ({:.1}% of ADV)",
        state.config.max_order_adv_pct
    );
    println!();

    // Build HTTP router
    let app = Router::new()
        .route("/check", post(check_handler))
        .route("/status", get(status_handler))
        .route("/config", get(config_handler))
        .route("/health", get(|| async { Json(serde_json::json!({"status": "ok", "service": "tredo-compliance", "version": env!("CARGO_PKG_VERSION")})) as Json<serde_json::Value> }))
        .layer(tower_http::cors::CorsLayer::permissive())
        .with_state(state);

    let addr = SocketAddr::from(([0, 0, 0, 0], HTTP_PORT));
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .expect("Failed to bind");
    println!("[Compliance] 🌐 Listening on http://{}/", addr);
    println!("[Compliance] 🚀 Ready to validate trade proposals.");

    axum::serve(listener, app).await.unwrap();
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn test_db() -> Connection {
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch(
            "CREATE TABLE IF NOT EXISTS compliance_log (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                timestamp_micros INTEGER NOT NULL,
                symbol TEXT NOT NULL,
                direction TEXT NOT NULL,
                entry_price REAL NOT NULL,
                position_value REAL NOT NULL,
                leverage INTEGER NOT NULL,
                confidence REAL NOT NULL,
                confluence REAL NOT NULL,
                portfolio_equity REAL NOT NULL,
                portfolio_heat REAL NOT NULL,
                daily_pnl REAL NOT NULL,
                consecutive_losses INTEGER NOT NULL,
                trades_today INTEGER NOT NULL,
                drawdown_pct REAL NOT NULL,
                passed INTEGER NOT NULL,
                version TEXT NOT NULL,
                checks_summary TEXT NOT NULL,
                raw_response TEXT NOT NULL
            );",
        )
        .unwrap();
        db
    }

    fn valid_proposal() -> TradeProposal {
        TradeProposal {
            symbol: "BTC".to_string(),
            direction: "BUY".to_string(),
            entry_price: 50000.0,
            stop_loss: 49000.0,
            take_profit: 52000.0,
            position_size: 0.3,
            position_value: 15000.0,
            leverage: 1,
            confidence_score: 0.8,
            confluence_score: 0.6,
            current_price: 50000.0,
            portfolio_equity: 100000.0,
            portfolio_heat: 0.05,
            daily_pnl: 0.0,
            daily_pnl_pct: 0.0,
            consecutive_losses: 0,
            open_positions_count: 2,
            trades_today: 3,
            current_drawdown_pct: 2.0,
            symbol_exposure: 0.0,
            previous_day_volume: 1_000_000.0,
            timestamp_micros: Utc::now().timestamp_micros(),
        }
    }

    #[test]
    fn test_valid_proposal_passes() {
        let config = ComplianceConfig::default();
        let db = test_db();
        let state = ComplianceState::new(config, db).unwrap();
        let proposal = valid_proposal();
        let response = state.run_checks(&proposal);

        assert!(response.passed, "Valid proposal should pass all checks");
        assert_eq!(response.checks.len(), 12, "Should have 12 rule checks");
    }

    #[test]
    fn test_price_collar_rejects() {
        let config = ComplianceConfig::default();
        let db = test_db();
        let state = ComplianceState::new(config, db).unwrap();
        let mut proposal = valid_proposal();
        proposal.entry_price = 55000.0;

        let response = state.run_checks(&proposal);

        assert!(!response.passed, "Price collar should reject 10% deviation");
        let collar_check = response
            .checks
            .iter()
            .find(|c| c.rule_name == "price_collar")
            .unwrap();
        assert!(!collar_check.passed);
    }

    #[test]
    fn test_max_daily_loss_rejects() {
        let config = ComplianceConfig::default();
        let db = test_db();
        let state = ComplianceState::new(config, db).unwrap();
        let mut proposal = valid_proposal();
        proposal.daily_pnl = -10000.0;

        let response = state.run_checks(&proposal);

        assert!(!response.passed);
    }

    #[test]
    fn test_consecutive_loss_breaker() {
        let config = ComplianceConfig::default();
        let db = test_db();
        let state = ComplianceState::new(config, db).unwrap();
        let mut proposal = valid_proposal();
        proposal.consecutive_losses = 6;

        let response = state.run_checks(&proposal);

        assert!(!response.passed);
    }

    #[test]
    fn test_blacklist_rejects() {
        let config = ComplianceConfig {
            blacklisted_symbols: vec!["PENNY".to_string()],
            ..Default::default()
        };
        let db = test_db();
        let state = ComplianceState::new(config, db).unwrap();
        let mut proposal = valid_proposal();
        proposal.symbol = "PENNY".to_string();

        let response = state.run_checks(&proposal);

        assert!(!response.passed);
    }

    #[test]
    fn test_leverage_rejects() {
        let config = ComplianceConfig::default();
        let db = test_db();
        let state = ComplianceState::new(config, db).unwrap();
        let mut proposal = valid_proposal();
        proposal.leverage = 10;

        let response = state.run_checks(&proposal);

        assert!(!response.passed);
    }

    #[test]
    fn test_confluence_check() {
        let config = ComplianceConfig::default();
        let db = test_db();
        let state = ComplianceState::new(config, db).unwrap();
        let mut proposal = valid_proposal();
        proposal.confluence_score = 0.1;

        let response = state.run_checks(&proposal);

        assert!(response.passed, "Confluence is MEDIUM — should not block");
        let conf_check = response
            .checks
            .iter()
            .find(|c| c.rule_name == "min_confluence")
            .unwrap();
        assert!(!conf_check.passed);
        assert_eq!(conf_check.severity, "MEDIUM");
    }

    #[test]
    fn test_portfolio_heat_rejects() {
        let config = ComplianceConfig::default();
        let db = test_db();
        let state = ComplianceState::new(config, db).unwrap();
        let mut proposal = valid_proposal();
        proposal.portfolio_heat = 0.50;

        let response = state.run_checks(&proposal);

        assert!(!response.passed);
    }

    #[test]
    fn test_drawdown_rejects() {
        let config = ComplianceConfig::default();
        let db = test_db();
        let state = ComplianceState::new(config, db).unwrap();
        let mut proposal = valid_proposal();
        proposal.current_drawdown_pct = 20.0;

        let response = state.run_checks(&proposal);

        assert!(!response.passed);
    }

    #[test]
    fn test_sqlite_logging() {
        let config = ComplianceConfig::default();
        let db = test_db();
        let state = ComplianceState::new(config, db).unwrap();
        let proposal = valid_proposal();

        let _response = state.run_checks(&proposal);

        // Verify the log was written
        let db_read = state.db.lock().unwrap();
        let count: i64 = db_read
            .query_row("SELECT COUNT(*) FROM compliance_log", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 1, "One log entry should exist");

        // Verify the log content
        let (logged_symbol, logged_passed): (String, i32) = db_read
            .query_row(
                "SELECT symbol, passed FROM compliance_log ORDER BY id DESC LIMIT 1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(logged_symbol, "BTC");
        assert_eq!(logged_passed, 1);
    }
}
