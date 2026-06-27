use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct WorldModelSnapshot {
    pub symbol_beliefs: HashMap<String, SymbolBelief>,
    pub cross_symbol_beliefs: Vec<CrossSymbolBelief>,
    pub macro_beliefs: MacroBeliefs,
    pub active_hypotheses: Vec<Hypothesis>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SymbolBelief {
    pub symbol: String,
    pub trend: f64,
    pub trend_confidence: f64,
    pub volatility_regime: VolatilityRegime,
    pub smart_money_activity: SmartMoneyActivity,
    pub belief_age_secs: i64,
    pub last_updated: chrono::DateTime<chrono::Utc>,
}

impl SymbolBelief {
    pub fn default_for(symbol: &str) -> Self {
        Self {
            symbol: symbol.to_string(),
            trend: 0.0,
            trend_confidence: 0.0,
            volatility_regime: VolatilityRegime::Normal,
            smart_money_activity: SmartMoneyActivity::Quiet,
            belief_age_secs: 0,
            last_updated: chrono::Utc::now(),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum VolatilityRegime {
    Compressed,
    Normal,
    Elevated,
    Extreme,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum SmartMoneyActivity {
    Accumulating,
    Distributing,
    Active,
    Quiet,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CrossSymbolBelief {
    pub pair: (String, String),
    pub correlation_belief: f64,
    pub relative_strength: f64,
    pub rationale: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct MacroBeliefs {
    pub overall_market_sentiment: f64,
    pub fear_greed_index_estimate: f64,
    pub key_narratives: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Hypothesis {
    pub id: String,
    pub statement: String,
    pub prior: f64,
    pub evidence_for: f64,
    pub evidence_against: f64,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub test_plan: String,
    pub status: HypothesisStatus,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum HypothesisStatus {
    Active,
    Confirmed,
    Rejected,
    Untestable,
}

#[derive(Debug, Clone)]
pub enum Evidence {
    PriceMove {
        magnitude: f64,
        direction: f64,
        volume_confirmation: bool,
    },
    VolumeSpike {
        ratio: f64,
    },
    NewsImpact {
        sentiment: f64,
        magnitude: f64,
    },
    ForecastReceived {
        median_change: f64,
        uncertainty: f64,
    },
}

#[derive(Debug, Clone)]
pub enum HypotheticalAction {
    GoLong,
    GoShort,
    HoldAndObserve,
    ReduceExposure,
    IncreaseExposure,
}

#[derive(Debug, Clone)]
pub enum WhatIfResult {
    Favorable {
        confidence: f64,
        expected_outcome: String,
    },
    Unfavorable {
        confidence: f64,
        expected_outcome: String,
    },
    Neutral {
        outcome: String,
    },
    Uncertain {
        reason: String,
    },
}

pub struct WorldModelEngine {
    model: parking_lot::RwLock<WorldModelSnapshot>,
    cross_symbol: parking_lot::RwLock<Vec<CrossSymbolBelief>>,
    macro_beliefs: parking_lot::RwLock<MacroBeliefs>,
    hypotheses: parking_lot::RwLock<Vec<Hypothesis>>,
}

impl WorldModelEngine {
    pub fn new() -> Self {
        Self {
            model: parking_lot::RwLock::new(WorldModelSnapshot::default()),
            cross_symbol: parking_lot::RwLock::new(Vec::new()),
            macro_beliefs: parking_lot::RwLock::new(MacroBeliefs::default()),
            hypotheses: parking_lot::RwLock::new(Vec::new()),
        }
    }

    pub fn update_belief(&self, symbol: &str, evidence: Evidence) {
        let mut model = self.model.write();
        let belief = model
            .symbol_beliefs
            .entry(symbol.to_string())
            .or_insert_with(|| SymbolBelief::default_for(symbol));
        match evidence {
            Evidence::PriceMove {
                magnitude,
                direction,
                volume_confirmation,
            } => {
                if volume_confirmation && magnitude.abs() > 0.02 {
                    belief.trend = belief.trend * 0.7 + direction.signum() * 0.3;
                    belief.trend_confidence = (belief.trend_confidence * 0.9 + 0.1).min(1.0);
                }
            }
            Evidence::VolumeSpike { ratio } => {
                if ratio > 3.0 {
                    belief.smart_money_activity = SmartMoneyActivity::Active;
                }
            }
            Evidence::NewsImpact {
                sentiment,
                magnitude,
            } => {
                belief.trend = belief.trend * 0.8 + sentiment.signum() * magnitude * 0.2;
            }
            Evidence::ForecastReceived {
                median_change,
                uncertainty,
            } => {
                if uncertainty < 0.01 {
                    belief.trend = belief.trend * 0.9 + median_change.signum() * 0.1;
                } else {
                    belief.trend_confidence *= 0.95;
                }
            }
        }
        belief.belief_age_secs = 0;
        belief.last_updated = chrono::Utc::now();
    }

    pub fn form_hypothesis(&self, statement: String, prior: f64, test_plan: String) {
        let mut hypotheses = self.hypotheses.write();
        hypotheses.push(Hypothesis {
            id: format!("hyp-{}", Uuid::new_v4()),
            statement,
            prior,
            evidence_for: 1.0,
            evidence_against: 1.0,
            created_at: chrono::Utc::now(),
            test_plan,
            status: HypothesisStatus::Active,
        });
    }

    pub fn what_if(&self, symbol: &str, action: HypotheticalAction) -> WhatIfResult {
        let model = self.model.read();
        let belief = model.symbol_beliefs.get(symbol);
        match (action, belief) {
            (HypotheticalAction::GoLong, Some(b)) if b.trend > 0.5 => WhatIfResult::Favorable {
                confidence: b.trend_confidence,
                expected_outcome: "Aligns with strong bullish trend".to_string(),
            },
            (HypotheticalAction::GoLong, Some(b)) if b.trend < -0.5 => WhatIfResult::Unfavorable {
                confidence: b.trend_confidence,
                expected_outcome: "Against bearish trend".to_string(),
            },
            (HypotheticalAction::HoldAndObserve, _) => WhatIfResult::Neutral {
                outcome: "Wait for more data".to_string(),
            },
            _ => WhatIfResult::Uncertain {
                reason: "Insufficient belief state".to_string(),
            },
        }
    }

    pub fn snapshot(&self) -> WorldModelSnapshot {
        let mut snap = self.model.read().clone();
        snap.cross_symbol_beliefs = self.cross_symbol.read().clone();
        snap.macro_beliefs = self.macro_beliefs.read().clone();
        snap.active_hypotheses = self.hypotheses.read().clone();
        snap
    }

    pub fn active_hypotheses(&self) -> Vec<Hypothesis> {
        self.hypotheses.read().clone()
    }
}

impl Default for WorldModelEngine {
    fn default() -> Self {
        Self::new()
    }
}
