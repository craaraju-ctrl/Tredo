// tredo.rs
// TREDO Agent Hierarchy — logical grouping of sub-agents under four managers.
//
//  Tredo (main orchestrator)
//  ├── Identifier   — scans & reads the market
//  │   ├── WatchlistScannerAgent
//  │   ├── MarketIntelligenceAgent
//  │   ├── PivotCalculatorAgent
//  │   ├── ConfluenceScorerAgent
//  │   ├── PatternRetrieverAgent
//  │   ├── SessionTimerAgent
//  │   └── RedFolderCheckerAgent
//  ├── Verifier     — validates risk & psychology before any trade
//  │   ├── RiskPsychologyAgent
//  │   ├── RiskCalculatorAgent
//  │   └── ReflectorAgent
//  ├── Executer     — decides & executes validated signals
//  │   ├── StrategyDecisionAgent
//  │   ├── PortfolioManagerAgent
//  │   └── ExecutionCoordinatorAgent
//  └── Guardian     — monitors drawdown, prevents overtrading, logs outcomes
//      ├── DrawdownMonitorAgent
//      ├── OvertradingPreventerAgent
//      └── OutcomeLoggerAgent
//
// IMPORTANT: this file is PURELY organisational. It holds Arc references to the
// same agent instances that AutonomousOrchestrator already owns — so there is
// zero duplication of state or logic.

use crate::types::{RiskAnalysis, TradeSignal};
use std::error::Error;
use std::sync::Arc;
use tredo_core::Agent;

// ─────────────────────────────────────────────────────────────────────────────
// IDENTIFIER — scans the market and identifies potential opportunities
// ─────────────────────────────────────────────────────────────────────────────

/// Scans and reads the market to surface actionable intelligence.
///
/// Sub-agents:
///  - `scanner`           → WatchlistScannerAgent
///  - `market_intel`      → MarketIntelligenceAgent
///  - `pivot_calc`        → PivotCalculatorAgent
///  - `confluence`        → ConfluenceScorerAgent
///  - `pattern_retriever` → PatternRetrieverAgent
///  - `session_timer`     → SessionTimerAgent
///  - `red_folder`        → RedFolderCheckerAgent
#[derive(Clone)]
pub struct Identifier {
    pub scanner: Arc<crate::scanner::WatchlistScannerAgent>,
    pub market_intel: Arc<crate::market_intelligence::MarketIntelligenceAgent>,
    pub pivot_calc: Arc<crate::pivot_calculator::PivotCalculatorAgent>,
    pub confluence: Arc<crate::confluence_scorer::ConfluenceScorerAgent>,
    pub pattern_retriever: Arc<crate::pattern_retriever::PatternRetrieverAgent>,
    pub session_timer: Arc<crate::session_timer::SessionTimerAgent>,
    pub red_folder: Arc<crate::red_folder_checker::RedFolderCheckerAgent>,
}

impl Identifier {
    /// Returns a human-readable tree of this manager's sub-agents.
    pub fn describe() -> &'static str {
        "Identifier\n\
         ├── WatchlistScannerAgent\n\
         ├── MarketIntelligenceAgent\n\
         ├── PivotCalculatorAgent\n\
         ├── ConfluenceScorerAgent\n\
         ├── PatternRetrieverAgent\n\
         ├── SessionTimerAgent\n\
         └── RedFolderCheckerAgent"
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// VERIFIER — validates risk & psychology before any trade
// ─────────────────────────────────────────────────────────────────────────────

/// Validates that every risk & psychology rule is satisfied.
///
/// Sub-agents:
///  - `risk_psych`   → RiskPsychologyAgent
///  - `risk_calc`    → RiskCalculatorAgent
///  - `reflector`    → ReflectorAgent
#[derive(Clone)]
pub struct Verifier {
    pub risk_psych: Arc<crate::risk_psychology::RiskPsychologyAgent>,
    pub risk_calc: Arc<crate::risk_calculator::RiskCalculatorAgent>,
    pub reflector: Arc<crate::reflector::ReflectorAgent>,
}

impl Verifier {
    pub fn describe() -> &'static str {
        "Verifier\n\
         ├── RiskPsychologyAgent\n\
         ├── RiskCalculatorAgent\n\
         └── ReflectorAgent"
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// EXECUTER — places trades
// ─────────────────────────────────────────────────────────────────────────────

/// Turns a verified signal into a real (paper) trade.
///
/// Sub-agents:
///  - `strategy`       → StrategyDecisionAgent
///  - `portfolio`      → PortfolioManagerAgent
///  - `execution`      → ExecutionCoordinatorAgent
#[derive(Clone)]
pub struct Executer {
    pub strategy: Arc<crate::strategy_decision::StrategyDecisionAgent>,
    pub portfolio: Arc<crate::portfolio_manager::PortfolioManagerAgent>,
    pub execution: Arc<crate::execution_coordinator::ExecutionCoordinatorAgent>,
}

impl Executer {
    pub fn describe() -> &'static str {
        "Executer\n\
         ├── StrategyDecisionAgent\n\
         ├── PortfolioManagerAgent\n\
         └── ExecutionCoordinatorAgent"
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// GUARDIAN — monitors discipline limits and logs outcomes
// ─────────────────────────────────────────────────────────────────────────────

/// Monitors account drawdown, limits overtrading, and logs outcomes.
///
/// Sub-agents:
///  - `drawdown`       → DrawdownMonitorAgent
///  - `overtrading`    → OvertradingPreventerAgent
///  - `outcome_logger` → OutcomeLoggerAgent
#[derive(Clone)]
pub struct Guardian {
    pub drawdown: Arc<crate::drawdown_monitor::DrawdownMonitorAgent>,
    pub overtrading: Arc<crate::overtrading_preventer::OvertradingPreventerAgent>,
    pub outcome_logger: Arc<crate::outcome_logger::OutcomeLoggerAgent>,
}

impl Guardian {
    pub fn describe() -> &'static str {
        "Guardian\n\
         ├── DrawdownMonitorAgent\n\
         ├── OvertradingPreventerAgent\n\
         └── OutcomeLoggerAgent"
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// TREDO — the top-level orchestration view
// ─────────────────────────────────────────────────────────────────────────────

/// Tredo is the top-level agent hierarchy wrapper.
///
/// It holds the four manager groups and provides a single `describe()` that
/// prints the complete agent tree:
///
/// ```text
/// Tredo
/// ├── Identifier
/// │   ├── WatchlistScannerAgent
/// │   ├── MarketIntelligenceAgent
/// │   ├── PivotCalculatorAgent
/// │   ├── ConfluenceScorerAgent
/// │   ├── PatternRetrieverAgent
/// │   ├── SessionTimerAgent
/// │   └── RedFolderCheckerAgent
/// ├── Verifier
/// │   ├── RiskPsychologyAgent
/// │   ├── RiskCalculatorAgent
/// │   └── ReflectorAgent
/// ├── Executer
/// │   ├── StrategyDecisionAgent
/// │   ├── PortfolioManagerAgent
/// │   └── ExecutionCoordinatorAgent
/// └── Guardian
///     ├── DrawdownMonitorAgent
///     ├── OvertradingPreventerAgent
///     └── OutcomeLoggerAgent
/// ```
#[derive(Clone)]
pub struct Tredo {
    pub identifier: Identifier,
    pub verifier: Verifier,
    pub executer: Executer,
    pub guardian: Guardian,
}

impl Tredo {
    /// Build a `Tredo` view from an existing `AutonomousOrchestrator`.
    ///
    /// All agent instances are shared via `Arc` — **no cloning of state occurs**.
    pub fn from_orchestrator(o: &crate::orchestrator_struct::AutonomousOrchestrator) -> Self {
        Self {
            identifier: Identifier {
                scanner: Arc::clone(&o.scanner),
                market_intel: Arc::clone(&o.market_intel),
                pivot_calc: Arc::clone(&o.pivot_calc),
                confluence: Arc::clone(&o.confluence),
                pattern_retriever: Arc::clone(&o.pattern_retriever),
                session_timer: Arc::clone(&o.session_timer),
                red_folder: Arc::clone(&o.red_folder),
            },
            verifier: Verifier {
                risk_psych: Arc::clone(&o.risk_psych),
                risk_calc: Arc::clone(&o.risk_calc),
                reflector: Arc::clone(&o.reflector),
            },
            executer: Executer {
                strategy: Arc::clone(&o.strategy),
                portfolio: Arc::clone(&o.portfolio),
                execution: Arc::clone(&o.execution),
            },
            guardian: Guardian {
                drawdown: Arc::clone(&o.drawdown),
                overtrading: Arc::clone(&o.overtrading),
                outcome_logger: Arc::clone(&o.outcome_logger),
            },
        }
    }

    /// Print the full Tredo agent tree to stdout.
    pub fn print_tree() {
        println!(
            "\nTredo\n\
             ├── {}\n\
             ├── {}\n\
             ├── {}\n\
             └── {}",
            Identifier::describe(),
            Verifier::describe(),
            Executer::describe(),
            Guardian::describe(),
        );
    }

    // ── Identifier dispatch ────────────────────────────────────────────────
    /// Run the Identifier group: scans the market and identifies opportunities.
    /// Returns (discipline_ok, confluence, pivots) where discipline_ok indicates
    /// whether session timer and red folder checks passed.
    pub async fn run_identifier(
        &self,
        symbol: &str,
        price: f64,
    ) -> Result<(bool, f64, tredo_core::PivotLevels), Box<dyn Error + Send + Sync>> {
        println!(
            "[Tredo::Identifier] Scanning market for {} @ {:.2}",
            symbol, price
        );

        // Run scanner
        let _ = self.identifier.scanner.scan_watchlist().await?;

        // Run market intelligence
        let (confluence, pivots) = self
            .identifier
            .market_intel
            .analyze_market(symbol, price)
            .await?;

        // Run pivot calculator
        let _ = self
            .identifier
            .pivot_calc
            .run(Some(tredo_core::AgentInput::PivotRequest {
                high: price * 1.01,
                low: price * 0.99,
                close: price,
            }))
            .await;

        // Run confluence scorer
        let _ = self.identifier.confluence.run(None).await;

        // Run pattern retriever
        let _ = self.identifier.pattern_retriever.run(None).await?;

        // Run session timer (discipline: session check)
        let session_ok = self.identifier.session_timer.run(None).await.is_ok();

        // Run red folder checker (discipline: news events check)
        let red_ok = self.identifier.red_folder.run(None).await.is_ok();

        let discipline_ok = session_ok && red_ok;
        if !discipline_ok {
            println!(
                "[Tredo::Identifier] ⚠ Discipline checks failed (session: {}, red_folder: {})",
                session_ok, red_ok
            );
        }

        println!(
            "[Tredo::Identifier] ✅ Analysis complete — Confluence: {:.1}%, Discipline: {}",
            confluence * 100.0,
            if discipline_ok { "OK" } else { "FAIL" }
        );
        Ok((discipline_ok, confluence, pivots))
    }

    // ── Verifier dispatch ──────────────────────────────────────────────────
    /// Run the Verifier group: validates risk & psychology.
    /// Delegates to sub-agents: risk_calc -> risk_psych -> reflector.
    /// Note: drawdown and overtrading checks are now performed by the Guardian group.
    pub async fn run_verifier(
        &self,
        symbol: &str,
        price: f64,
        equity: f64,
    ) -> Result<RiskAnalysis, Box<dyn Error + Send + Sync>> {
        println!(
            "[Tredo::Verifier] Validating risk for {} @ {:.2} (equity: {:.2})",
            symbol, price, equity
        );

        // Run guardian checks (drawdown + overtrading) to see if we are allowed to trade
        let (drawdown_res, overtrading_res) = tokio::join!(
            self.guardian.drawdown.run(None),
            self.guardian.overtrading.run(None),
        );
        let discipline_ok = drawdown_res.is_ok() && overtrading_res.is_ok();

        if !discipline_ok {
            println!("[Tredo::Verifier] ⚠ Guardian discipline checks failed");
            return Ok(RiskAnalysis {
                max_position_size: 0.0,
                risk_per_trade_pct: 0.0,
                risk_reward_ratio: 0.0,
                portfolio_heat: 1.0,
                daily_drawdown_pct: 0.0,
                var_95: 0.0,
                recommendation: crate::types::RiskRecommendation::Halt,
                psychology_warnings: vec!["Discipline checks failed".to_string()],
            });
        }

        // Run risk psychology with real equity from state
        let analysis = self
            .verifier
            .risk_psych
            .analyze_risk(&tredo_core::MarketContext {
                symbol: symbol.to_string(),
                current_price: price,
                high: price * 1.01,
                low: price * 0.99,
                previous_close: price * 0.998,
                timestamp: chrono::Utc::now(),
                daily_pnl: 0.0,
                equity,
                consecutive_losses: 0,
                is_red_folder_day: false,
                trend_direction: None,
            })
            .await?;

        // Run risk calculator
        let _ = self.verifier.risk_calc.run(None).await;

        // Run reflector
        let _ = self.verifier.reflector.reflect(symbol).await;

        println!(
            "[Tredo::Verifier] ✅ Risk check — Recommendation: {:?}, Heat: {:.1}%",
            analysis.recommendation,
            analysis.portfolio_heat * 100.0
        );
        Ok(analysis)
    }

    // ── Executer dispatch ─────────────────────────────────────────────────
    /// Run the Executer group: makes a trade decision and executes it.
    /// Delegates to sub-agents: strategy_decision -> portfolio_manager -> execution_coordinator.
    /// Run the Executer group autonomously.
    /// The agent (StrategyDecision) identifies direction + entry/SL/TP itself from indicators (RSI, MACD, patterns, volume, pivots, regime, etc.).
    /// No external price points or direction are provided — this is what makes it agentic AI, not a bot.
    /// Pure agentic entry point (no aggregated signal provided from caller).
    pub async fn run_executer(
        &self,
        symbol: &str,
        current_price: f64,
    ) -> Result<Option<TradeSignal>, Box<dyn Error + Send + Sync>> {
        self.run_executer_with_aggregation(symbol, current_price, None)
            .await
    }

    /// Agentic entry point that accepts the AggregatedSignal computed by MarketIntelligence.
    /// This is the correct pattern: skills are aggregated first, then the consensus is
    /// handed to the decision layer so the agent actually listens to its own thoughts.
    pub async fn run_executer_with_aggregation(
        &self,
        symbol: &str,
        current_price: f64,
        aggregated_signal: Option<&tredo_core::AggregatedSignal>,
    ) -> Result<Option<TradeSignal>, Box<dyn Error + Send + Sync>> {
        println!(
            "[Tredo::Executer] Agent making fully autonomous decision for {} @ current {:.2} (using aggregated skills consensus)",
            symbol, current_price
        );

        // The agent decides direction + its own entry/SL/TP. AggregatedSignal is now a first-class input.
        let signal_opt = self
            .executer
            .strategy
            .generate_signal_with_aggregation(symbol, current_price, aggregated_signal)
            .await?;

        match &signal_opt {
            Some(sig) => {
                println!(
                    "[Tredo::Executer] ✅ AGENT decided {:?} {} @ entry={:.2} SL={:.2} TP={:.2} (conf: {:.1}%)",
                    sig.direction,
                    symbol,
                    sig.entry_price,
                    sig.stop_loss,
                    sig.take_profit,
                    sig.confidence_score * 100.0
                );

                // Execute the trade (execute_paper_trade internally handles position management)
                let exec_result = self.executer.execution.execute_paper_trade(sig).await?;
                println!("[Tredo::Executer] ✅ {}", exec_result);

                // Log outcome via Guardian
                let _ = self.guardian.outcome_logger.run(None).await;

                Ok(Some(sig.clone()))
            }
            None => {
                println!("[Tredo::Executer] Agent decided HOLD for {}", symbol);
                Ok(None)
            }
        }
    }

    /// Return the full agent tree as a JSON-friendly string (for the web API).
    pub fn tree_json() -> serde_json::Value {
        serde_json::json!({
            "name": "Tredo",
            "role": "Main Orchestrator",
            "children": [
                {
                    "name": "Identifier",
                    "role": "Scans & reads the market",
                    "children": [
                        { "name": "WatchlistScannerAgent",  "role": "Scans watchlist for setups" },
                        { "name": "MarketIntelligenceAgent","role": "Fetches price & Kronos forecast" },
                        { "name": "PivotCalculatorAgent",   "role": "Calculates pivot levels" },
                        { "name": "ConfluenceScorerAgent",  "role": "Scores signal confluence" },
                        { "name": "PatternRetrieverAgent",  "role": "Retrieves historical patterns" },
                        { "name": "SessionTimerAgent",      "role": "Guards trading session hours" },
                        { "name": "RedFolderCheckerAgent",  "role": "Blocks trades on news events" }
                    ]
                },
                {
                    "name": "Verifier",
                    "role": "Validates risk & psychology",
                    "children": [
                        { "name": "RiskPsychologyAgent",      "role": "Assesses trader psychology & heat" },
                        { "name": "RiskCalculatorAgent",      "role": "Calculates position size & R:R" },
                        { "name": "ReflectorAgent",           "role": "Runs LLM self-reflection check" }
                    ]
                },
                {
                    "name": "Executer",
                    "role": "Places trades",
                    "children": [
                        { "name": "StrategyDecisionAgent",      "role": "LLM final BUY/SELL/HOLD decision" },
                        { "name": "PortfolioManagerAgent",      "role": "Manages portfolio & positions" },
                        { "name": "ExecutionCoordinatorAgent",  "role": "Places paper/live orders" }
                    ]
                },
                {
                    "name": "Guardian",
                    "role": "Monitors discipline & logs outcomes",
                    "children": [
                        { "name": "DrawdownMonitorAgent",     "role": "Monitors daily drawdown limits" },
                        { "name": "OvertradingPreventerAgent","role": "Prevents overtrading" },
                        { "name": "OutcomeLoggerAgent",         "role": "Logs trade outcomes & PnL" }
                    ]
                }
            ]
        })
    }
}
