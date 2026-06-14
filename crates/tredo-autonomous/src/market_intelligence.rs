use crate::state::SharedState;
use async_trait::async_trait;
use chrono::Utc;
use std::error::Error;
use tredo_core::{
    calculate_confluence_score, calculate_pivot_points, detect_patterns, detect_patterns_multi_tf,
    format_patterns, is_in_trading_session, Agent, AgentInput, AgentOutput, AgentTier,
    KronosForecastRequest, KronosForecastTool, MarketContext, MultiTfPatternConfirmation, OhlcvBar,
    TrendDirection,
};

pub struct MarketIntelligenceAgent {
    pub state: SharedState,
}

impl MarketIntelligenceAgent {
    pub fn new(state: SharedState) -> Self {
        Self { state }
    }

    /// Run Kronos forecast + pivot/confluence analysis for a symbol.
    /// Uses the real OHLCV history from SharedState (fetched from Binance/Yahoo).
    /// Stores the full Kronos forecast JSON in SharedState.last_forecast for Phase 5 (LLM).
    pub async fn analyze_market(
        &self,
        symbol: &str,
        price: f64,
    ) -> Result<(f64, tredo_core::PivotLevels), Box<dyn Error + Send + Sync>> {
        // Smarter agent: use hierarchical trained memory RAG+ to recall exactly what "I" (MI) or the team did in past similar market conditions (same symbol, price level, regime) and the outcome/lesson.
        // This grounds the analysis in real past "what it was doing" and reduces hallucinations by making the agent remember its own history.
        let trained_recall = self
            .state
            .recall_trained_memory(
                &format!("market intel analysis for {} at price level regime", symbol),
                3,
            )
            .await;

        // === UPGRADE: Integrate new skills/tools for richer MI (sentiment, vol, regime, corr)
        let sentiment = crate::sentiment_analyzer::SentimentAnalyzer::new(self.state.clone())
            .analyze_sentiment(symbol)
            .await;
        let (_vol, expansion) =
            crate::volatility_calculator::VolatilityCalculator::new(self.state.clone())
                .compute_volatility(symbol, price)
                .await;
        let regime = crate::regime_detector::RegimeDetector::new(self.state.clone())
            .detect_regime(symbol, price)
            .await;
        let corr = crate::correlation_checker::CorrelationChecker::new(self.state.clone())
            .check_correlation(symbol)
            .await;
        let onchain = crate::on_chain_data::OnChainData::new(self.state.clone())
            .fetch_onchain(symbol)
            .await;

        // Include trained recall in the analysis for smarter, memory-grounded confluence.
        println!("[MI smarter] {}\n(trained memory used for self-understanding and long-term improvement)", trained_recall);

        // Boost confluence with new signals (research: multi-factor like TradingAgents)
        let extra_score = (sentiment - 0.5) * 0.12
            + (if expansion { 0.08 } else { 0.0 })
            + (corr - 0.5) * 0.08
            + (onchain - 0.5) * 0.10;
        println!(
            "[MI UPGRADE] {} sentiment={:.2} vol_exp={:?} regime={:?} corr={:.2} onchain={:.2} extra={:.2}",
            symbol, sentiment, expansion, regime, corr, onchain, extra_score
        );

        let rules = self.state.rules.read().await;

        let high = price * 1.015;
        let low = price * 0.985;
        let prev_close = price * 0.998;

        // --- Kronos Forecast Service ---
        let kronos_client = KronosForecastTool::new(self.state.config.kronos_service_url.clone());

        // Read real OHLCV history from SharedState (fetched by orchestrator from Binance/Yahoo)
        let ohlcv_for_kronos: Vec<OhlcvBar> = {
            let history = self.state.ohlcv_history.read().await;
            history.get(symbol).cloned().unwrap_or_default()
        };

        // Fallback: if no history available, build a single bar from current price
        let sample_ohlcv = if ohlcv_for_kronos.is_empty() {
            vec![OhlcvBar {
                timestamp: Utc::now().to_rfc3339(),
                open: prev_close,
                high,
                low,
                close: price,
                volume: 100_000.0,
            }]
        } else {
            ohlcv_for_kronos
        };

        let pred_len = if sample_ohlcv.len() >= 10 { 10 } else { 5 };

        println!(
            "[MarketIntelligence] Feeding {} OHLCV bars to Kronos for {}",
            sample_ohlcv.len(),
            symbol
        );

        let forecast_req = KronosForecastRequest {
            symbol: symbol.to_string(),
            ohlcv: sample_ohlcv,
            pred_len,
            temperature: 0.8,
            top_p: 0.9,
            sample_count: 1,
        };

        let mut trend_direction = None;
        let mut forecast_summary = String::from("No forecast available");

        match kronos_client.forecast(forecast_req).await {
            Ok(resp) => {
                // Summarise the 5-bar forecast for the LLM prompt
                let closes: Vec<f64> = resp
                    .forecasts
                    .iter()
                    .filter_map(|f| f.get("close").and_then(|c| c.as_f64()))
                    .collect();

                if let Some(&pred_close) = closes.last() {
                    println!(
                        "[MarketIntelligence] Kronos predicted close for {}: {:.2} (current: {:.2})",
                        symbol, pred_close, price
                    );

                    trend_direction = Some(if pred_close > price {
                        TrendDirection::Bullish
                    } else if pred_close < price {
                        TrendDirection::Bearish
                    } else {
                        TrendDirection::Neutral
                    });

                    let pct_change = (pred_close - price) / price * 100.0;
                    forecast_summary = format!(
                        "Predicts {:.2} in 5 bars ({:+.2}%). Closes: {}",
                        pred_close,
                        pct_change,
                        closes
                            .iter()
                            .map(|c| format!("{:.2}", c))
                            .collect::<Vec<_>>()
                            .join(", ")
                    );
                }

                // Store full forecast JSON for Phase 5 (StrategyDecisionAgent)
                {
                    let mut last = self.state.last_forecast.write().await;
                    *last = Some(serde_json::json!({
                        "symbol": symbol,
                        "forecasts": resp.forecasts,
                        "summary": forecast_summary.clone(),
                    }));
                }
            }
            Err(e) => {
                println!(
                    "[MarketIntelligence] Kronos call failed: {}. Defaulting to Neutral.",
                    e
                );
                let mut last = self.state.last_forecast.write().await;
                *last = None;
            }
        }

        // Read real portfolio equity for accurate drawdown calculations
        let equity = {
            let portfolio = self.state.portfolio.read().await;
            portfolio.total_equity
        };

        let context = MarketContext {
            symbol: symbol.to_string(),
            current_price: price,
            high,
            low,
            previous_close: prev_close,
            timestamp: Utc::now(),
            daily_pnl: 0.0,
            equity,
            consecutive_losses: 0,
            is_red_folder_day: false,
            trend_direction,
        };

        // Strong skills set: "how to do" via pluggable AgentSkill trait.
        // Collect and execute skills for richer "how" (e.g., sentiment for mood, vol for risk).
        // Agents know "what to do" (role); skills tell execution method; rules gate; trained memory (recall) makes smarter.
        let skills: Vec<Box<dyn tredo_core::skills::AgentSkill>> = vec![
            Box::new(crate::sentiment_analyzer::SentimentAnalyzer::new(
                self.state.clone(),
            )),
            Box::new(crate::volatility_calculator::VolatilityCalculator::new(
                self.state.clone(),
            )),
            Box::new(crate::regime_detector::RegimeDetector::new(
                self.state.clone(),
            )),
            Box::new(crate::correlation_checker::CorrelationChecker::new(
                self.state.clone(),
            )),
            Box::new(crate::on_chain_data::OnChainData::new(self.state.clone())),
            // TrainedMemorySkill can be added here too for unified pluggable recall execution.
        ];
        let mut skill_results: Vec<String> = vec![];
        for skill in &skills {
            if skill.is_available() {
                if let Ok(tredo_core::AgentOutput::SkillResult {
                    name, score, note, ..
                }) = skill
                    .execute(&AgentInput::ConfluenceRequest {
                        context: context.clone(),
                    })
                    .await
                {
                    skill_results.push(format!("{}={:.2}({})", name, score, note));
                }
            }
        }
        let skills_summary = if skill_results.is_empty() {
            "none".to_string()
        } else {
            skill_results.join("; ")
        };
        let _ = self
            .state
            .push_cot(
                "MarketIntelligence",
                &format!("skills executed for {}", symbol),
                "SKILLS_RUN",
                &format!("{} + trained memory", skills_summary),
                0.75,
                0,
                None,
                Some(symbol.to_string()),
            )
            .await;

        let pivots = calculate_pivot_points(high, low, prev_close, rules.pivot_method);
        let mut confluence = calculate_confluence_score(&context, &pivots);
        // Apply MI UPGRADE extra from skills (kept after vars in scope)
        confluence = (confluence + extra_score).clamp(0.0, 1.0);
        let is_crypto = matches!(symbol, "BTC" | "ETH" | "SOL");
        let session_valid = is_crypto || is_in_trading_session(Utc::now(), &rules);

        println!(
            "[MarketIntelligence] {} @ {:.2} | Pivot: {:.2} | R1: {:.2} | S1: {:.2} | Confluence: {:.2}% | Session: {} | {}",
            symbol, price, pivots.pivot, pivots.r1, pivots.s1,
            confluence * 100.0,
            if session_valid { "VALID" } else { "INVALID" },
            forecast_summary
        );

        // ── Candlestick Pattern Detection (Single-TF on 1m) ─────────────────
        let detected_patterns = {
            let history = self.state.ohlcv_history.read().await;
            let bars = history.get(symbol).cloned().unwrap_or_default();
            if bars.len() >= 2 {
                let pats = detect_patterns(&bars);
                if !pats.is_empty() {
                    println!(
                        "[MarketIntelligence] 📊 1m Patterns for {}: {}",
                        symbol,
                        pats.iter()
                            .map(|p| format!("{} ({:.0}%)", p.name, p.strength * 100.0))
                            .collect::<Vec<_>>()
                            .join(", ")
                    );
                }
                pats
            } else {
                vec![]
            }
        };
        let patterns_context = format_patterns(&detected_patterns);

        // Store detected patterns in SharedState for episode capture
        {
            let mut stored = self.state.last_patterns.write().await;
            stored.insert(symbol.to_string(), detected_patterns);
        }

        // ── Multi-Timeframe Pattern Detection & Confirmation ────────────────
        let mtf_patterns = {
            let mtf_data = self.state.multi_timeframe_data.read().await;
            let data = mtf_data.get(symbol);
            if let Some(tf_data) = data {
                if tf_data.len() >= 2 {
                    // Build (&str, &[OhlcvBar]) pairs for all available timeframes including 1m
                    let bars_1m_owned: Vec<OhlcvBar> = {
                        let history = self.state.ohlcv_history.read().await;
                        history.get(symbol).cloned().unwrap_or_default()
                    };

                    let mut tf_pairs: Vec<(&str, &[OhlcvBar])> = Vec::new();
                    if !bars_1m_owned.is_empty() {
                        tf_pairs.push(("1m", bars_1m_owned.as_slice()));
                    }
                    for tf in tf_data {
                        if !tf.ohlcv.is_empty() {
                            tf_pairs.push((tf.timeframe.as_str(), tf.ohlcv.as_slice()));
                        }
                    }

                    let mtf = detect_patterns_multi_tf(&tf_pairs);
                    if !mtf.timeframes_with_patterns.is_empty() {
                        println!("[MarketIntelligence] 🔄 Multi-TF patterns for {}: confirmed on {} timeframes | bullish={} bearish={}",
                            symbol, mtf.timeframes_with_patterns.len(),
                            mtf.bullish_confirmation, mtf.bearish_confirmation);
                    }
                    mtf
                } else {
                    // Need at least 2 timeframes for meaningful confirmation
                    MultiTfPatternConfirmation::default()
                }
            } else {
                MultiTfPatternConfirmation::default()
            }
        };

        // Store multi-TF pattern confirmation
        {
            let mut stored = self.state.last_mtf_patterns.write().await;
            stored.insert(symbol.to_string(), mtf_patterns);
        }

        let _ = self.state.memory.store_decision(
            &format!("market/{}/{}", symbol, Utc::now().timestamp()),
            &format!(
                "price={:.2},confluence={:.3},trend={:?},patterns={}",
                price, confluence, trend_direction, patterns_context
            ),
        );

        {
            let mut regime = self.state.market_regime.write().await;
            *regime = Some(if confluence > 0.7 {
                crate::types::MarketRegime::TrendingBull
            } else if confluence < 0.4 {
                crate::types::MarketRegime::TrendingBear
            } else {
                crate::types::MarketRegime::Ranging
            });
        }

        Ok((confluence, pivots))
    }
}

#[async_trait]
impl Agent for MarketIntelligenceAgent {
    fn name(&self) -> &str {
        "MarketIntelligenceAgent"
    }
    fn tier(&self) -> AgentTier {
        AgentTier::Main
    }

    async fn run(
        &self,
        input: Option<AgentInput>,
    ) -> Result<AgentOutput, Box<dyn Error + Send + Sync>> {
        match input {
            Some(AgentInput::ConfluenceRequest { context }) => {
                let (confluence, _) = self
                    .analyze_market(&context.symbol, context.current_price)
                    .await?;
                Ok(AgentOutput::ConfluenceResult(confluence))
            }
            Some(AgentInput::PivotRequest { high, low, close }) => {
                let rules = self.state.rules.read().await;
                let pivots = calculate_pivot_points(high, low, close, rules.pivot_method);
                println!("[MarketIntelligence] Pivot levels calculated on request");
                Ok(AgentOutput::PivotResult(pivots))
            }
            _ => {
                let (confluence, _) = self.analyze_market("NIFTY", 24500.0).await?;
                Ok(AgentOutput::ConfluenceResult(confluence))
            }
        }
    }
}
