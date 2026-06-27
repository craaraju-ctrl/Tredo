use tredo_autonomous::state::SharedState;
use tredo_core::TradeDirection;

pub enum Decision {
    Approve { reason: String },
    Caution { reason: String },
    Reject { reason: String },
}

pub struct PortfolioReasoner {
    state: SharedState,
    #[allow(dead_code)]
    max_sector_exposure: f64,
    #[allow(dead_code)]
    max_correlation: f64,
}

impl PortfolioReasoner {
    pub fn new(state: SharedState) -> Self {
        Self {
            state,
            max_sector_exposure: 0.30,
            max_correlation: 0.85,
        }
    }

    pub async fn should_open_new(&self, symbol: &str, direction: TradeDirection) -> Decision {
        let portfolio = self.state.portfolio.read().await;
        let equity = portfolio.total_equity;
        if equity <= 0.0 {
            return Decision::Reject {
                reason: "Zero equity".to_string(),
            };
        }
        let current_heat: f64 = portfolio
            .open_positions
            .iter()
            .map(|p| p.risk_amount)
            .sum::<f64>()
            / equity;
        if current_heat > 0.15 {
            return Decision::Reject {
                reason: format!("Portfolio heat at {:.1}%", current_heat * 100.0),
            };
        }

        // Check sector exposure
        let sector = Self::get_sector(symbol);
        let sector_exposure: f64 = portfolio
            .open_positions
            .iter()
            .filter(|p| Self::get_sector(&p.symbol) == sector)
            .map(|p| p.quantity * p.current_price)
            .sum::<f64>()
            / equity;
        if sector_exposure > self.max_sector_exposure {
            return Decision::Reject {
                reason: format!(
                    "{} sector exposure at {:.1}%",
                    sector,
                    sector_exposure * 100.0
                ),
            };
        }

        // Check correlation with existing positions
        for pos in portfolio.open_positions.iter() {
            let corr = Self::estimate_correlation(symbol, &pos.symbol);
            if corr.abs() > self.max_correlation
                && Self::direction_matches(&direction, &pos.direction)
            {
                return Decision::Reject {
                    reason: format!(
                        "High correlation ({:.2}) with existing {}",
                        corr, pos.symbol
                    ),
                };
            }
        }

        drop(portfolio);
        Decision::Approve {
            reason: "Portfolio composition allows this trade".to_string(),
        }
    }

    fn get_sector(symbol: &str) -> &'static str {
        match symbol {
            "BTC" | "ETH" | "SOL" | "BNB" | "XRP" | "ADA" | "DOGE" | "AVAX" => "crypto_l1",
            "MATIC" | "ARB" | "OP" => "crypto_l2",
            "LINK" | "UNI" | "AAVE" => "defi",
            _ => "other",
        }
    }

    fn estimate_correlation(_sym1: &str, _sym2: &str) -> f64 {
        // Placeholder: would use recent price history for rolling correlation
        // For now, assume no correlation to avoid blocking trades
        0.0
    }

    fn direction_matches(a: &TradeDirection, b: &TradeDirection) -> bool {
        matches!(
            (a, b),
            (TradeDirection::Long, TradeDirection::Long)
                | (TradeDirection::Short, TradeDirection::Short)
        )
    }

    /// Compute how much adding this symbol would improve portfolio diversification.
    /// Returns 0.0 to 1.0 where positive = beneficial diversification.
    pub async fn compute_diversification_benefit(&self, symbol: &str) -> f64 {
        let portfolio = self.state.portfolio.read().await;
        if portfolio.open_positions.is_empty() {
            return 1.0; // First position is always diversifying
        }
        let mut correlations = Vec::new();
        for pos in portfolio.open_positions.iter() {
            let corr = Self::estimate_correlation(symbol, &pos.symbol);
            correlations.push(corr.abs());
        }
        drop(portfolio);
        let avg_corr = if correlations.is_empty() {
            0.0
        } else {
            correlations.iter().sum::<f64>() / correlations.len() as f64
        };
        // Lower average correlation = higher diversification benefit
        (1.0 - avg_corr).clamp(0.0, 1.0)
    }
}
