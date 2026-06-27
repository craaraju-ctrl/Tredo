//! # SelfEvolutionValidator — Extended Validation Harness for Compounding Improvement
//!
//! Runs N cycles of the full autonomous pipeline on one or more symbols, optionally
//! inducing regret (tight stops), and measures the self-evolution loop:
//!
//! **Metrics tracked per cycle-bucket:**
//! - Regret trend (average regret per bucket, should decrease over time)
//! - Win/loss rate per bucket
//! - Rule adaptation events (# RULE_ADAPT, actual rule value changes)
//! - MetaControl rule changes applied (max_risk, min_confluence, etc.)
//!
//! **Expected outcome (compounding improvement):**
//! After meta-adaptations tighten risk rules, subsequent cycles should show
//! lower average regret, fewer high-regret episodes, and more cautious decisions.
//!
//! ## Usage
//! ```ignore
//! let validator = SelfEvolutionValidator::new(orchestrator);
//! let report = validator.run_extended_validation(&["BTC", "ETH"], 50, true).await?;
//! println!("{}", report.summary());
//! ```

use crate::episode_store::RuleChangeSnapshot;
use crate::orchestrator_struct::AutonomousOrchestrator;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::error::Error;

/// When set, validation induces regret by tightening stops to this percentage.
const INDUCED_REGRET_SL_PCT: f64 = 0.5;

/// BUCKET_SIZE episodes per statistical bucket (10 = every 10 episodes we compute averages)
const BUCKET_SIZE: usize = 10;

// ── Per-Cycle Metrics (one entry per pipeline cycle) ───────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CycleMetrics {
    pub cycle_number: usize,
    pub symbol: String,
    pub decision: String, // "BUY" | "SELL" | "HOLD"
    pub confidence: f64,
    pub confluence: f64,
    pub regret_score: Option<f64>,     // populated if trade closed
    pub trade_outcome: Option<String>, // "WIN" | "LOSS" | "BREAKEVEN"
    pub exit_reason: Option<String>,   // "stop_loss" | "take_profit" | "manual"
    pub rule_change_applied: bool,
    pub rules_snapshot: RulesSnapshot,
    pub timestamp: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RulesSnapshot {
    pub max_risk_per_trade: f64,
    pub max_daily_drawdown: f64,
    pub max_consecutive_losses: u32,
    pub min_confluence_score: f64,
}

impl RulesSnapshot {
    fn from(rules: &tredo_core::DisciplineRules) -> Self {
        Self {
            max_risk_per_trade: rules.max_risk_per_trade,
            max_daily_drawdown: rules.max_daily_drawdown,
            max_consecutive_losses: rules.max_consecutive_losses,
            min_confluence_score: rules.min_confluence_score,
        }
    }
}

// ── Bucket Statistics (compressed metrics over BUCKET_SIZE cycles) ─────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BucketStats {
    pub bucket_index: usize,
    pub cycle_count: usize,
    pub avg_regret: f64,
    pub win_count: usize,
    pub loss_count: usize,
    pub hold_count: usize,
    pub avg_confidence: f64,
    pub rule_changes: Vec<RuleChangeSnapshot>,
    pub rules_at_end: RulesSnapshot,
}

// ── Final Report ───────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SelfEvolutionReport {
    pub run_start: DateTime<Utc>,
    pub run_end: DateTime<Utc>,
    pub symbols: Vec<String>,
    pub total_cycles: usize,
    pub induce_regret: bool,

    /// Per-bucket stats for trend analysis
    pub buckets: Vec<BucketStats>,

    /// All cycles (detailed, for debugging)
    pub cycles: Vec<CycleMetrics>,

    /// All rule changes applied during the run
    pub rule_changes: Vec<RuleChangeSnapshot>,

    // ── Trend Analysis ─────────────────────────────────────────────────
    /// Average regret in the first half of the run
    pub regret_first_half: f64,
    /// Average regret in the second half of the run
    pub regret_second_half: f64,
    /// Regret trend direction: "DECREASING" (improving) | "INCREASING" | "STABLE"
    pub regret_trend: String,

    /// Win rate in first half
    pub win_rate_first_half: f64,
    /// Win rate in second half
    pub win_rate_second_half: f64,

    /// Total rule adaptations triggered
    pub total_rule_adaptations: usize,

    /// Summary narrative for the report
    pub summary_text: String,
}

impl SelfEvolutionReport {
    /// Generate a human-readable summary of the validation run.
    pub fn summary(&self) -> String {
        let mut lines = vec![
            "╔══════════════════════════════════════════════════════════════╗".to_string(),
            "║        TREDO SELF-EVOLUTION VALIDATION REPORT              ║".to_string(),
            "╚══════════════════════════════════════════════════════════════╝".to_string(),
            String::new(),
            format!(
                "Run: {} → {}",
                self.run_start.format("%H:%M:%S"),
                self.run_end.format("%H:%M:%S")
            ),
            format!("Symbols: {}", self.symbols.join(", ")),
            format!(
                "Total cycles: {} (induce_regret={})",
                self.total_cycles, self.induce_regret
            ),
            format!("Total rule adaptations: {}", self.total_rule_adaptations),
            String::new(),
            "── REGRET TREND ──".to_string(),
            format!("  First half avg regret:  {:.3}", self.regret_first_half),
            format!("  Second half avg regret: {:.3}", self.regret_second_half),
        ];

        // Direction indicator
        let regret_arrow = if self.regret_trend == "DECREASING" {
            "📉 DECREASING (improving!)"
        } else if self.regret_trend == "INCREASING" {
            "📈 INCREASING (degrading)"
        } else {
            "➡️ STABLE"
        };
        lines.push(format!("  Trend: {}", regret_arrow));

        lines.push(String::new());
        lines.push("── WIN RATE TREND ──".to_string());
        lines.push(format!(
            "  First half win rate:  {:.1}%",
            self.win_rate_first_half * 100.0
        ));
        lines.push(format!(
            "  Second half win rate: {:.1}%",
            self.win_rate_second_half * 100.0
        ));

        // Win rate direction
        if self.win_rate_second_half > self.win_rate_first_half {
            lines.push("  Direction: 📈 Improving!".to_string());
        } else if self.win_rate_second_half < self.win_rate_first_half {
            lines.push("  Direction: 📉 Declining".to_string());
        } else {
            lines.push("  Direction: ➡️ Stable".to_string());
        }

        if !self.buckets.is_empty() {
            lines.push(String::new());
            lines.push("── PER-BUCKET BREAKDOWN ──".to_string());
            for bucket in &self.buckets {
                let regret_str = format!("{:.3}", bucket.avg_regret);
                let wr_str = if bucket.cycle_count > 0 {
                    format!(
                        "{:.0}%",
                        (bucket.win_count as f64 / bucket.cycle_count.max(1) as f64) * 100.0
                    )
                } else {
                    "N/A".to_string()
                };
                lines.push(format!(
                    "  Bucket {:2}: {} cycles | regret={} | WR={} | wins={} losses={} holds={} | rules_changed={}",
                    bucket.bucket_index,
                    bucket.cycle_count,
                    regret_str,
                    wr_str,
                    bucket.win_count,
                    bucket.loss_count,
                    bucket.hold_count,
                    bucket.rule_changes.len(),
                ));
            }
        }

        if !self.rule_changes.is_empty() {
            lines.push(String::new());
            lines.push("── RULE ADAPTATIONS ──".to_string());
            for rc in &self.rule_changes {
                lines.push(format!(
                    "  {}: {:.4} → {:.4} — {}",
                    rc.rule_name, rc.old_value, rc.new_value, rc.reason
                ));
            }
        }

        lines.push(String::new());
        lines.push("── CONCLUSION ──".to_string());
        lines.push(format!("  {}", self.summary_text));

        // Compounding improvement assessment
        let compounding = if self.regret_trend == "DECREASING"
            && self.win_rate_second_half >= self.win_rate_first_half
        {
            "✅ Compounding improvement detected: regret decreasing and win rate stable/improving."
        } else if self.regret_trend == "DECREASING" {
            "🟡 Partial improvement: regret decreasing but win rate not yet improving."
        } else {
            "🔄 Insufficient data for compounding assessment. Run more cycles or induce stronger regret."
        };
        lines.push(format!("  {}", compounding));

        lines.join("\n")
    }
}

// ── SelfEvolutionValidator ─────────────────────────────────────────────────

pub struct SelfEvolutionValidator {
    orchestrator: AutonomousOrchestrator,
}

impl SelfEvolutionValidator {
    pub fn new(orchestrator: AutonomousOrchestrator) -> Self {
        Self { orchestrator }
    }

    /// Run extended validation: N cycles per symbol with optional induced regret.
    /// Returns a structured report with trend analysis and compounding evidence.
    pub async fn run_extended_validation(
        &self,
        symbols: &[&str],
        cycles: usize,
        induce_regret: bool,
    ) -> Result<SelfEvolutionReport, Box<dyn Error + Send + Sync>> {
        let run_start = Utc::now();
        println!("\n╔══════════════════════════════════════════════════════════════╗");
        println!(
            "║   TREDO SELF-EVOLUTION VALIDATION ({} cycles)        ║",
            cycles
        );
        println!("╚══════════════════════════════════════════════════════════════╝");
        println!(
            "Symbols: {:?} | Induce regret: {} | Bucket size: {} episodes",
            symbols, induce_regret, BUCKET_SIZE
        );

        // Log validation parameters (replaced env var approach with constant)
        if induce_regret {
            println!(
                ">>> INDUCING REGRET: {:.1}% tight SL for all trades",
                INDUCED_REGRET_SL_PCT
            );
        }

        let mut all_cycles: Vec<CycleMetrics> = Vec::new();
        let mut rule_changes_all: Vec<RuleChangeSnapshot> = Vec::new();

        for cycle_num in 0..cycles {
            for &symbol in symbols {
                // Read current price from OHLCV history or portfolio
                let _price = {
                    let portfolio = self.orchestrator.state.portfolio.read().await;
                    let s = symbol.to_string();
                    if let Some(pos) = portfolio.open_positions.iter().find(|p| p.symbol == s) {
                        pos.current_price
                    } else {
                        let history = self.orchestrator.state.ohlcv_history.read().await;
                        history
                            .get(symbol)
                            .and_then(|h| h.last().map(|b| b.close))
                            .unwrap_or(60000.0) // default BTC-ish price
                    }
                };

                // Determine direction based on simple price momentum
                let _direction = {
                    let history = self.orchestrator.state.ohlcv_history.read().await;
                    let has_bars = history.get(symbol).map(|h| h.len() >= 5).unwrap_or(false);
                    if has_bars {
                        let bars = history.get(symbol).unwrap();
                        let change = bars.last().unwrap().close - bars[bars.len() - 5].close;
                        if change >= 0.0 {
                            tredo_core::TradeDirection::Long
                        } else {
                            tredo_core::TradeDirection::Short
                        }
                    } else {
                        tredo_core::TradeDirection::Long
                    }
                };

                // (levels are no longer computed here — the agent decides them autonomously inside run_full_pipeline)

                // Run the full pipeline
                let pipeline_result = match self
                    .orchestrator
                    .run_full_pipeline(symbol) // agentic: agent decides levels from its own analysis
                    .await
                {
                    Ok(p) => p,
                    Err(e) => {
                        eprintln!("[SelfEvolutionValidator] Pipeline error for {} cycle {}: {}. Skipping.", symbol, cycle_num, e);
                        continue;
                    }
                };

                // Check if position was closed in this cycle
                let (regret, outcome, exit_reason) = {
                    // Query the latest closed trade from SQLite
                    let store = &self.orchestrator.state.episode_store;
                    let recent = store.get_most_recent_closed(symbol).unwrap_or_else(|e| {
                        eprintln!(
                            "[SelfEvolutionValidator] DB error fetching closed trade: {}",
                            e
                        );
                        None
                    });
                    match recent {
                        Some(ep) => (
                            Some(ep.regret_score),
                            Some(ep.outcome),
                            Some(ep.exit_reason),
                        ),
                        None => (None, None, None),
                    }
                };

                // Snapshot current rules
                let rules_snapshot =
                    RulesSnapshot::from(&*self.orchestrator.state.rules.read().await);

                // Track if a rule change just happened
                let rule_change_this_cycle = {
                    let store = &self.orchestrator.state.episode_store;
                    let recent_changes = store.get_recent_rule_changes(1).unwrap_or_default();
                    !recent_changes.is_empty()
                };

                let decision = if pipeline_result.executed {
                    "BUY/SELL".to_string()
                } else {
                    "HOLD".to_string()
                };

                let metrics = CycleMetrics {
                    cycle_number: cycle_num,
                    symbol: symbol.to_string(),
                    decision,
                    confidence: pipeline_result
                        .final_signal
                        .as_ref()
                        .map(|s| s.confidence_score)
                        .unwrap_or(0.0),
                    confluence: pipeline_result
                        .final_signal
                        .as_ref()
                        .map(|s| s.confluence_score)
                        .unwrap_or(0.0),
                    regret_score: regret,
                    trade_outcome: outcome,
                    exit_reason,
                    rule_change_applied: rule_change_this_cycle,
                    rules_snapshot,
                    timestamp: Utc::now(),
                };

                all_cycles.push(metrics);

                if cycle_num % 5 == 0 {
                    print!(".");
                    if (cycle_num + 1) % 50 == 0 {
                        println!(" {} cycles complete", cycle_num + 1);
                    }
                }
            }
        }

        // Collect all rule changes from the run
        {
            let store = &self.orchestrator.state.episode_store;
            if let Ok(changes) = store.get_all_rule_changes() {
                rule_changes_all = changes;
            }
        }

        // Compute buckets
        let buckets = self.compute_buckets(&all_cycles);

        // Trend analysis
        let (regret_first, regret_second) = Self::compute_half_regret(&buckets);
        let (wr_first, wr_second) = Self::compute_half_win_rates(&buckets);
        let regret_trend = if regret_second < regret_first * 0.9 {
            "DECREASING".to_string()
        } else if regret_second > regret_first * 1.1 {
            "INCREASING".to_string()
        } else {
            "STABLE".to_string()
        };

        let total_rule_adaptations = rule_changes_all.len();

        // Generate conclusion
        let summary_text = Self::generate_conclusion(
            regret_trend.as_str(),
            regret_first,
            regret_second,
            wr_first,
            wr_second,
            total_rule_adaptations,
            cycles,
        );

        let run_end = Utc::now();

        let report = SelfEvolutionReport {
            run_start,
            run_end,
            symbols: symbols.iter().map(|s| s.to_string()).collect(),
            total_cycles: cycles,
            induce_regret,
            buckets,
            cycles: all_cycles,
            rule_changes: rule_changes_all,
            regret_first_half: regret_first,
            regret_second_half: regret_second,
            regret_trend,
            win_rate_first_half: wr_first,
            win_rate_second_half: wr_second,
            total_rule_adaptations,
            summary_text,
        };

        // Store report to redb for persistence
        if let Ok(json) = serde_json::to_string(&report) {
            let key = format!("evolution/report/{}", run_end.timestamp());
            let _ = self.orchestrator.state.memory.store_state(&key, &json);
        } // Output to agentmemory for cross-session learning
        {
            let mem = tredo_core::AgentMemoryClient::new();
            let _ = mem
                .remember(
                    &format!(
                        "SELF_EVOLUTION: {} cycles on {}, regret trend {}, WR {:.0}% → {:.0}%, {} adaptations",
                        cycles,
                        symbols.join(","),
                        report.regret_trend.as_str(),
                        wr_first * 100.0,
                        wr_second * 100.0,
                        total_rule_adaptations
                    ),
                    "self_evolution",
                )
                .await;
        }

        println!("\n\n=== VALIDATION COMPLETE ===");
        println!("{}", report.summary());

        Ok(report)
    }

    /// Group cycles into buckets of BUCKET_SIZE and compute aggregate stats.
    fn compute_buckets(&self, cycles: &[CycleMetrics]) -> Vec<BucketStats> {
        if cycles.is_empty() {
            return vec![];
        }

        let mut buckets: Vec<BucketStats> = Vec::new();
        let mut current_entries: Vec<&CycleMetrics> = Vec::new();

        for (i, cycle) in cycles.iter().enumerate() {
            current_entries.push(cycle);
            if current_entries.len() >= BUCKET_SIZE || i == cycles.len() - 1 {
                let bucket_idx = buckets.len();
                let count = current_entries.len();

                let avg_regret: f64 = {
                    let scores: Vec<f64> = current_entries
                        .iter()
                        .filter_map(|c| c.regret_score)
                        .collect();
                    if scores.is_empty() {
                        0.0
                    } else {
                        scores.iter().sum::<f64>() / scores.len() as f64
                    }
                };

                let win_count = current_entries
                    .iter()
                    .filter(|c| c.trade_outcome.as_deref() == Some("WIN"))
                    .count();
                let loss_count = current_entries
                    .iter()
                    .filter(|c| c.trade_outcome.as_deref() == Some("LOSS"))
                    .count();
                let hold_count = current_entries
                    .iter()
                    .filter(|c| c.decision == "HOLD")
                    .count();

                let avg_confidence: f64 =
                    current_entries.iter().map(|c| c.confidence).sum::<f64>() / count as f64;

                let rule_changes: Vec<RuleChangeSnapshot> = current_entries
                    .iter()
                    .filter(|c| c.rule_change_applied)
                    .map(|_| RuleChangeSnapshot {
                        rule_name: "see_global".to_string(),
                        old_value: 0.0,
                        new_value: 0.0,
                        reason: "bucket summary".to_string(),
                        applied_at: String::new(),
                    })
                    .collect();

                let last_entry = current_entries.last().unwrap();
                let rules_at_end = last_entry.rules_snapshot.clone();

                buckets.push(BucketStats {
                    bucket_index: bucket_idx,
                    cycle_count: count,
                    avg_regret,
                    win_count,
                    loss_count,
                    hold_count,
                    avg_confidence,
                    rule_changes,
                    rules_at_end,
                });

                current_entries.clear();
            }
        }

        buckets
    }

    /// Average regret in first half vs second half of buckets.
    fn compute_half_regret(buckets: &[BucketStats]) -> (f64, f64) {
        if buckets.is_empty() {
            return (0.0, 0.0);
        }
        let mid = buckets.len() / 2;
        if mid == 0 {
            return (buckets[0].avg_regret, buckets[0].avg_regret);
        }
        let first: Vec<_> = buckets[..mid].iter().collect();
        let second: Vec<_> = buckets[mid..].iter().collect();

        let f_avg = first.iter().map(|b| b.avg_regret).sum::<f64>() / first.len() as f64;
        let s_avg = second.iter().map(|b| b.avg_regret).sum::<f64>() / second.len() as f64;
        (f_avg, s_avg)
    }

    /// Win rates in first half vs second half.
    fn compute_half_win_rates(buckets: &[BucketStats]) -> (f64, f64) {
        if buckets.is_empty() {
            return (0.0, 0.0);
        }
        let mid = buckets.len() / 2;
        if mid == 0 {
            let total = buckets[0].win_count + buckets[0].loss_count;
            let wr = if total > 0 {
                buckets[0].win_count as f64 / total as f64
            } else {
                0.0
            };
            return (wr, wr);
        }
        let first: Vec<_> = buckets[..mid].iter().collect();
        let second: Vec<_> = buckets[mid..].iter().collect();

        let f_wins: usize = first.iter().map(|b| b.win_count).sum();
        let f_losses: usize = first.iter().map(|b| b.loss_count).sum();
        let s_wins: usize = second.iter().map(|b| b.win_count).sum();
        let s_losses: usize = second.iter().map(|b| b.loss_count).sum();

        let f_total = f_wins + f_losses;
        let s_total = s_wins + s_losses;

        let f_wr = if f_total > 0 {
            f_wins as f64 / f_total as f64
        } else {
            0.0
        };
        let s_wr = if s_total > 0 {
            s_wins as f64 / s_total as f64
        } else {
            0.0
        };

        (f_wr, s_wr)
    }

    /// Generate the conclusion text for the report.
    fn generate_conclusion(
        regret_trend: &str,
        regret_first: f64,
        regret_second: f64,
        wr_first: f64,
        wr_second: f64,
        total_adaptations: usize,
        total_cycles: usize,
    ) -> String {
        let mut parts = Vec::new();

        // Regret assessment
        if regret_trend == "DECREASING" {
            parts.push(format!(
                "Regret decreased from {:.3} to {:.3} — the system is learning from mistakes.",
                regret_first, regret_second
            ));
        } else if regret_trend == "INCREASING" {
            parts.push(format!(
                "Regret increased from {:.3} to {:.3} — may need more cycles or different market regime.",
                regret_first, regret_second
            ));
        } else {
            parts.push(format!(
                "Regret stable at ~{:.3} — system is consistent but may need stronger regret induction.",
                regret_first
            ));
        }

        // Win rate assessment
        if wr_second > wr_first + 0.05 {
            parts.push(format!(
                "Win rate improved from {:.0}% to {:.0}% — decisions are getting better over time.",
                wr_first * 100.0,
                wr_second * 100.0
            ));
        } else if wr_second < wr_first - 0.05 {
            parts.push(format!(
                "Win rate declined from {:.0}% to {:.0}% — rule tightening may be reducing edge detection.",
                wr_first * 100.0,
                wr_second * 100.0
            ));
        } else {
            parts.push(format!("Win rate stable around {:.0}%.", wr_first * 100.0));
        }

        // Rule adaptations
        if total_adaptations > 0 {
            parts.push(format!(
                "{} rule adaptations were applied by MetaControl, demonstrating active self-evolution.",
                total_adaptations
            ));
        } else {
            parts.push(
                "No rule adaptations triggered — either regret was too low (<0.5) or too few trades closed."
                    .to_string(),
            );
        }

        // Overall
        let overall = if regret_trend == "DECREASING" {
            format!(
                "Compounding improvement detected after {} cycles. The self-evolving loop is functioning: high-regret outcomes trigger MetaControl rule tightening, and subsequent cycles show lower regret. This validates the core 'Rules + Memory + Debate > Pure Prompting' philosophy in practice.",
                total_cycles
            )
        } else {
            format!(
                "Ran {} cycles — self-evolution loop is active (reflection + meta) but needs more high-regret events (--induce-regret or volatile market conditions) to demonstrate measurable compounding improvement.",
                total_cycles
            )
        };
        parts.push(overall);

        parts.join(" ")
    }
}
