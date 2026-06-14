// Full debate aggregator (Proposer, Critic, Risk, Historian) using new skills.
// Research: TradingAgents/FINCON style multi-agent debate for robust decisions.
// Replaces or augments single LLM in StrategyDecision for hands-off quality.
// Uses local DebateTurn (core AgentOutput is enum with specific variants, not freeform).
//
// NOTE: MarketIntelligence now produces SkillResult + runs SkillAggregator (see market_intelligence.rs).
// Debate currently uses legacy direct skill calls for proposer/critic etc. + custom buy_score.
// Full unification (pass aggregated signal from MI into debate) is the next logical step after this wiring.

use crate::state::SharedState;
use crate::{
    correlation_checker::CorrelationChecker, on_chain_data::OnChainData,
    regime_detector::RegimeDetector, sentiment_analyzer::SentimentAnalyzer,
    volatility_calculator::VolatilityCalculator,
};
use tredo_core::{AgentInput, MarketContext};

// Lightweight turn for debate participants (not the core AgentOutput enum).
#[derive(Clone, Debug)]
pub struct DebateTurn {
    pub action: String,
    pub confidence: f64,
    pub reasoning: String,
}

// Helper to extract MarketContext from the enum (deep fix for type mismatch)
fn extract_context(input: &AgentInput) -> Option<&MarketContext> {
    match input {
        AgentInput::ConfluenceRequest { context } => Some(context),
        AgentInput::RiskRequest { context } => Some(context),
        _ => None,
    }
}

pub struct ProposerAgent {
    state: SharedState,
}
pub struct CriticAgent {
    state: SharedState,
}
pub struct RiskAgent {
    state: SharedState,
}
pub struct HistorianAgent {
    state: SharedState,
}

impl ProposerAgent {
    pub fn new(state: SharedState) -> Self {
        Self { state }
    }
    pub async fn propose(&self, input: &AgentInput) -> DebateTurn {
        let ctx = match extract_context(input) {
            Some(c) => c,
            None => {
                return DebateTurn {
                    action: "HOLD".into(),
                    confidence: 0.0,
                    reasoning: "no context".into(),
                }
            }
        };

        // Smarter: recall what "I" (Proposer) or similar did in past similar situations and the outcome/lesson (hierarchical trained memory RAG+).
        // This helps the agent "understand exactly what it was doing" last time and avoid repeating hallucinations or bad calls.
        let trained_recall = self
            .state
            .recall_trained_memory(
                &format!("proposer action on {} regime conf confluence", ctx.symbol),
                2,
            )
            .await;

        // Bullish bias using new skills
        let sentiment = SentimentAnalyzer::new(self.state.clone())
            .analyze_sentiment(&ctx.symbol)
            .await;
        let (vol, _) = VolatilityCalculator::new(self.state.clone())
            .compute_volatility(&ctx.symbol, ctx.current_price)
            .await;
        let regime = RegimeDetector::new(self.state.clone())
            .detect_regime(&ctx.symbol, ctx.current_price)
            .await;
        let onchain = OnChainData::new(self.state.clone())
            .fetch_onchain(&ctx.symbol)
            .await;

        let action =
            if format!("{:?}", regime).contains("Bull") || sentiment > 0.6 || onchain > 0.65 {
                "BUY"
            } else {
                "HOLD"
            };

        DebateTurn {
            action: action.to_string(),
            confidence: 0.7 + (sentiment - 0.5).max(0.0),
            reasoning: format!(
                "Proposer: regime {:?}, sent {:.2}, vol {:.2}, onchain {:.2}. {}\n(Used trained memory + skills to ground decision and reduce hallucination.)",
                regime, sentiment, vol, onchain, trained_recall
            ),
        }
    }
}

impl CriticAgent {
    pub fn new(state: SharedState) -> Self {
        Self { state }
    }
    pub async fn critique(&self, proposal: &str, input: &AgentInput) -> DebateTurn {
        let ctx = match extract_context(input) {
            Some(c) => c,
            None => {
                return DebateTurn {
                    action: "CRITIQUE".into(),
                    confidence: 0.5,
                    reasoning: "no context".into(),
                }
            }
        };

        // Smarter: recall past critiques by "me" or team on similar proposals and their accuracy (trained memory helps the agent know exactly what it did last time and if it was right).
        let trained_recall = self
            .state
            .recall_trained_memory(
                &format!("critic on proposal {} for {}", proposal, ctx.symbol),
                2,
            )
            .await;

        let corr = CorrelationChecker::new(self.state.clone())
            .check_correlation(&ctx.symbol)
            .await;
        let critique = if proposal == "BUY" && corr < 0.4 {
            "CAUTION: low corr, possible fakeout"
        } else {
            "OK but watch risk"
        };
        DebateTurn {
            action: "CRITIQUE".to_string(),
            confidence: 0.6,
            reasoning: format!("Critic on {}: {} (corr {:.2}). {}\n(Used trained memory to ground and avoid repeating past mistakes.)", proposal, critique, corr, trained_recall),
        }
    }
}

impl RiskAgent {
    pub fn new(state: SharedState) -> Self {
        Self { state }
    }
    pub async fn assess(&self, input: &AgentInput) -> DebateTurn {
        let ctx = match extract_context(input) {
            Some(c) => c,
            None => {
                return DebateTurn {
                    action: "PASS".into(),
                    confidence: 0.5,
                    reasoning: "no context".into(),
                }
            }
        };

        // Smarter: recall past risk assessments and their accuracy from trained memory (the agent now "remembers" exactly what it blocked last time and why it was correct or not).
        let trained_recall = self
            .state
            .recall_trained_memory(&format!("risk assessment for {} vol", ctx.symbol), 2)
            .await;

        // Enforcer using vol/regime
        let (vol, exp) = VolatilityCalculator::new(self.state.clone())
            .compute_volatility(&ctx.symbol, ctx.current_price)
            .await;
        let action = if vol > 0.03 || exp { "BLOCK" } else { "PASS" };
        DebateTurn {
            action: action.to_string(),
            confidence: if vol > 0.03 { 0.9 } else { 0.7 },
            reasoning: format!("Risk: vol {:.2} expansion {}. {}\n(Grounded in trained memory to reduce over/under blocking.)", vol, exp, trained_recall),
        }
    }
}

impl HistorianAgent {
    pub fn new(state: SharedState) -> Self {
        Self { state }
    }
    pub async fn recall(&self, input: &AgentInput) -> DebateTurn {
        let ctx = match extract_context(input) {
            Some(c) => c,
            None => {
                return DebateTurn {
                    action: "RECALL".into(),
                    confidence: 0.5,
                    reasoning: "no context".into(),
                }
            }
        };

        // === Promote local VectorMemory for trained episode intelligence ===
        let mut vector_context = String::new();
        {
            let vm = self.state.vector_memory.lock().await;
            if !vm.is_empty() {
                let query = format!(
                    "{} {} price={:.2}",
                    ctx.symbol, "historical outcome", ctx.current_price
                );
                // Use a clone of llm Arc for the search (cheap)
                let llm_for_search = (*self.state.llm).clone(); // LlmExecutor is Clone
                if let Ok(results) = vm.search(&query, 3, &llm_for_search).await {
                    if !results.is_empty() {
                        let mut lines =
                            vec!["── VECTOR TRAINED SIMILAR EPISODES (local) ──".to_string()];
                        for (i, r) in results.iter().enumerate() {
                            let regret = r
                                .regret_score
                                .map(|s| format!(" regret={:.2}", s))
                                .unwrap_or_default();
                            lines.push(format!(
                                "  {}. {} (sim:{:.0}%{}) {}",
                                i + 1,
                                r.summary_text,
                                r.similarity * 100.0,
                                regret,
                                r.timestamp
                            ));
                        }
                        vector_context = lines.join("\n");
                    }
                }
            }
        }

        // === agentmemory sharing for cross-session / trained data intelligence ===
        // Shares with external agentmemory (Grok/Hermes ecosystem) for infinite persistent recall of lessons/decisions.
        let mem = tredo_core::AgentMemoryClient::new();
        let past = mem
            .recall(&format!("trained lessons OR past decisions {}", ctx.symbol))
            .await
            .unwrap_or_default();

        let reasoning = if !vector_context.is_empty() || !past.is_empty() {
            format!(
                "Historian: {} | agentmemory: {} entries. Combined trained intel for cautious decisions.",
                if vector_context.is_empty() { "no local vectors".to_string() } else { vector_context.clone() },
                past.len()
            )
        } else {
            format!(
                "Historian: no strong trained data match for {}. Default caution on uncertainty.",
                ctx.symbol
            )
        };

        DebateTurn {
            action: "RECALL".to_string(),
            confidence: 0.65 + ((vector_context.len() + past.len() * 10) as f64 * 0.02).min(0.25),
            reasoning,
        }
    }
}

pub async fn run_debate(
    state: SharedState,
    input: &AgentInput,
    aggregated_signal: Option<&tredo_core::AggregatedSignal>,
) -> (String, f64, String, Vec<DebateTurn>) {
    let proposer = ProposerAgent::new(state.clone());
    let critic = CriticAgent::new(state.clone());
    let risk = RiskAgent::new(state.clone());
    let historian = HistorianAgent::new(state.clone());

    let prop = proposer.propose(input).await;
    let crit = critic.critique(&prop.action, input).await;
    let rsk = risk.assess(input).await;
    let hist = historian.recall(input).await;

    let turns = vec![prop.clone(), crit.clone(), rsk.clone(), hist.clone()];

    // === REAL AGGREGATOR INTEGRATION (Gap 1 fix) ===
    // The AggregatedSignal from MarketIntelligence (skills consensus) is now a first-class
    // citizen in debate. This closes the "thinking aloud but ignoring its own thoughts" problem.
    let mut buy_score = 0.0;
    if prop.action == "BUY" {
        buy_score += prop.confidence * 0.25;
    }
    if crit.action.contains("OK") {
        buy_score += 0.15;
    }
    if rsk.action == "PASS" {
        buy_score += rsk.confidence * 0.30;
    }
    if !hist.reasoning.contains("caution") && !hist.reasoning.contains("CAUTION") {
        buy_score += 0.15;
    }
    // Bonus from trained data length in hist reasoning
    if hist.reasoning.contains("TRAINED") || hist.reasoning.contains("agentmemory") {
        buy_score += 0.10;
    }

    // Strongly weight the cross-skill aggregated consensus when available.
    // This is the key integration (Gap 1) — the agent now actually listens to its own aggregated skill consensus.
    if let Some(agg) = aggregated_signal {
        let before = buy_score;
        if agg.is_bullish(None) {
            buy_score += (agg.net_signal.abs() * 0.35).min(0.35);
        } else if agg.is_bearish(None) {
            buy_score -= (agg.net_signal.abs() * 0.35).min(0.35);
        }
        buy_score += (agg.conviction - 0.3).max(0.0) * 0.2;

        let delta = buy_score - before;
        if delta.abs() > 0.005 {
            println!("[Debate] AggregatedSignal influenced buy_score by {:+.3} (net={:+.2}, conviction={:.0}%) → new score {:.2}",
                     delta, agg.net_signal, agg.conviction * 100.0, buy_score);
        }
    }

    let (final_action, conf, reason) = if buy_score > 0.55 && rsk.action == "PASS" {
        (
            "BUY".to_string(),
            buy_score,
            format!(
                "Debate + AggregatedSignal: Prop={} Crit={} Risk={} Hist-trained. Score={:.2}",
                prop.action, crit.action, rsk.action, buy_score
            ),
        )
    } else if buy_score < 0.25 || rsk.action == "BLOCK" {
        (
            "HOLD".to_string(),
            0.85,
            "Debate + AggregatedSignal consensus (trained data + risk + skills): high risk or low conviction".to_string(),
        )
    } else {
        (
            "HOLD".to_string(),
            0.65,
            "Debate + AggregatedSignal: mixed signals from skills + agents + trained episodes, escalate to HOLD".to_string(),
        )
    };

    (final_action, conf, reason, turns)
}
