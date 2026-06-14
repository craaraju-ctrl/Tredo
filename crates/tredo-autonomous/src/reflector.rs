use crate::state::SharedState;
use async_trait::async_trait;
use chrono::Utc;
use std::error::Error;
use tredo_core::{
    Agent, AgentInput, AgentOutput, AgentTier, LlmExecutor, PostTradeReflection, TradingEpisode,
};

pub struct ReflectorAgent {
    pub state: SharedState,
}

impl ReflectorAgent {
    pub fn new(state: SharedState) -> Self {
        Self { state }
    }

    /// Lightweight daily reflection — reads today's portfolio state.
    pub async fn reflect(&self, symbol: &str) -> Result<String, Box<dyn Error + Send + Sync>> {
        println!("[Reflector] Reflecting on past decisions for {}...", symbol);

        let today_key = format!("decisions/{}/{}", symbol, Utc::now().format("%Y%m%d"));
        let _recent = self.state.memory.get_decision(&today_key).ok().flatten();

        let pattern_key = format!("patterns/{}", symbol);
        let _patterns = self.state.memory.get_decision(&pattern_key).ok().flatten();

        let portfolio = self.state.portfolio.read().await;

        let reflection = format!(
            "Reflection for {}: Daily P&L: ₹{:.2} | Trades: {} | Wins: {} | Losses: {} | Consecutive Losses: {}",
            symbol, portfolio.daily_pnl, portfolio.total_trades_today,
            portfolio.winning_trades_today, portfolio.losing_trades_today,
            portfolio.consecutive_losses
        );

        println!("[Reflector] {}", reflection);

        let reflection_key = format!("reflections/{}/{}", symbol, Utc::now().timestamp());
        let _ = self
            .state
            .memory
            .store_decision(&reflection_key, &reflection);

        Ok(reflection)
    }

    /// Deep post-trade reflection — analyses a specific closed trade via the LLM,
    /// generates a structured PostTradeReflection, stores the episode in memory.
    pub async fn deep_reflect_on_episode(
        &self,
        episode: &TradingEpisode,
        llm: &LlmExecutor,
    ) -> Result<PostTradeReflection, Box<dyn Error + Send + Sync>> {
        println!(
            "[Reflector] 🔬 Deep reflecting on episode {}...",
            episode.episode_id
        );

        // Smarter: before reflecting, recall what "I" (Reflector) or the system did in past similar episodes for this symbol/action, and the previous lessons (this helps the agent understand exactly what it was doing last time and build better reflections, reducing hallucinated lessons).
        let trained_recall = self
            .state
            .recall_trained_memory(
                &format!("reflection on {} {} action", episode.symbol, episode.action),
                2,
            )
            .await;
        println!(
            "[Reflector smarter] Using trained memory for better self-understanding: {}",
            trained_recall
        );

        let episode_summary = format!(
            "Symbol: {} | Action: {} | Entry: {:.2} | SL: {:.2} | TP: {:.2} | Confidence: {:.1}%\nMarket: price={:.2} trend={} confluence={:.1}% regime={} session={}",
            episode.symbol, episode.action, episode.entry_price,
            episode.stop_loss, episode.take_profit,
            episode.confidence * 100.0,
            episode.market_state.price, episode.market_state.trend,
            episode.market_state.confluence * 100.0,
            episode.market_state.regime, episode.market_state.session_valid,
        );

        let outcome_summary = match &episode.outcome {
            Some(o) => format!(
                "Exit: {:.2} | P&L: ₹{:.2} ({:+.2}%) | Reason: {} | Held: {}s | Max: {:.2} | Min: {:.2}",
                o.exit_price, o.pnl, o.pnl_pct * 100.0,
                o.exit_reason, o.holding_period_secs,
                o.max_unrealized_pnl, o.min_unrealized_pnl,
            ),
            None => "Trade still open or no outcome recorded.".to_string(),
        };

        let reflection = llm
            .ask_for_reflection(&episode_summary, &outcome_summary)
            .await;

        // Store the reflection in memory alongside the episode
        if let Ok(json) = serde_json::to_string(&reflection) {
            let key = format!("reflection/{}", episode.episode_id);
            let _ = self.state.memory.store_state(&key, &json);
        }

        // Promote to vector memory for trained intelligence (semantic search in debate/historian)
        let summary = format!(
            "{} reflection: lesson={} regret={:.2}",
            episode.symbol, reflection.lesson, reflection.regret_score
        );
        let vm = self.state.vector_memory.clone();
        let llm_ref = self.state.llm.clone();
        let eid = episode.episode_id.clone();
        let sym = episode.symbol.clone();
        tokio::spawn(async move {
            let mut v = vm.lock().await;
            let _ = v
                .store(
                    &eid,
                    &sym,
                    &summary,
                    Some(reflection.regret_score),
                    &llm_ref,
                )
                .await;
        });

        // If there's a suggested rule change, store it for the MetaControlAgent
        if let Some(ref change) = reflection.suggested_rule_change {
            let key = format!("rule_suggestion/{}", Utc::now().timestamp());
            let _ = self.state.memory.store_state(&key, change);
            println!("[Reflector] 💡 Rule change suggestion stored: {}", change);
        }

        if reflection.should_alert {
            println!("[Reflector] 🚨 CRITICAL LESSON: {}", reflection.lesson);
            // Wire Notifier
            tredo_core::notifier::alert(
                "CRITICAL REFLECTION",
                &format!(
                    "{}: {} (regret {:.2})",
                    episode.symbol, reflection.lesson, reflection.regret_score
                ),
            )
            .await;
        }

        println!(
            "[Reflector] ✅ Deep reflection complete — regret: {:.2}, lesson: {}",
            reflection.regret_score, reflection.lesson
        );

        Ok(reflection)
    }
}

#[async_trait]
impl Agent for ReflectorAgent {
    fn name(&self) -> &str {
        "ReflectorAgent"
    }
    fn tier(&self) -> AgentTier {
        AgentTier::Main
    }

    async fn run(
        &self,
        input: Option<AgentInput>,
    ) -> Result<AgentOutput, Box<dyn Error + Send + Sync>> {
        let symbol = match &input {
            Some(AgentInput::ConfluenceRequest { context }) => context.symbol.clone(),
            _ => "NIFTY".to_string(),
        };

        let _ = self.reflect(&symbol).await;
        Ok(AgentOutput::Done)
    }
}
