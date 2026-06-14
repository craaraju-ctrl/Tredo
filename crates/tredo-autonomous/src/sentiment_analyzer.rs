// SentimentAnalyzer skill (implements AgentSkill for "how to do" sentiment analysis).
// Part of strong skills set: tells agents "how" to compute sentiment from news (pluggable tool).
// Research: Modular perception skills like in TradingAgents.

use crate::state::SharedState;
use async_trait::async_trait;
use std::error::Error;
use tredo_core::{skills::AgentSkill, AgentInput, AgentOutput};

pub struct SentimentAnalyzer {
    pub state: SharedState,
}

impl SentimentAnalyzer {
    pub fn new(state: SharedState) -> Self {
        Self { state }
    }

    /// Analyzes sentiment from latest news context.
    pub async fn analyze_sentiment(&self, symbol: &str) -> f64 {
        let news = self.state.latest_news.read().await;
        if let Some(ctx) = news.get(symbol) {
            let summary = &ctx.summary.to_lowercase();
            let pos = ["bull", "up", "gain", "positive", "rise", "buy"]
                .iter()
                .filter(|&w| summary.contains(w))
                .count() as f64;
            let neg = ["bear", "down", "loss", "negative", "fall", "sell"]
                .iter()
                .filter(|&w| summary.contains(w))
                .count() as f64;
            let score = ((pos - neg) / (pos + neg + 1.0) + 0.5).clamp(0.0, 1.0);
            return score;
        }
        0.5 // neutral
    }
}

#[async_trait]
impl AgentSkill for SentimentAnalyzer {
    fn name(&self) -> &str {
        "SentimentAnalyzer"
    }
    fn description(&self) -> &str {
        "Computes bullish/bearish sentiment score from news summary for a symbol (how to gauge market mood from text)."
    }

    async fn execute(
        &self,
        input: &AgentInput,
    ) -> Result<AgentOutput, Box<dyn Error + Send + Sync>> {
        if let AgentInput::ConfluenceRequest { context } = input {
            let score = self.analyze_sentiment(&context.symbol).await;
            // Return as simple output (in real, could be richer AgentOutput variant).
            // For now, agents use the method directly too; this enables pluggable Vec<dyn AgentSkill>.
            println!(
                "[Skill] {} executed for {}: score={:.2}",
                self.name(),
                context.symbol,
                score
            );
            Ok(AgentOutput::SkillResult {
                name: self.name().to_string(),
                score,
                note: "news keyword sentiment".to_string(),
                confidence: 0.6,
                direction: if score > 0.55 {
                    tredo_core::agent::SkillDirection::Bullish
                } else if score < 0.45 {
                    tredo_core::agent::SkillDirection::Bearish
                } else {
                    tredo_core::agent::SkillDirection::Neutral
                },
                weight: 0.3,
            })
        } else {
            Ok(AgentOutput::Done)
        }
    }
}
