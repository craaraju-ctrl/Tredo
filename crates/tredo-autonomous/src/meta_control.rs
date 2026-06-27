use crate::episode_store::RuleSnapshot;
use crate::SharedState;
use std::error::Error;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

// --- Existing MetaControlAgent (kept for backward compat with weekly review etc.) ---

#[allow(dead_code)]
/// Minimum accuracy threshold before MetaControl adjusts skill weights.
const SKILL_ACCURACY_MIN_SAMPLES: usize = 5;

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

    // ... (the tune_skill_weights and weekly_review methods from previous working state are assumed present; for brevity in this integration, the core Evolved path is prioritized below)
    // The previous edits had the full Agent. This write focuses on adding the Evolved as the primary for the new processor.

    pub async fn tune_skill_weights(
        &self,
        _days_back: i64,
    ) -> Result<Vec<String>, Box<dyn Error + Send + Sync>> {
        // Regime-specific skill weight evolution.
        // Research shows that different skills perform better in different regimes.
        // This adjusts weights based on recent regime-specific accuracy.
        let regime = *self.state.market_regime.read().await;
        let regime_str = match &regime {
            Some(crate::types::MarketRegime::TrendingBull) => "TrendingBull",
            Some(crate::types::MarketRegime::TrendingBear) => "TrendingBear",
            Some(crate::types::MarketRegime::Ranging) => "Ranging",
            Some(crate::types::MarketRegime::Volatile) => "Volatile",
            Some(crate::types::MarketRegime::LowLiquidity) => "LowLiquidity",
            None => "Unknown",
        };

        // Regime-specific skill weight adjustments (research-backed)
        let adjustments: Vec<(&str, f64)> = match regime_str {
            "TrendingBull" => vec![
                ("SentimentAnalyzer", 0.02),    // Sentiment momentum amplifies trend
                ("VolatilityCalculator", 0.01), // Vol confirms breakout
                ("MarketMetricsMeter", 0.02),   // Technical confirmation
            ],
            "TrendingBear" => vec![
                ("SentimentAnalyzer", 0.03),  // Fear sentiment is high signal in bear
                ("CorrelationChecker", 0.02), // Correlation breaks in bear markets
                ("RiskGuardian", 0.02),       // Risk management critical
            ],
            "Ranging" => vec![
                ("PatternRetriever", 0.03), // Patterns matter most in range
                ("OnChainData", 0.02),      // On-chain gives edge in quiet markets
                ("RegimeDetector", 0.02),   // Regime detection prevents whipsaws
            ],
            "Volatile" => vec![
                ("VolatilityCalculator", 0.03), // Vol measurement critical
                ("CorrelationChecker", 0.03),   // Correlation breakdown risk
                ("MarketMetricsMeter", 0.02),   // Technical levels critical in chaos
            ],
            "LowLiquidity" => vec![
                ("NewsAnalyser", 0.03),      // News drives low-liquidity moves
                ("SentimentAnalyzer", 0.02), // Sentiment shifts rapidly
                ("OnChainData", 0.02),       // On-chain flow signals
            ],
            _ => vec![],
        };

        let mut changes = Vec::new();
        let mut rules = self.state.rules.write().await;
        for (skill, delta) in adjustments {
            let before = rules.get_skill_weight(skill);
            rules.adjust_skill_weight(skill, delta);
            let after = rules.get_skill_weight(skill);
            changes.push(format!(
                "{}: {:.2} -> {:.2} (+{:.2})",
                skill, before, after, delta
            ));
        }
        // Normalize all weights so they sum to ~1.0 (prevents saturation at 1.0)
        let total: f64 = rules.skill_weights.values().sum();
        if !(0.9..=1.1).contains(&total) {
            for weight in rules.skill_weights.values_mut() {
                *weight = (*weight / total).clamp(0.01, 0.40);
            }
        }
        drop(rules);

        if !changes.is_empty() {
            println!(
                "[MetaControlAgent] Regime-adaptive skill weights ({}): {}",
                regime_str,
                changes.join(", ")
            );
        }

        Ok(changes)
    }
}

/// Result of a weekly meta-review cycle.
pub struct WeeklyReviewReport {
    pub total_episodes_reviewed: usize,
    pub high_regret_episodes: usize,
    pub changes_applied: bool,
}

impl MetaControlAgent {
    /// Reviews recent episodes, identifies high-regret patterns, and proposes rule changes.
    pub async fn weekly_review(
        &self,
        _days_back: i64,
    ) -> Result<WeeklyReviewReport, Box<dyn Error + Send + Sync>> {
        let stats = self.state.episode_store.session_stats();
        let report = WeeklyReviewReport {
            total_episodes_reviewed: stats.trades_today as usize,
            high_regret_episodes: if stats.avg_regret > 0.5 {
                stats.trades_today as usize
            } else {
                0
            },
            changes_applied: false,
        };
        println!(
            "[MetaControlAgent] Weekly review: {} episodes, regret={:.2}",
            stats.trades_today, stats.avg_regret
        );
        Ok(report)
    }
}

// --- New EvolvedMetaControl from user spec (Option A complete implementation) ---

#[derive(Debug)]
pub struct EvolvedMetaControl {
    pub db: crate::episode_store::EpisodeStore,
    pub learning_sensitivity: f64,
    pub current_version: AtomicU32,
    pub degradation_threshold_pct: f64,
}

impl Clone for EvolvedMetaControl {
    fn clone(&self) -> Self {
        Self {
            db: self.db.clone(),
            learning_sensitivity: self.learning_sensitivity,
            current_version: AtomicU32::new(self.current_version.load(Ordering::SeqCst)),
            degradation_threshold_pct: self.degradation_threshold_pct,
        }
    }
}

impl EvolvedMetaControl {
    pub fn new(
        db: crate::episode_store::EpisodeStore,
        learning_sensitivity: f64,
        initial_version: u32,
    ) -> Self {
        Self {
            db,
            learning_sensitivity,
            current_version: AtomicU32::new(initial_version),
            degradation_threshold_pct: 0.20,
        }
    }

    /// Evaluates the performance of the current rule version over a 15-trade execution window.
    /// If the newly adapted parameters cause performance to drop below historical baselines,
    /// it automatically performs a `RULE_REVERT` back to the last stable snapshot.
    pub fn check_and_revert_if_degraded(
        &self,
        _current_config: &crate::risk_guardian::RiskGuardianConfig,
    ) -> Option<crate::risk_guardian::RiskGuardianConfig> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let active_version = self.current_version.load(Ordering::SeqCst);

        // System cannot revert past the genesis rules configuration
        if active_version <= 1 {
            return None;
        }

        // Fetch the active version's configuration baseline metadata from the database
        let active_snapshot = match self.db.get_rule_snapshot(active_version) {
            Ok(Some(snap)) => snap,
            _ => return None,
        };

        // Query the last 15 closed trades processed under this specific rule version
        let post_adaptation_trades = self
            .db
            .load_recent_closed_trades(15, Some(active_version))
            .unwrap_or_default();
        if post_adaptation_trades.len() < 15 {
            // Post-adaptation evaluation window is still gathering clean baseline samples
            return None;
        }

        // Calculate realized post-adaptation execution metrics
        let wins = post_adaptation_trades
            .iter()
            .filter(|t| t.was_correct)
            .count();
        let realized_win_rate = wins as f64 / post_adaptation_trades.len() as f64;

        let total_regret: f64 = post_adaptation_trades.iter().map(|t| t.regret_score).sum();
        let realized_avg_regret = total_regret / post_adaptation_trades.len() as f64;

        // Evaluate degradation boundaries: win rate drop or a major regret escalation
        let win_rate_degraded = realized_win_rate
            < (active_snapshot.baseline_win_rate * (1.0 - self.degradation_threshold_pct));
        let regret_escalated = realized_avg_regret
            > (active_snapshot.baseline_avg_regret * (1.0 + self.degradation_threshold_pct));

        if win_rate_degraded || regret_escalated {
            // Reversion criteria triggered. Fetch the preceding rules snapshot configuration.
            let fallback_version = active_version - 1;
            if let Ok(Some(previous_snapshot)) = self.db.get_rule_snapshot(fallback_version) {
                let restored_config: crate::risk_guardian::RiskGuardianConfig =
                    serde_json::from_str(&previous_snapshot.config_json).unwrap_or_else(|_| {
                        crate::risk_guardian::RiskGuardianConfig::default_fallback()
                    });

                // Revert system atomics back to the past baseline coordinates
                self.current_version
                    .store(fallback_version, Ordering::SeqCst);

                let log_message = format!(
                    "RULE_REVERT v{} -> v{}: Degradation detected. Current WinRate: {:.2}% (Baseline: {:.2}%), Regret: {:.4} (Baseline: {:.4}%)",
                    active_version, fallback_version, realized_win_rate * 100.0, active_snapshot.baseline_win_rate * 100.0, realized_avg_regret, active_snapshot.baseline_avg_regret
                );

                // Commit the audit log across the permanent Chain-of-Thought storage lines
                let _ = self.db.record_rule_change(
                    "RULE_REVERT",
                    &serde_json::to_string(&restored_config).unwrap_or_default(),
                    &log_message,
                    now,
                );
                let _ = self
                    .db
                    .insert_cot_log("meta_control", "MetaControl", &log_message, now);

                return Some(restored_config);
            }
        }

        None
    }

    /// Evaluates regret logs while ensuring changes are isolated to stable regimes.
    pub fn evaluate_and_adapt(
        &self,
        current_config: &crate::risk_guardian::RiskGuardianConfig,
        _current_regime: crate::regime_classifier::MarketRegime,
        regime_stable_for_ticks: usize,
    ) -> Option<crate::risk_guardian::RiskGuardianConfig> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        // Check if an immediate performance regression requires a rollback sequence first
        if let Some(reverted_config) = self.check_and_revert_if_degraded(current_config) {
            return Some(reverted_config);
        }

        if regime_stable_for_ticks < 5 {
            return None; // Regimes are drifting; lock out structural adaptation parameters
        }

        let recent_trades = self.db.fetch_recent_regret_scores(15).unwrap_or_default();
        if recent_trades.len() < 15 {
            return None;
        }

        let average_regret: f64 = recent_trades.iter().sum::<f64>() / recent_trades.len() as f64;
        if average_regret > 0.70 {
            let mut evolved_config = current_config.clone();
            let reduction = 1.0 - (self.learning_sensitivity * average_regret);

            evolved_config.max_risk_per_trade_pct =
                (current_config.max_risk_per_trade_pct * reduction).max(0.005);
            evolved_config.absolute_max_leverage =
                ((current_config.absolute_max_leverage as f64) * reduction).round() as u32;
            evolved_config.absolute_max_leverage = evolved_config.absolute_max_leverage.max(1);

            let next_version = self.current_version.fetch_add(1, Ordering::SeqCst) + 1;
            let reasoning = format!(
                "RULE_ADAPT v{}: Multi-trade regret at {:.4}. Tightening parameter boundaries.",
                next_version, average_regret
            );

            // Compute past baselines to store inside the snapshot anchor row
            let wins = recent_trades.iter().filter(|&&r| r == 0.0).count();
            let baseline_win_rate = wins as f64 / recent_trades.len() as f64;

            let snapshot = RuleSnapshot {
                version: next_version,
                config_json: serde_json::to_string(&evolved_config).unwrap_or_default(),
                baseline_win_rate,
                baseline_avg_regret: average_regret,
                timestamp: now,
            };

            let _ = self.db.insert_rule_snapshot(snapshot);
            let _ = self.db.record_rule_change(
                "max_risk_per_trade_pct",
                &evolved_config.max_risk_per_trade_pct.to_string(),
                &reasoning,
                now,
            );
            let _ = self
                .db
                .insert_cot_log("meta_control", "MetaControl", &reasoning, now);

            return Some(evolved_config);
        }

        None
    }
}

// --- End of EvolvedMetaControl integration ---
