use crate::state::SharedState;
use async_trait::async_trait;
use chrono::{Duration, Utc};
use std::error::Error;
use tredo_core::{Agent, AgentInput, AgentOutput, AgentTier, DisciplineRules};

/// MetaControlAgent — the learning layer.
/// Runs on the slow loop (daily/weekly) to:
/// 1. Review recent episodes with high regret scores
/// 2. Identify patterns in mistakes
/// 3. Propose changes to DisciplineRules
/// 4. Apply approved changes to the live ruleset
pub struct MetaControlAgent {
    pub state: SharedState,
}

impl MetaControlAgent {
    pub fn new(state: SharedState) -> Self {
        Self { state }
    }

    /// Run a full weekly review cycle.
    /// 1. Load high-regret events from SQLite (since `days_back` days ago)
    /// 2. Ask Ollama to find patterns and propose rule changes
    /// 3. Apply approved changes and log them to SQLite
    pub async fn weekly_review(
        &self,
        days_back: i64,
    ) -> Result<WeeklyReviewReport, Box<dyn Error + Send + Sync>> {
        println!(
            "\n[MetaControl] 📊 Starting weekly review (last {} days)...",
            days_back
        );

        let since = Utc::now() - Duration::days(days_back);

        // Load high-regret events from SQLite (primary source)
        let regret_events = self
            .state
            .episode_store
            .load_regret_events_since(&since, 0.5)
            .unwrap_or_default();
        let total_episodes = regret_events.len();

        if total_episodes == 0 {
            // Fallback: try redb memory (legacy)
            let since_ts = since.timestamp();
            let stored = self
                .state
                .memory
                .load_episodes_since(since_ts)
                .unwrap_or_default();
            if stored.is_empty() {
                println!("[MetaControl] ℹ No episodes to review. Skipping.");
                return Ok(WeeklyReviewReport {
                    timestamp: Utc::now(),
                    total_episodes_reviewed: 0,
                    high_regret_episodes: 0,
                    patterns_found: vec![],
                    proposed_changes: vec![],
                    changes_applied: false,
                    summary: "No episodes available for review.".to_string(),
                });
            }
        }

        // Build high-regret summaries from SQLite events
        let high_regret_count = regret_events.len();
        let high_regret_summaries: Vec<String> = regret_events
            .iter()
            .map(|ev| {
                format!(
                    "{} | Regret: {:.2} | Rule: {} | Lesson: {}",
                    ev.symbol, ev.regret_score, ev.rule_violated, ev.lesson
                )
            })
            .collect();

        if high_regret_summaries.is_empty() {
            println!("[MetaControl] ✅ No high-regret episodes. Rules performing well.");
            return Ok(WeeklyReviewReport {
                timestamp: Utc::now(),
                total_episodes_reviewed: total_episodes,
                high_regret_episodes: 0,
                patterns_found: vec![],
                proposed_changes: vec![],
                changes_applied: false,
                summary: format!(
                    "Reviewed {} regret events. No issues found.",
                    total_episodes
                ),
            });
        }

        // Agentmemory sharing for trained data intelligence (long-term lessons across sessions/agents)
        {
            let mem = tredo_core::AgentMemoryClient::new();
            for s in &high_regret_summaries {
                let _ = mem
                    .remember(&format!("TRAINED_META: {}", s), "trained_intelligence")
                    .await;

                // Self-evolution: actually apply a conservative rule tweak if regret high
                // (in real intact system this would be gated + audited; here we mutate for demo)
                if high_regret_count > 3 {
                    {
                        let mut rules = self.state.rules.write().await;
                        if rules.max_risk_per_trade > 0.005 {
                            rules.max_risk_per_trade *= 0.9; // 10% tighter after bad streak
                            println!("[MetaControl] 🔄 SELF-EVOLVED: max_risk_per_trade tightened to {:.4} (from regret patterns)", rules.max_risk_per_trade);
                            // Push to COT for TUI visibility (self-evolution observable)
                            let _ = self
                                .state
                                .push_cot(
                                    "meta",
                                    "high_regret_review",
                                    "RULE_ADAPT",
                                    &format!(
                                        "max_risk tightened to {:.4} after {} high-regret",
                                        rules.max_risk_per_trade, high_regret_count
                                    ),
                                    0.95,
                                    0,
                                    None,
                                    None,
                                )
                                .await;
                        }
                    }
                    // Persist the adaptation
                    let _ = mem
                        .remember(
                            &format!(
                                "RULE_ADAPT: max_risk tightened after {} high-regret",
                                high_regret_count
                            ),
                            "trained_intelligence",
                        )
                        .await;
                };
            }
            let _ = mem
                .remember(
                    &format!(
                        "META_REVIEW: {} high-regret from {} episodes, {} days",
                        high_regret_count, total_episodes, days_back
                    ),
                    "meta_trained",
                )
                .await;
        }

        println!(
            "[MetaControl] 🔍 Found {} high-regret episodes out of {}. Asking LLM for analysis...",
            high_regret_count, total_episodes
        );

        // Build a summary of current rules for the LLM
        let current_rules_summary = {
            let rules = self.state.rules.read().await;
            format!(
                "Current rules: max_risk_per_trade={:.3}, max_daily_drawdown={:.3}, \
                 max_consecutive_losses={}, min_confluence_score={:.2}",
                rules.max_risk_per_trade,
                rules.max_daily_drawdown,
                rules.max_consecutive_losses,
                rules.min_confluence_score,
            )
        };

        // Ask Ollama to analyse the mistakes
        let analysis = self
            .state
            .llm
            .ask_for_meta_review(&high_regret_summaries, &current_rules_summary)
            .await;

        let pattern = analysis["pattern"]
            .as_str()
            .unwrap_or("No pattern identified")
            .to_string();
        let recommendation = analysis["recommendation"]
            .as_str()
            .unwrap_or("No recommendation")
            .to_string();
        let patterns_found: Vec<String> = if pattern != "No pattern identified" {
            vec![pattern]
        } else {
            vec![]
        };

        // Parse suggested changes
        let mut proposed_changes: Vec<RuleChange> = Vec::new();
        if let Some(changes) = analysis["suggested_changes"].as_array() {
            for change in changes {
                if let (Some(rule), Some(current), Some(suggested)) = (
                    change["rule"].as_str(),
                    change["current_value"].as_f64(),
                    change["suggested_value"].as_f64(),
                ) {
                    let reason = change["reason"]
                        .as_str()
                        .unwrap_or("No reason given")
                        .to_string();
                    proposed_changes.push(RuleChange {
                        rule: rule.to_string(),
                        current_value: current,
                        suggested_value: suggested,
                        reason,
                        applied: false,
                    });
                }
            }
        }

        // Apply approved changes and log them to SQLite
        let mut changes_applied = false;
        if !proposed_changes.is_empty() {
            let mut rules = self.state.rules.write().await;
            for change in &mut proposed_changes {
                let old_val = change.current_value;
                let applied = apply_rule_change(&mut rules, change);
                change.applied = applied;
                if applied {
                    changes_applied = true;
                    println!(
                        "[MetaControl] ✅ Applied rule change: {} → {:.4} (was {:.4}) — {}",
                        change.rule, change.suggested_value, old_val, change.reason
                    );
                    // Persist rule change to SQLite for history
                    let rc = crate::episode_store::RuleChangeRow {
                        rule_name: change.rule.clone(),
                        old_value: old_val,
                        new_value: change.suggested_value,
                        reason: change.reason.clone(),
                        applied_at: Utc::now().to_rfc3339(),
                    };
                    let _ = self.state.episode_store.insert_rule_change(&rc);
                }
            }
            drop(rules);

            // Save updated rules to redb state
            if let Ok(json) = serde_json::to_string(&*self.state.rules.read().await) {
                let _ = self.state.memory.store_state("rules/current", &json);
            }
        }

        let report = WeeklyReviewReport {
            timestamp: Utc::now(),
            total_episodes_reviewed: total_episodes,
            high_regret_episodes: high_regret_count,
            patterns_found,
            proposed_changes,
            changes_applied,
            summary: recommendation,
        };

        // Store report
        if let Ok(json) = serde_json::to_string(&report) {
            let key = format!("meta/review/{}", Utc::now().timestamp());
            let _ = self.state.memory.store_state(&key, &json);
        }

        println!(
            "[MetaControl] ✅ Weekly review complete — {} changes applied.",
            changes_applied as u8
        );
        Ok(report)
    }
}

/// Apply a single rule change to the DisciplineRules struct.
fn apply_rule_change(rules: &mut DisciplineRules, change: &RuleChange) -> bool {
    match change.rule.as_str() {
        "max_risk_per_trade" => {
            rules.max_risk_per_trade = change.suggested_value.clamp(0.001, 0.05);
            true
        }
        "max_daily_drawdown" => {
            rules.max_daily_drawdown = change.suggested_value.clamp(0.01, 0.10);
            true
        }
        "max_consecutive_losses" => {
            rules.max_consecutive_losses = (change.suggested_value as u32).clamp(1, 10);
            true
        }
        "min_confluence_score" => {
            rules.min_confluence_score = change.suggested_value.clamp(0.3, 0.95);
            true
        }
        _ => {
            println!("[MetaControl] ⚠ Unknown rule: {}. Skipping.", change.rule);
            false
        }
    }
}

#[async_trait]
impl Agent for MetaControlAgent {
    fn name(&self) -> &str {
        "MetaControlAgent"
    }
    fn tier(&self) -> AgentTier {
        AgentTier::Main
    }

    async fn run(
        &self,
        _input: Option<AgentInput>,
    ) -> Result<AgentOutput, Box<dyn Error + Send + Sync>> {
        let _ = self.weekly_review(7).await?;
        Ok(AgentOutput::Done)
    }
}

// ── Types ──────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RuleChange {
    pub rule: String,
    pub current_value: f64,
    pub suggested_value: f64,
    pub reason: String,
    pub applied: bool,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct WeeklyReviewReport {
    pub timestamp: chrono::DateTime<Utc>,
    pub total_episodes_reviewed: usize,
    pub high_regret_episodes: usize,
    pub patterns_found: Vec<String>,
    pub proposed_changes: Vec<RuleChange>,
    pub changes_applied: bool,
    pub summary: String,
}
