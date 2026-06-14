// outcome_processor.rs
// OutcomeProcessor — calculates regret score, generates lesson, and persists
// a completed trade episode to the SQLite EpisodeStore.
//
// Called by ExecutionCoordinatorAgent when a position closes (SL/TP hit).
// This is the critical missing piece that closes the feedback loop.

use crate::episode_store::{ClosedEpisode, RegretEvent};
use crate::state::SharedState;
use crate::types::OpenPosition;
use chrono::Utc;
use uuid::Uuid;

pub struct OutcomeProcessor {
    pub state: SharedState,
}

impl OutcomeProcessor {
    pub fn new(state: SharedState) -> Self {
        Self { state }
    }

    /// Called when a position closes. Scores regret, stores episode, and if
    /// regret >= 0.5 also inserts a `regret_event` for MetaControl to review.
    pub async fn close_episode(
        &self,
        pos: &OpenPosition,
        exit_price: f64,
        exit_reason: &str, // "stop_loss" | "take_profit" | "manual"
        pnl: f64,
    ) {
        let portfolio = self.state.portfolio.read().await;
        let portfolio_heat: f64 = {
            let total_risk: f64 = portfolio.open_positions.iter().map(|p| p.risk_amount).sum();
            if portfolio.total_equity > 0.0 {
                total_risk / portfolio.total_equity
            } else {
                0.0
            }
        };
        let consecutive_losses = portfolio.consecutive_losses;
        drop(portfolio);

        let pnl_pct = if pos.entry_price > 0.0 {
            pnl / (pos.entry_price * pos.quantity)
        } else {
            0.0
        };

        let outcome = if pnl > 5.0 {
            "WIN"
        } else if pnl < -5.0 {
            "LOSS"
        } else {
            "BREAKEVEN"
        };

        // ── Market context at exit ────────────────────────────────────────
        let market_regime = {
            let r = self.state.market_regime.read().await;
            match *r {
                Some(crate::types::MarketRegime::TrendingBull) => "TrendingBull",
                Some(crate::types::MarketRegime::TrendingBear) => "TrendingBear",
                Some(crate::types::MarketRegime::Ranging) => "Ranging",
                Some(crate::types::MarketRegime::Volatile) => "Volatile",
                _ => "Unknown",
            }
            .to_string()
        };

        let agent_reasoning = self.state.last_llm_reason.read().await.clone();

        // ── Retrieve confluence from the original signal (if stored) ──────
        let confluence_score = {
            let signals = self.state.last_signals.read().await;
            signals
                .iter()
                .rfind(|s| s.symbol == pos.symbol)
                .map(|s| s.confluence_score)
                .unwrap_or(0.0)
        };

        // ── Regret scoring (deterministic, no LLM call needed) ────────────
        let (regret_score, lesson, rule_violated) = self
            .score_regret(
                outcome,
                confluence_score,
                consecutive_losses,
                portfolio_heat,
                pnl_pct,
            )
            .await;

        let session = {
            use crate::helpers::get_indian_session_info;
            let info = get_indian_session_info(Utc::now());
            if info.market_open {
                info.session_name
            } else {
                "OffHours".to_string()
            }
        };

        let episode = ClosedEpisode {
            id: Uuid::new_v4().to_string(),
            symbol: pos.symbol.clone(),
            direction: format!("{:?}", pos.direction),
            entry_price: pos.entry_price,
            exit_price,
            stop_loss: pos.stop_loss,
            take_profit: pos.take_profit,
            position_size: pos.quantity,
            pnl,
            pnl_pct,
            outcome: outcome.to_string(),
            exit_reason: exit_reason.to_string(),
            regret_score,
            lesson: lesson.clone(),
            confluence_score,
            portfolio_heat,
            market_regime: market_regime.clone(),
            session,
            agent_reasoning,
            consecutive_losses_at_entry: consecutive_losses,
            entry_time: pos.entry_time.to_rfc3339(),
            exit_time: Utc::now().to_rfc3339(),
        };

        // ── Persist to SQLite ─────────────────────────────────────────────
        let store = &self.state.episode_store;
        match store.insert_closed_trade(&episode) {
            Ok(_) => println!(
                "[OutcomeProcessor] 💾 Episode stored: {} {} {} | P&L: ₹{:.2} | Regret: {:.2}",
                episode.symbol, episode.outcome, episode.exit_reason, pnl, regret_score
            ),
            Err(e) => eprintln!("[OutcomeProcessor] ⚠ Failed to store episode: {}", e),
        }

        // ── High-regret → regret_events table ─────────────────────────────
        if regret_score >= 0.5 {
            let ev = RegretEvent {
                episode_id: episode.id.clone(),
                symbol: episode.symbol.clone(),
                regret_score,
                lesson: lesson.clone(),
                rule_violated,
                recorded_at: Utc::now().to_rfc3339(),
            };
            if let Err(e) = store.insert_regret_event(&ev) {
                eprintln!("[OutcomeProcessor] ⚠ Failed to store regret event: {}", e);
            } else {
                println!(
                    "[OutcomeProcessor] ⚠ HIGH REGRET event logged (score: {:.2}) — {}",
                    regret_score, lesson
                );
            }
        }

        // ── Auto-trigger MetaControl if 3+ bad trades today ───────────────
        let bad_today = store.count_regret_events_today();
        if bad_today >= 3 {
            println!(
                "[OutcomeProcessor] 🚨 {} high-regret trades today — triggering MetaControl review",
                bad_today
            );
            let state_clone = self.state.clone();
            tokio::spawn(async move {
                let mc = crate::meta_control::MetaControlAgent::new(state_clone);
                match mc.weekly_review(1).await {
                    Ok(report) => println!(
                        "[OutcomeProcessor] 🧠 Emergency MetaControl: {} changes applied",
                        report.changes_applied as u8
                    ),
                    Err(e) => eprintln!("[OutcomeProcessor] MetaControl error: {}", e),
                }
            });
        }
    }

    /// Pure regret scoring — no LLM, deterministic, zero latency.
    ///
    /// Returns (regret_score, lesson, rule_violated)
    async fn score_regret(
        &self,
        outcome: &str,
        confluence: f64,
        consecutive_losses: u32,
        portfolio_heat: f64,
        pnl_pct: f64,
    ) -> (f64, String, String) {
        let rules = self.state.rules.read().await;
        let mut score = 0.0_f64;
        let mut lessons: Vec<&str> = Vec::new();
        let mut rule_violated = String::new();

        if outcome == "LOSS" {
            // Low confluence entry
            if confluence < rules.min_confluence_score {
                score += 0.3;
                lessons.push("Entered with insufficient confluence");
                rule_violated = "min_confluence_score".to_string();
            }

            // Was already in a loss streak — overtrading
            if consecutive_losses >= 2 {
                score += 0.25;
                lessons.push("Traded while on a losing streak");
                if rule_violated.is_empty() {
                    rule_violated = "max_consecutive_losses".to_string();
                }
            }

            // Portfolio heat already high — ignored risk rules
            if portfolio_heat > 0.30 {
                score += 0.25;
                lessons.push("Portfolio heat was too high at entry");
                if rule_violated.is_empty() {
                    rule_violated = "portfolio_heat_limit".to_string();
                }
            }

            // Large loss — poor risk sizing or SL was too wide
            if pnl_pct < -0.025 {
                score += 0.20;
                lessons.push("Loss exceeded expected risk — SL may have been too wide");
                if rule_violated.is_empty() {
                    rule_violated = "max_risk_per_trade".to_string();
                }
            }
        } else if outcome == "WIN" {
            // Won despite low confidence — lucky, not skilled
            if confluence < 0.55 {
                score += 0.15;
                lessons.push("Won despite low confluence — may have been lucky");
            }

            // Premature exit (left money on table)
            if pnl_pct > 0.0 && pnl_pct < 0.005 {
                score += 0.10;
                lessons.push("Exited too early — left profit on the table");
            }
        }

        score = score.min(1.0);
        let lesson = if lessons.is_empty() {
            "Good trade — discipline maintained".to_string()
        } else {
            lessons.join("; ")
        };

        (score, lesson, rule_violated)
    }
}
