use crate::debate::run_debate;
use crate::helpers::{calculate_position_size, calculate_risk_reward, get_indian_session_info};
use crate::state::SharedState;
use crate::types::TradeSignal;
use async_trait::async_trait;
use chrono::Utc;
use std::error::Error;
use tredo_core::{
    calculate_confluence_score, calculate_pivot_points, validate_trade_setup, Agent, AgentInput,
    AgentOutput, AgentTier, MarketContext,
}; // Full debate aggregator wired (Proposer etc using new skills)

pub struct StrategyDecisionAgent {
    pub state: SharedState,
}

impl StrategyDecisionAgent {
    pub fn new(state: SharedState) -> Self {
        Self { state }
    }

    /// Generate a trade signal for a symbol by:
    /// 1. Reading the Kronos forecast stored by Phase 2 (MarketIntelligenceAgent)
    /// 2. Gathering calendar events, goals, and multi-timeframe context
    /// 3. Asking the Ollama LLM to decide BUY / SELL / HOLD with entry, SL, TP
    /// 4. Validating the decision against discipline rules
    /// 5. Returning the populated TradeSignal (or None if HOLD)
    pub async fn generate_signal(
        &self,
        symbol: &str,
        _direction: tredo_core::TradeDirection,
        entry: f64,
        _stop: f64,
        _target: f64,
    ) -> Result<Option<TradeSignal>, Box<dyn Error + Send + Sync>> {
        let rules = self.state.rules.read().await;
        let portfolio = self.state.portfolio.read().await;

        let context = MarketContext {
            symbol: symbol.to_string(),
            current_price: entry,
            high: entry * 1.01,
            low: entry * 0.99,
            previous_close: entry * 0.998,
            timestamp: Utc::now(),
            daily_pnl: portfolio.daily_pnl,
            equity: portfolio.cash_balance
                + portfolio
                    .open_positions
                    .iter()
                    .map(|p| p.current_price * p.quantity)
                    .sum::<f64>(),
            consecutive_losses: portfolio.consecutive_losses,
            is_red_folder_day: false,
            trend_direction: None,
        };

        let pivots = calculate_pivot_points(
            context.high,
            context.low,
            context.previous_close,
            rules.pivot_method,
        );
        let confluence = calculate_confluence_score(&context, &pivots);
        let session = get_indian_session_info(Utc::now());

        // Pull Kronos forecast
        let forecast_summary = {
            let last = self.state.last_forecast.read().await;
            match last.as_ref() {
                Some(v) => v["summary"]
                    .as_str()
                    .unwrap_or("No forecast summary")
                    .to_string(),
                None => "Kronos unavailable".to_string(),
            }
        };

        // Trend label
        let trend_label = {
            let regime = self.state.market_regime.read().await;
            match *regime {
                Some(crate::types::MarketRegime::TrendingBull) => "Bullish",
                Some(crate::types::MarketRegime::TrendingBear) => "Bearish",
                Some(crate::types::MarketRegime::Ranging) => "Ranging",
                _ => "Neutral",
            }
        };

        let portfolio_heat: f64 = {
            let total_risk: f64 = portfolio.open_positions.iter().map(|p| p.risk_amount).sum();
            if portfolio.total_equity > 0.0 {
                total_risk / portfolio.total_equity
            } else {
                0.0
            }
        };
        let consecutive_losses = portfolio.consecutive_losses;
        let daily_pnl_pct = portfolio.daily_pnl_pct;
        let total_trades_today = portfolio.total_trades_today;
        drop(portfolio);
        drop(rules);

        // ── Run full debate (Rust priority for robust, self-aware decisions) ─
        // Note: call early before drops; use separate rules read for validate if needed
        let (debate_action, debate_conf, debate_reason, debate_turns) = crate::debate::run_debate(
            self.state.clone(),
            &AgentInput::ConfluenceRequest {
                context: context.clone(),
            },
        )
        .await;

        // Push debate turns to COT for observability
        for turn in &debate_turns {
            let _ = self
                .state
                .push_cot(
                    "debate",
                    &format!("input for {}", symbol),
                    &turn.action,
                    &turn.reasoning,
                    turn.confidence,
                    0,
                    None,
                    Some(symbol.to_string()),
                )
                .await;
        }

        // If debate gives strong signal, use it (reduces LLM reliance) - basic validate without dropped rules
        let rules_for_validate = self.state.rules.read().await;
        if debate_conf > 0.75
            && (debate_action == "BUY" || debate_action == "SELL")
            && validate_trade_setup(&context, &rules_for_validate).passed
        {
            let signal = TradeSignal {
                symbol: symbol.to_string(),
                direction: if debate_action == "BUY" {
                    tredo_core::TradeDirection::Long
                } else {
                    tredo_core::TradeDirection::Short
                },
                entry_price: context.current_price,
                stop_loss: {
                    let base_sl = context.current_price * 0.98;
                    if std::env::var("VALIDATION_INDUCE_REGRET")
                        .map(|v| v == "true")
                        .unwrap_or(false)
                    {
                        let tight = std::env::var("VALIDATION_TIGHT_SL_PCT")
                            .ok()
                            .and_then(|s| s.parse::<f64>().ok())
                            .unwrap_or(0.5)
                            / 100.0;
                        context.current_price * (1.0 - tight)
                    } else {
                        base_sl
                    }
                },
                take_profit: context.current_price * 1.04,
                position_size: 0.01, // default; risk layer will adjust
                confidence_score: debate_conf,
                confluence_score: 0.7,
                risk_reward_ratio: 2.0,
                reasoning: format!("DEBATE: {}", debate_reason),
                timestamp: Utc::now(),
                session_valid: true,
                risk_check_passed: true,
            };
            return Ok(Some(signal));
        }
        drop(rules_for_validate);

        // ── Gather enriched agentic context (for LLM fallback or synthesis) ─────────────────────────────────
        // Calendar events (upcoming high-impact events)
        let calendar_context = {
            let events = self.state.calendar_events.read().await;
            let today_events: Vec<String> = events
                .iter()
                .filter(|e| e.is_today() || e.is_upcoming(3))
                .map(|e| {
                    format!(
                        "{} [{}] {} — {}",
                        e.title, e.currency, e.date, e.description
                    )
                })
                .collect();
            if today_events.is_empty() {
                "No high-impact economic events today or in the next 3 days.".to_string()
            } else {
                today_events.join("\n")
            }
        };

        // Trading goals & mode
        let (trading_mode, daily_goal_context) = {
            let goals = self.state.trading_goals.read().await;
            let mode_str = match goals.mode {
                tredo_core::TradingMode::Aggressive => "Aggressive",
                tredo_core::TradingMode::Normal => "Normal",
                tredo_core::TradingMode::Conservative => "Conservative",
                tredo_core::TradingMode::Halted => "HALTED",
            };
            let goal = format!(
                "Daily target: {:+.2}% | Current P&L: {:+.2}% | Trades today: {}/{} | Mode: {}",
                goals.daily_target_pnl_pct * 100.0,
                daily_pnl_pct * 100.0,
                total_trades_today,
                goals.max_daily_trades,
                mode_str,
            );
            (mode_str.to_string(), goal)
        };

        // Multi-timeframe context (higher timeframe pivots)
        let multi_tf_context = {
            let mtf = self.state.multi_timeframe_data.read().await;
            match mtf.get(symbol) {
                Some(tf_data) => tf_data
                    .iter()
                    .map(|tf| {
                        let pivot_str = tf
                            .pivots
                            .as_ref()
                            .map(|p| format!("Pivot={:.2} R1={:.2} S1={:.2}", p.pivot, p.r1, p.s1))
                            .unwrap_or_else(|| "No pivot data".to_string());
                        format!(
                            "{}: Confluence={:.1}% | {}",
                            tf.timeframe,
                            tf.confluence * 100.0,
                            pivot_str
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("\n"),
                None => "Higher timeframe data not yet available.".to_string(),
            }
        };

        // Agent market summary (from reflection)
        let agent_market_summary = { self.state.agent_market_summary.read().await.clone() };

        // News context (from NewsFetcher, populated by medium loop)
        let news_context = {
            let news = self.state.latest_news.read().await;
            match news.get(symbol) {
                Some(ctx) => ctx.to_prompt_string(),
                None => "No recent news for this symbol.".to_string(),
            }
        };

        // Candlestick patterns (detected by MarketIntelligenceAgent)
        let patterns_context = {
            let pats = self.state.last_patterns.read().await;
            match pats.get(symbol) {
                Some(p) if !p.is_empty() => tredo_core::format_patterns(p),
                _ => String::new(),
            }
        };

        // Multi-timeframe pattern confirmation (cross-TF validation)
        let mtf_patterns_context = {
            let mtf = self.state.last_mtf_patterns.read().await;
            match mtf.get(symbol) {
                Some(p) if !p.timeframes_with_patterns.is_empty() => {
                    tredo_core::format_mtf_confirmation(p)
                }
                _ => String::new(),
            }
        };

        // Combine single-TF and multi-TF pattern context
        let combined_patterns_context = if !mtf_patterns_context.is_empty() {
            format!("{}\n\n{}", patterns_context, mtf_patterns_context)
        } else {
            patterns_context.clone()
        };

        // Similar episodes from vector memory (semantic similarity search)
        let similar_episodes_context = {
            let vm = self.state.vector_memory.lock().await;
            if !vm.is_empty() {
                let query = format!(
                    "{} {} trend={} confluence={:.1}% price={:.2}",
                    symbol,
                    trend_label,
                    trend_label,
                    confluence * 100.0,
                    entry
                );
                match vm.search(&query, 3, &self.state.llm).await {
                    Ok(results) if !results.is_empty() => {
                        let mut lines = vec!["── SIMILAR PAST EPISODES ──".to_string()];
                        for (i, r) in results.iter().enumerate() {
                            let regret = r
                                .regret_score
                                .map(|s| format!(" regret={:.2}", s))
                                .unwrap_or_default();
                            lines.push(format!(
                                "  {}. {} {} (sim: {:.0}%{}) {}",
                                i + 1,
                                r.symbol,
                                r.timestamp.format("%m/%d"),
                                r.similarity * 100.0,
                                regret,
                                r.summary_text,
                            ));
                        }
                        lines.join("\n")
                    }
                    _ => String::new(),
                }
            } else {
                String::new()
            }
        };

        // ── Full debate aggregator (easy extension: Proposer/Critic/Risk/Historian + new skills) ──
        // Research-backed for robust hands-off decisions before LLM.
        let debate_input = AgentInput::ConfluenceRequest {
            context: context.clone(),
        };

        // Strong rules + trained memory (from user request): Rules (DisciplinedCore) tell "what to do and what not to do".
        // Skills tell "how to do" (pluggable via AgentSkill trait: sentiment, vol, trained recall, etc.).
        // Agents/sub-agents already know "what to do" (their roles in Tredo hierarchy: Identifier etc.).
        // Hierarchical trained memory (RAG+ vector + agentmemory) makes them "understand exactly what they were doing" in past (recall past actions/outcomes/lessons).
        // This + skills/rules = smarter execution, long-term improvement (as trained memory grows), reduced hallucinations (grounded in real history, not prompt).
        let trained_recall = self
            .state
            .recall_trained_memory(
                &format!(
                    "pre-decision rules for {} confluence {}",
                    symbol, confluence
                ),
                3,
            )
            .await;

        {
            let mut rules = self.state.rules.write().await;
            tredo_core::apply_trained_memory_to_rules(&mut rules, &trained_recall);
        }

        self.state.push_cot(
            "StrongRules+Skills+TrainedMemory",
            &format!("Rules/skills check before debate for {}", symbol),
            "RULES_SKILLS_APPLIED",
            &format!("Trained memory + rules updated (skills available: sentiment/vol/etc). Recall: {}", if trained_recall.len() > 50 { &trained_recall[..50] } else { &trained_recall }),
            0.9,
            0,
            None,
            Some(symbol.to_string()),
        ).await;
        let (debate_action, debate_conf, debate_reason, debate_turns) =
            run_debate(self.state.clone(), &debate_input).await;

        // Push debate turns to COT for full observability (finishes wiring)
        let debate_chain = self
            .state
            .start_cot_chain(
                "DebateCoordinator",
                &format!("Debate for {} decision", symbol),
                "DEBATE_RUN",
                "Multi-agent debate with trained vector + agentmemory intel",
                debate_conf,
            )
            .await;
        for (i, turn) in debate_turns.iter().enumerate() {
            let _ = self
                .state
                .add_cot_step(
                    debate_chain,
                    &format!(
                        "Debate-{}",
                        ["Proposer", "Critic", "Risk", "Historian"][i.min(3)]
                    ),
                    &format!("Debate input for {}", symbol),
                    &turn.action,
                    &turn.reasoning,
                    turn.confidence,
                    Some(symbol.to_string()),
                )
                .await;
        }

        // Remember debate outcome to agentmemory for sharing + trained intelligence
        {
            let mem = tredo_core::AgentMemoryClient::new();
            let mem_content = format!(
                "DEBATE {} {}: action={} conf={:.2} reason={}",
                symbol,
                chrono::Utc::now().date_naive(),
                debate_action,
                debate_conf,
                debate_reason
            );
            let _ = mem.remember(&mem_content, "debate_decision").await;
        }

        // Fallback or augment with LLM only if debate uncertain (keeps selective LLM usage)
        let decision = if debate_conf > 0.65 {
            // lowered threshold to use debate more (trained data bias)
            // Use debate result
            tredo_core::LlmTradeDecision {
                action: debate_action.clone(),
                entry,
                sl: if std::env::var("VALIDATION_INDUCE_REGRET")
                    .map(|v| v == "true")
                    .unwrap_or(false)
                {
                    let tight = std::env::var("VALIDATION_TIGHT_SL_PCT")
                        .ok()
                        .and_then(|s| s.parse::<f64>().ok())
                        .unwrap_or(0.5)
                        / 100.0;
                    entry * (1.0 - tight)
                } else {
                    entry * 0.99
                },
                tp: entry * 1.02,
                reason: debate_reason.clone(),
            }
        } else {
            self.state
                .llm
                .ask_for_trade_decision(
                    symbol,
                    entry,
                    confluence,
                    trend_label,
                    pivots.pivot,
                    pivots.r1,
                    pivots.s1,
                    &forecast_summary,
                    portfolio_heat,
                    session.market_open,
                    consecutive_losses,
                    &calendar_context,
                    &trading_mode,
                    &daily_goal_context,
                    &multi_tf_context,
                    &agent_market_summary,
                    &news_context,
                    &similar_episodes_context,
                    &combined_patterns_context,
                )
                .await
        };

        // Store LLM reasoning
        {
            let mut reason_store = self.state.last_llm_reason.write().await;
            *reason_store = format!(
                "[{}] {} | Mode: {} | Calendar: {} | Reason: {}",
                symbol,
                decision.action,
                trading_mode,
                if calendar_context.len() > 20 {
                    "loaded"
                } else {
                    "none"
                },
                decision.reason
            );
        }

        // === agentmemory integration: persistent memory for runtime agents ===
        // Recall past similar decisions for context, remember this one.
        // This gives the TREDO trading agents (beyond sqlite episodes) infinite long-term memory
        // shared with coding agents (Grok/Hermes via same store). Use in debate/LLM prompts.
        {
            let mem = tredo_core::AgentMemoryClient::new();
            let past = mem
                .recall(&format!("past decisions {}", symbol))
                .await
                .unwrap_or_default();
            if !past.is_empty() {
                println!(
                    "[StrategyDecision] Recalled from agentmemory for {}: {} entries",
                    symbol,
                    past.len()
                );
            }
            let mem_content = format!(
                "{}: {} | {} | Mode: {} | Reason: {}",
                symbol, decision.action, decision.reason, trading_mode, decision.reason
            );
            let _ = mem.remember(&mem_content, "trading_decision").await;
        }

        if decision.action == "HOLD" {
            println!(
                "[StrategyDecision] 🤚 LLM HOLD for {} — {} | Mode: {}",
                symbol, decision.reason, trading_mode
            );
            return Ok(None);
        }

        // ── Map LLM action to TradeDirection ───────────────────────────────────
        let trade_direction = if decision.action == "BUY" {
            tredo_core::TradeDirection::Long
        } else {
            tredo_core::TradeDirection::Short
        };

        let entry_price = decision.entry;
        let stop_loss = decision.sl;
        let take_profit = decision.tp;

        // ── Re-validate ──────────────────────────────────────────────────────
        let rules2 = self.state.rules.read().await;
        let portfolio2 = self.state.portfolio.read().await;
        let discipline = validate_trade_setup(&context, &rules2);

        // Apply goal-based risk multiplier
        let goals = self.state.trading_goals.read().await;
        let risk_mult = goals.effective_risk_multiplier();
        let adjusted_risk = if portfolio2.consecutive_losses >= 2 {
            rules2.max_risk_per_trade * 0.5 * risk_mult
        } else {
            rules2.max_risk_per_trade * risk_mult
        };
        drop(goals);

        let position_size = calculate_position_size(
            portfolio2.total_equity,
            adjusted_risk,
            entry_price,
            stop_loss,
        );

        let risk_reward =
            calculate_risk_reward(entry_price, stop_loss, take_profit, trade_direction);

        let is_crypto = matches!(symbol, "BTC" | "ETH" | "SOL");
        let session_info = if is_crypto {
            true
        } else {
            get_indian_strategy_session_info(Utc::now())
        };

        // Apply goal-based confidence threshold
        let goals2 = self.state.trading_goals.read().await;
        let min_conf = goals2.effective_min_confidence();
        drop(goals2);

        let confidence = if discipline.passed && confluence >= min_conf {
            let base = confluence;
            let rr_bonus = (risk_reward / 3.0).min(0.2);
            (base + rr_bonus).min(1.0)
        } else if discipline.passed {
            confluence * 0.8 // below goal threshold but discipline passes
        } else {
            confluence * 0.5
        };

        let signal = TradeSignal {
            symbol: symbol.to_string(),
            direction: trade_direction,
            entry_price,
            stop_loss,
            take_profit,
            position_size,
            confidence_score: confidence,
            confluence_score: confluence,
            risk_reward_ratio: risk_reward,
            reasoning: format!(
                "LLM[{}]: {} | Mode: {} | Conf: {:.1}% | R:R {:.1}:1 | Discipline: {}",
                decision.action,
                decision.reason,
                trading_mode,
                confidence * 100.0,
                risk_reward,
                if discipline.passed { "PASS" } else { "FAIL" },
            ),
            timestamp: Utc::now(),
            session_valid: session_info,
            risk_check_passed: discipline.passed,
        };

        println!(
            "[StrategyDecision] 🤖 LLM {} {} @ {:.2} | Mode: {} | Confidence: {:.1}% | {}",
            decision.action,
            symbol,
            entry_price,
            trading_mode,
            confidence * 100.0,
            decision.reason
        );

        // Store signal in history
        {
            let mut signals = self.state.last_signals.write().await;
            signals.push(signal.clone());
            if signals.len() > 100 {
                signals.remove(0);
            }
        }

        let _ = self.state.memory.store_decision(
            &format!("signal/{}/{}", symbol, Utc::now().timestamp()),
            &signal.reasoning,
        );

        Ok(Some(signal))
    }
}

/// Returns whether the market session is valid for the given time.
fn get_indian_strategy_session_info(now: chrono::DateTime<Utc>) -> bool {
    use chrono::Timelike;
    let ist = now + chrono::Duration::hours(5) + chrono::Duration::minutes(30);
    let hour = ist.hour();
    let min = ist.minute();
    let time_mins = hour * 60 + min;
    (9 * 60 + 15..=15 * 60 + 30).contains(&time_mins)
}

#[async_trait]
impl Agent for StrategyDecisionAgent {
    fn name(&self) -> &str {
        "StrategyDecisionAgent"
    }
    fn tier(&self) -> AgentTier {
        AgentTier::Main
    }

    async fn run(
        &self,
        _input: Option<AgentInput>,
    ) -> Result<AgentOutput, Box<dyn Error + Send + Sync>> {
        println!("[StrategyDecisionAgent] LLM-driven signal generation — call generate_signal() directly.");
        Ok(AgentOutput::Done)
    }
}
// Note: AgentMemoryClient integration (for long-term recall beyond local episodes) is available via tredo_core::AgentMemoryClient.
// See debate.rs and core/agentmemory.rs for usage patterns. TODOs for full remember/recall in decision paths remain for future hardening.

// Before calling LLM for debate, e.g.:
// let past = mem.recall(&format!("past decisions {}", symbol)).await.unwrap_or_default();
// include past in prompt context for the model.
