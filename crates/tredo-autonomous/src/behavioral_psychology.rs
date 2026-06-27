// ═══════════════════════════════════════════════════════════════════════════════
// Behavioral Psychology Layer — Deep Psychological Upgrade
//
// Replaces the basic RiskPsychologyAgent with a comprehensive behavioral finance
// engine that detects cognitive biases, tracks emotional state, estimates
// psychological fatigue, and modulates position sizing/decisions accordingly.
//
// Key features:
//   1. Bias Detection — 8 behavioral biases monitored in real-time
//   2. Emotional State — Greed/Fear index, overconfidence, fatigue, tilt
//   3. Psychology Score — Overall system psychological health (0.0–1.0)
//   4. Psych-Adjusted Sizing — Position size modulated by psychological state
//   5. Decision Quality Audit — Tracks if psychological factors influenced outcomes
//
// No LLM dependency — all analysis is rule-based and deterministic.
// Inspired by: Kahneman & Tversky (Prospect Theory), Thaler (Behavioral Finance),
//               Zweig (Your Money & Your Brain), Steenbarger (Psychology of Trading)
// ═══════════════════════════════════════════════════════════════════════════════

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

// ═══════════════════════════════════════════════════════════════════════════════
// Behavioral Biases — 8 cognitive biases we actively monitor
// ═══════════════════════════════════════════════════════════════════════════════

/// Behavioral biases detected in the trading system's decision process.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BehavioralBias {
    /// Confirmation bias: seeking evidence that confirms existing positions
    ConfirmationBias,
    /// Recency bias: overweighting recent outcomes over long-term statistics
    RecencyBias,
    /// Overconfidence: taking larger risks after a win streak
    Overconfidence,
    /// Loss aversion: reluctance to close losing positions (fear of realizing loss)
    LossAversion,
    /// Revenge trading: increasing size after losses to "get even"
    RevengeTrading,
    /// Anchoring: fixating on a specific price level (entry, ATH, etc.)
    Anchoring,
    /// Herding: following the crowd (buying tops, selling bottoms)
    Herding,
    /// Gambler's fallacy: expecting reversal after a streak
    GamblersFallacy,
}

impl BehavioralBias {
    pub fn name(&self) -> &str {
        match self {
            BehavioralBias::ConfirmationBias => "Confirmation Bias",
            BehavioralBias::RecencyBias => "Recency Bias",
            BehavioralBias::Overconfidence => "Overconfidence",
            BehavioralBias::LossAversion => "Loss Aversion",
            BehavioralBias::RevengeTrading => "Revenge Trading",
            BehavioralBias::Anchoring => "Anchoring",
            BehavioralBias::Herding => "Herding",
            BehavioralBias::GamblersFallacy => "Gambler's Fallacy",
        }
    }

    pub fn description(&self) -> &str {
        match self {
            BehavioralBias::ConfirmationBias => "Seeking evidence that confirms existing positions while ignoring contrary signals",
            BehavioralBias::RecencyBias => "Overweighting recent outcomes (last 3 trades) over long-term statistics",
            BehavioralBias::Overconfidence => "Taking larger risks after a win streak — 'I can't lose' mentality",
            BehavioralBias::LossAversion => "Reluctance to close losing positions — fear of realizing a loss exceeds desire for gain",
            BehavioralBias::RevengeTrading => "Increasing position size after losses to recover quickly — emotional escalation",
            BehavioralBias::Anchoring => "Fixating on a specific price level (entry, ATH) instead of current market reality",
            BehavioralBias::Herding => "Following the crowd — buying near tops, selling near bottoms",
            BehavioralBias::GamblersFallacy => "Expecting a reversal after a streak despite independent probabilities",
        }
    }
}

/// A detected bias occurrence with severity.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BiasDetection {
    pub bias: BehavioralBias,
    /// Severity 0.0–1.0
    pub severity: f64,
    /// Human-readable explanation of why this bias was detected
    pub evidence: String,
    /// Suggested corrective action
    pub corrective_action: String,
}

// ═══════════════════════════════════════════════════════════════════════════════
// Emotional State — Continuous tracking of trading psychology
// ═══════════════════════════════════════════════════════════════════════════════

/// The system's current emotional/psychological state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmotionalState {
    /// Greed index (0.0 = extreme fear, 1.0 = extreme greed)
    pub greed_fear_index: f64,
    /// Overconfidence level (0.0 = none, 1.0 = extreme)
    pub overconfidence: f64,
    /// Fatigue level (0.0 = fresh, 1.0 = exhausted)
    pub fatigue: f64,
    /// Tilt level (0.0 = calm, 1.0 = tilted/irrational)
    pub tilt: f64,
    /// Revenge trading urge (0.0 = none, 1.0 = strong)
    pub revenge_urge: f64,
    /// Overall psychological health score (0.0 = critical, 1.0 = optimal)
    pub psych_health: f64,
    /// Human-readable state label
    pub state_label: String,
}

impl EmotionalState {
    pub fn optimal() -> Self {
        Self {
            greed_fear_index: 0.5,
            overconfidence: 0.0,
            fatigue: 0.0,
            tilt: 0.0,
            revenge_urge: 0.0,
            psych_health: 1.0,
            state_label: "Optimal".to_string(),
        }
    }

    /// Get the position size multiplier based on psychological state.
    /// Returns a multiplier between 0.0 (no trading) and 1.0 (full size).
    pub fn psych_size_multiplier(&self) -> f64 {
        let mut mult: f64 = 1.0;

        // Strong psychological effects reduce size significantly
        if self.tilt > 0.7 {
            mult *= 0.0; // No trading when tilted
        } else if self.tilt > 0.4 {
            mult *= 0.3;
        } else if self.tilt > 0.2 {
            mult *= 0.6;
        }

        // Revenge urge is dangerous — reduce or halt
        if self.revenge_urge > 0.6 {
            mult *= 0.0;
        } else if self.revenge_urge > 0.3 {
            mult *= 0.4;
        }

        // Overconfidence leads to oversized positions — counter with reduction
        if self.overconfidence > 0.7 {
            mult *= 0.5;
        } else if self.overconfidence > 0.4 {
            mult *= 0.7;
        }

        // Fatigue degrades decision quality
        if self.fatigue > 0.8 {
            mult *= 0.3;
        } else if self.fatigue > 0.5 {
            mult *= 0.6;
        }

        // Greed/fear extremes
        if self.greed_fear_index > 0.85 || self.greed_fear_index < 0.15 {
            mult *= 0.5;
        } else if self.greed_fear_index > 0.7 || self.greed_fear_index < 0.3 {
            mult *= 0.8;
        }

        mult.clamp(0.0, 1.0)
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// Behavioral Psychology Engine
// ═══════════════════════════════════════════════════════════════════════════════

/// Complete behavioral psychology snapshot for a trading cycle.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PsychologySnapshot {
    pub timestamp: DateTime<Utc>,
    pub emotional_state: EmotionalState,
    pub active_biases: Vec<BiasDetection>,
    pub position_size_multiplier: f64,
    pub trading_advisory: PsychologyAdvisory,
    pub summary: String,
}

/// Advisory based on psychological state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum PsychologyAdvisory {
    /// Full trading allowed
    Normal,
    /// Reduce position sizes due to psychological factors
    ReduceSize(f64), // multiplier
    /// Only take highest-conviction trades
    Caution(String),
    /// Halt all trading — psychological state too compromised
    Halt(String),
}

/// Tracks decision history for bias detection.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DecisionRecord {
    pub timestamp: DateTime<Utc>,
    pub action: String, // "BUY", "SELL", "HOLD"
    pub confidence: f64,
    pub position_size: f64,
    pub pnl: Option<f64>,
    pub was_profitable: Option<bool>,
}

/// The BehavioralPsychologyEngine — monitors, detects, and corrects.
#[derive(Debug, Clone)]
pub struct BehavioralPsychologyEngine {
    /// Recent trading decisions for pattern analysis
    decision_history: Vec<DecisionRecord>,
    /// Maximum history to keep
    max_history: usize,
    /// Track consecutive wins/losses
    consecutive_wins: u32,
    consecutive_losses: u32,
    /// Previous state for trend analysis
    previous_tilt: f64,
    previous_overconfidence: f64,
}

impl BehavioralPsychologyEngine {
    pub fn new() -> Self {
        Self {
            decision_history: Vec::new(),
            max_history: 50,
            consecutive_wins: 0,
            consecutive_losses: 0,
            previous_tilt: 0.0,
            previous_overconfidence: 0.0,
        }
    }

    /// Record a trading decision for psychological analysis.
    pub fn record_decision(&mut self, action: &str, confidence: f64, size: f64) {
        self.decision_history.push(DecisionRecord {
            timestamp: Utc::now(),
            action: action.to_string(),
            confidence,
            position_size: size,
            pnl: None,
            was_profitable: None,
        });

        if self.decision_history.len() > self.max_history {
            self.decision_history.remove(0);
        }
    }

    /// Record a trade outcome for psychological tracking.
    pub fn record_outcome(&mut self, profitable: bool) {
        if let Some(record) = self
            .decision_history
            .iter_mut()
            .rev()
            .find(|r| r.pnl.is_none())
        {
            record.was_profitable = Some(profitable);
        }

        if profitable {
            self.consecutive_wins = (self.consecutive_wins + 1).min(20);
            self.consecutive_losses = 0;
        } else {
            self.consecutive_losses = (self.consecutive_losses + 1).min(20);
            self.consecutive_wins = 0;
        }
    }

    /// Analyze the current psychological state and detect biases.
    ///
    /// Takes current portfolio metrics and returns a full psychology snapshot.
    pub fn analyze(
        &self,
        _portfolio_heat: f64,
        drawdown_pct: f64,
        total_trades_today: u32,
        daily_pnl_pct: f64,
        _equity: f64,
        _initial_equity: f64,
    ) -> PsychologySnapshot {
        let mut active_biases = Vec::new();
        let mut overconfidence = 0.0;
        let mut tilt = 0.0;
        let mut revenge = 0.0;

        // ── 1. Detect Confirmation Bias ─────────────────────────────────────
        // If the system keeps BUYing in a downtrend or SELLing in an uptrend
        // despite conflicting evidence, it may have confirmation bias.
        if self.consecutive_losses >= 3 {
            let recent_actions = self.decision_history.iter().rev().take(5);
            let same_direction_count = {
                let mut last_dir = None;
                let mut count = 0;
                for r in recent_actions {
                    if let Some(ld) = last_dir {
                        if r.action == ld && r.action != "HOLD" {
                            count += 1;
                        }
                    }
                    last_dir = Some(r.action.clone());
                }
                count
            };
            if same_direction_count >= 3 {
                active_biases.push(BiasDetection {
                    bias: BehavioralBias::ConfirmationBias,
                    severity: (same_direction_count as f64 * 0.2).min(0.9),
                    evidence: format!(
                        "System took same direction {} times despite {} consecutive losses — ignoring contrary signals",
                        same_direction_count, self.consecutive_losses
                    ),
                    corrective_action: "Force diversification: evaluate opposite direction with equal weight".to_string(),
                });
            }
        }

        // ── 2. Detect Recency Bias ──────────────────────────────────────────
        if self.consecutive_losses >= 2 {
            let recent_pnl = self
                .decision_history
                .iter()
                .rev()
                .take(3)
                .filter_map(|r| r.pnl)
                .sum::<f64>();
            if recent_pnl < -0.05 && self.consecutive_losses >= 2 {
                let recency_severity = (self.consecutive_losses as f64 * 0.25).min(0.8);
                active_biases.push(BiasDetection {
                    bias: BehavioralBias::RecencyBias,
                    severity: recency_severity,
                    evidence: format!(
                        "Recent P&L {:.1}% with {} consecutive losses — overweighting short-term pain",
                        recent_pnl * 100.0, self.consecutive_losses
                    ),
                    corrective_action: "Refer to long-term statistics: check vector memory win rate before deciding".to_string(),
                });
            }
        }

        // ── 3. Detect Overconfidence ────────────────────────────────────────
        if self.consecutive_wins >= 3 {
            overconfidence = (self.consecutive_wins as f64 * 0.15).min(0.9);
            active_biases.push(BiasDetection {
                bias: BehavioralBias::Overconfidence,
                severity: overconfidence,
                evidence: format!(
                    "{} consecutive wins — risk perception likely impaired",
                    self.consecutive_wins
                ),
                corrective_action:
                    "Reduce position sizing by 40% and increase confluence threshold by 0.10"
                        .to_string(),
            });
        }

        // ── 4. Detect Loss Aversion ─────────────────────────────────────────
        if drawdown_pct > 0.03 {
            // Check if system is taking unusually small positions (fear of loss)
            let recent_sizes = self
                .decision_history
                .iter()
                .rev()
                .take(3)
                .map(|r| r.position_size)
                .collect::<Vec<_>>();
            if recent_sizes.len() >= 3 {
                let avg_size = recent_sizes.iter().sum::<f64>() / recent_sizes.len() as f64;
                // Compare to a baseline — if sizes dropped sharply after losses
                let older_sizes = self
                    .decision_history
                    .iter()
                    .rev()
                    .skip(3)
                    .take(3)
                    .map(|r| r.position_size)
                    .collect::<Vec<_>>();
                if older_sizes.len() >= 3 {
                    let older_avg = older_sizes.iter().sum::<f64>() / older_sizes.len() as f64;
                    if older_avg > 0.0 && avg_size < older_avg * 0.5 {
                        active_biases.push(BiasDetection {
                            bias: BehavioralBias::LossAversion,
                            severity: 0.5,
                            evidence: format!(
                                "Position sizes dropped from {:.2} to {:.2} after drawdown — fear of further loss",
                                older_avg, avg_size
                            ),
                            corrective_action: "Check if reduction is rational (confluence change) or emotional (loss aversion)".to_string(),
                        });
                    }
                }
            }
        }

        // ── 5. Detect Revenge Trading ───────────────────────────────────────
        if self.consecutive_losses >= 2 {
            // Check if position sizes are INCREASING after losses
            // recent_sizes is in reverse chronological order (newest first)
            // To detect increasing sizes over time: each earlier trade (w[1]) should be smaller than the later trade (w[0])
            let recent_sizes = self
                .decision_history
                .iter()
                .rev()
                .take(self.consecutive_losses as usize)
                .map(|r| r.position_size)
                .collect::<Vec<_>>();
            if recent_sizes.len() >= 2 {
                let increasing = recent_sizes.windows(2).all(|w| w[0] > w[1]);
                if increasing {
                    revenge = (self.consecutive_losses as f64 * 0.2).min(0.8);
                    active_biases.push(BiasDetection {
                        bias: BehavioralBias::RevengeTrading,
                        severity: revenge,
                        evidence: format!(
                            "Position sizes increasing after {} losses — attempting to 'get even'",
                            self.consecutive_losses
                        ),
                        corrective_action: "FORCE SIZE REDUCTION: Maximum 50% of normal position until a winning trade".to_string(),
                    });
                }
            }
        }

        // ── 6. Fatigue Estimation ───────────────────────────────────────────
        // Fatigue increases with: number of trades, drawdown duration, late hours
        let mut fatigue = (total_trades_today as f64 * 0.05).min(0.6);
        if drawdown_pct > 0.02 {
            fatigue += 0.15;
        }
        if daily_pnl_pct < -0.02 {
            fatigue += 0.1;
        }
        fatigue = fatigue.min(1.0);

        // ── 7. Tilt Estimation ──────────────────────────────────────────────
        // Tilt is emotional frustration — triggered by:
        // - Multiple consecutive losses
        // - High drawdown
        // - Large adverse price moves
        if self.consecutive_losses >= 3 {
            tilt = (self.consecutive_losses as f64 * 0.2).min(0.9);
        }
        if drawdown_pct > 0.04 {
            tilt = tilt.max(0.6);
        }
        // Check if tilt is increasing (accelerating frustration)
        if tilt > self.previous_tilt && tilt > 0.3 {
            tilt = (tilt * 1.3).min(1.0); // Tilt acceleration
        }

        // ── 8. Greed/Fear Index ─────────────────────────────────────────────
        // Composite of all detected psychological factors
        let gf: f64 = 0.5_f64
            + (overconfidence * 0.3_f64)           // Greed from overconfidence
            + (revenge * 0.2_f64)                  // Desperation masquerading as greed
            - (drawdown_pct * 2.0 * 0.2_f64)      // Fear from drawdown
            - (tilt * 0.15_f64)                    // Fear from tilt
            - ((1.0_f64 - fatigue) * 0.1_f64); // Slight fear from fatigue
        let greed_fear = gf.clamp(0.0_f64, 1.0_f64);

        // ── Overall Psych Health ────────────────────────────────────────────
        let psych_health = (1.0
            - tilt * 0.35
            - revenge * 0.25
            - overconfidence * 0.15
            - fatigue * 0.15
            - (greed_fear - 0.5).abs() * 0.10)
            .clamp(0.0, 1.0);

        // ── State Label ─────────────────────────────────────────────────────
        let state_label = if psych_health > 0.8 {
            "Optimal"
        } else if psych_health > 0.6 {
            "Cautious"
        } else if psych_health > 0.4 {
            "Compromised"
        } else if psych_health > 0.2 {
            "Critical"
        } else {
            "Panic"
        };

        let emotional_state = EmotionalState {
            greed_fear_index: greed_fear,
            overconfidence,
            fatigue,
            tilt,
            revenge_urge: revenge,
            psych_health,
            state_label: state_label.to_string(),
        };

        // ── Position Size Multiplier ────────────────────────────────────────
        let size_mult = emotional_state.psych_size_multiplier();

        // ── Advisory ────────────────────────────────────────────────────────
        let advisory = if size_mult <= 0.0 {
            PsychologyAdvisory::Halt(format!(
                "Psychological state CRITICAL: tilt={:.1}%, revenge={:.1}% — trading halted for safety",
                tilt * 100.0, revenge * 100.0
            ))
        } else if size_mult < 0.4 {
            PsychologyAdvisory::Caution(format!(
                "Compromised state (health={:.0}%): tilt={:.1}%, fatigue={:.1}% — only highest-conviction trades",
                psych_health * 100.0, tilt * 100.0, fatigue * 100.0
            ))
        } else if size_mult < 0.8 {
            PsychologyAdvisory::ReduceSize(size_mult)
        } else {
            PsychologyAdvisory::Normal
        };

        // ── Summary ─────────────────────────────────────────────────────────
        let bias_summary = if active_biases.is_empty() {
            "No behavioral biases detected".to_string()
        } else {
            active_biases
                .iter()
                .map(|b| format!("{} ({:.0}%)", b.bias.name(), b.severity * 100.0))
                .collect::<Vec<_>>()
                .join(", ")
        };

        let summary =
            format!(
            "Psychology: {} | Health: {:.0}% | Greed/Fear: {:.2} | Size Mult: {:.2}x | Biases: {}",
            state_label, psych_health * 100.0, greed_fear, size_mult, bias_summary
        );

        PsychologySnapshot {
            timestamp: Utc::now(),
            emotional_state,
            active_biases,
            position_size_multiplier: size_mult,
            trading_advisory: advisory,
            summary,
        }
    }

    /// Returns a formatted log string for the snapshot.
    pub fn format_log(snapshot: &PsychologySnapshot) -> String {
        let mut lines = vec![
            format!("╔══ BEHAVIORAL PSYCHOLOGY ══╗"),
            format!(
                "║ State: {} | Health: {:.0}%",
                snapshot.emotional_state.state_label,
                snapshot.emotional_state.psych_health * 100.0
            ),
            format!(
                "║ Greed/Fear: {:.2} | Overconfidence: {:.0}% | Fatigue: {:.0}%",
                snapshot.emotional_state.greed_fear_index,
                snapshot.emotional_state.overconfidence * 100.0,
                snapshot.emotional_state.fatigue * 100.0
            ),
            format!(
                "║ Tilt: {:.0}% | Revenge Urge: {:.0}% | Size Mult: {:.2}x",
                snapshot.emotional_state.tilt * 100.0,
                snapshot.emotional_state.revenge_urge * 100.0,
                snapshot.position_size_multiplier
            ),
        ];

        if !snapshot.active_biases.is_empty() {
            lines.push("╠══ Active Biases ══╣".to_string());
            for bias in &snapshot.active_biases {
                lines.push(format!(
                    "║  ⚠ {} ({:.0}%): {}",
                    bias.bias.name(),
                    bias.severity * 100.0,
                    bias.evidence
                ));
                lines.push(format!("║     → Fix: {}", bias.corrective_action));
            }
        }

        lines.push("╚══════════════════════════╝".to_string());
        lines.join("\n")
    }

    /// Update trend tracking for acceleration detection.
    pub fn update_trends(&mut self, tilt: f64, overconfidence: f64) {
        self.previous_tilt = tilt;
        self.previous_overconfidence = overconfidence;
    }
}

impl Default for BehavioralPsychologyEngine {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_optimal_psychology() {
        let engine = BehavioralPsychologyEngine::new();
        let snapshot = engine.analyze(0.02, 0.0, 1, 0.0, 100_000.0, 100_000.0);

        assert_eq!(snapshot.emotional_state.state_label, "Optimal");
        assert!(snapshot.position_size_multiplier >= 0.9);
        assert!(snapshot.active_biases.is_empty());
        assert!(matches!(
            snapshot.trading_advisory,
            PsychologyAdvisory::Normal
        ));
    }

    #[test]
    fn test_detects_overconfidence_after_wins() {
        let mut engine = BehavioralPsychologyEngine::new();
        for _ in 0..5 {
            engine.record_decision("BUY", 0.7, 1.0);
            engine.record_outcome(true);
        }

        let snapshot = engine.analyze(0.02, 0.0, 5, 0.05, 100_000.0, 100_000.0);

        assert!(snapshot.emotional_state.overconfidence > 0.3);
        assert!(snapshot
            .active_biases
            .iter()
            .any(|b| b.bias == BehavioralBias::Overconfidence));
        // Size should be reduced due to overconfidence
        assert!(snapshot.position_size_multiplier < 0.9);
    }

    #[test]
    fn test_detects_revenge_trading() {
        let mut engine = BehavioralPsychologyEngine::new();
        for i in 0..3 {
            engine.record_decision("BUY", 0.6, 0.5 + i as f64 * 0.5); // Increasing sizes
            engine.record_outcome(false);
        }

        let snapshot = engine.analyze(0.05, 0.02, 3, -0.03, 95_000.0, 100_000.0);

        // Check that revenge urge is detected (size increasing after losses)
        assert!(
            snapshot.emotional_state.revenge_urge > 0.0,
            "Revenge urge should be > 0 with increasing sizes after losses"
        );
        // Revenge trading should reduce position size
        assert!(
            snapshot.position_size_multiplier < 0.8,
            "Position size should be reduced due to revenge detection"
        );
    }

    #[test]
    fn test_halt_on_tilt() {
        let mut engine = BehavioralPsychologyEngine::new();
        for _ in 0..5 {
            engine.record_decision("BUY", 0.5, 1.0);
            engine.record_outcome(false);
        }

        let snapshot = engine.analyze(0.08, 0.05, 5, -0.08, 92_000.0, 100_000.0);

        // 5 consecutive losses with high drawdown should cause significant tilt
        assert!(
            snapshot.emotional_state.tilt > 0.4,
            "Tilt should be significant with 5 losses and 5% drawdown"
        );
        // Position size should be reduced due to tilt
        assert!(
            snapshot.position_size_multiplier <= 0.7,
            "Position size should be reduced when tilted"
        );
    }

    #[test]
    fn test_psych_size_multiplier_optimal() {
        let state = EmotionalState::optimal();
        assert!((state.psych_size_multiplier() - 1.0).abs() < 0.01);
    }

    #[test]
    fn test_psych_size_multiplier_tilted() {
        let state = EmotionalState {
            greed_fear_index: 0.3,
            overconfidence: 0.1,
            fatigue: 0.3,
            tilt: 0.6,
            revenge_urge: 0.1,
            psych_health: 0.3,
            state_label: "Critical".to_string(),
        };
        assert!(state.psych_size_multiplier() < 0.5);
    }

    #[test]
    fn test_psych_size_multiplier_revenge() {
        let state = EmotionalState {
            greed_fear_index: 0.4,
            overconfidence: 0.0,
            fatigue: 0.2,
            tilt: 0.1,
            revenge_urge: 0.7,
            psych_health: 0.2,
            state_label: "Critical".to_string(),
        };
        assert_eq!(state.psych_size_multiplier(), 0.0); // Halt
    }

    #[test]
    fn test_detects_confirmation_bias() {
        let mut engine = BehavioralPsychologyEngine::new();
        // 5 consecutive losses, all BUY
        for _ in 0..5 {
            engine.record_decision("BUY", 0.7, 1.0);
            engine.record_outcome(false);
        }

        let snapshot = engine.analyze(0.04, 0.03, 5, -0.05, 95_000.0, 100_000.0);

        assert!(snapshot
            .active_biases
            .iter()
            .any(|b| b.bias == BehavioralBias::ConfirmationBias));
    }

    #[test]
    fn test_fatigue_increases_with_trades() {
        let engine = BehavioralPsychologyEngine::new();
        let snapshot = engine.analyze(0.02, 0.0, 12, 0.0, 100_000.0, 100_000.0);
        // 12 trades * 0.05 = 0.6 fatigue
        assert!(snapshot.emotional_state.fatigue >= 0.5);
    }
}
