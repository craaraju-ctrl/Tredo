use chrono::Utc;
use std::error::Error;
use tredo_core::{Agent, PivotLevels};

// NOTE: These phase methods are preserved for backward API compatibility.
// The pipeline now routes through Tredo groups (see tredo.rs) instead.
#[allow(dead_code)]
impl crate::orchestrator_struct::AutonomousOrchestrator {
    pub async fn phase1_discipline_checks(&self) -> Result<bool, Box<dyn Error + Send + Sync>> {
        println!("\n[PHASE 1] Discipline Checks");
        // Call sub-agents
        let session_ok = self.session_timer.run(None).await?;
        let drawdown_ok = self.drawdown.run(None).await?;
        let red_ok = self.red_folder.run(None).await?;
        let over_ok = self.overtrading.run(None).await?;

        println!(
            "[PHASE 1] Session: {} | Drawdown: {} | RedFolder: {} | Overtrading: {}",
            if session_ok.is_ok() { "OK" } else { "FAIL" },
            if drawdown_ok.is_ok() { "OK" } else { "FAIL" },
            if red_ok.is_ok() { "OK" } else { "FAIL" },
            if over_ok.is_ok() { "OK" } else { "FAIL" }
        );

        Ok(session_ok.is_ok() && drawdown_ok.is_ok() && red_ok.is_ok() && over_ok.is_ok())
    }

    pub async fn phase2_market_analysis(
        &self,
        symbol: &str,
        price: f64,
    ) -> Result<(f64, PivotLevels), Box<dyn Error + Send + Sync>> {
        println!("\n[PHASE 2] Market Analysis for {}", symbol);
        let (confluence, pivots) = self.market_intel.analyze_market(symbol, price).await?;
        let _ = self
            .pivot_calc
            .run(Some(tredo_core::AgentInput::PivotRequest {
                high: price * 1.01,
                low: price * 0.99,
                close: price,
            }))
            .await;
        Ok((confluence, pivots))
    }

    pub async fn phase3_risk_assessment(
        &self,
        symbol: &str,
        price: f64,
    ) -> Result<crate::types::RiskAnalysis, Box<dyn Error + Send + Sync>> {
        println!("\n[PHASE 3] Risk Assessment");
        let equity = {
            let portfolio = self.state.portfolio.read().await;
            portfolio.total_equity
        };
        let analysis = self
            .risk_psych
            .analyze_risk(&tredo_core::MarketContext {
                symbol: symbol.to_string(),
                current_price: price,
                high: price * 1.01,
                low: price * 0.99,
                previous_close: price * 0.998,
                timestamp: Utc::now(),
                daily_pnl: 0.0,
                equity,
                consecutive_losses: 0,
                is_red_folder_day: false,
                trend_direction: None,
            })
            .await?;
        Ok(analysis)
    }

    pub async fn phase4_reflection(
        &self,
        symbol: &str,
    ) -> Result<String, Box<dyn Error + Send + Sync>> {
        println!("\n[PHASE 4] Reflection");
        let reflection = self.reflector.reflect(symbol).await?;
        Ok(reflection)
    }
}
