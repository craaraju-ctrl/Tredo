// OnChainData tool/skill for crypto on-chain metrics.
// Uses data already available in state (OHLCV history, prices) to compute on-chain signals.
// This is an alternative to expensive on-chain APIs - uses price/volume as proxy for on-chain activity.

use crate::state::SharedState;
use async_trait::async_trait;
use std::error::Error;
use tredo_core::{skills::AgentSkill, AgentInput, AgentOutput};

pub struct OnChainData {
    pub state: SharedState,
}

impl OnChainData {
    pub fn new(state: SharedState) -> Self {
        Self { state }
    }

    /// Computes on-chain score from available market data.
    /// Returns: >0.5 = bullish (accumulation), <0.5 = bearish (distribution)
    ///
    /// Uses multiple signals:
    /// - Volume trend (high volume + price rise = accumulation)
    /// - Price momentum
    /// - Volatility contraction (smart money accumulating quietly)
    pub async fn fetch_onchain(&self, symbol: &str) -> f64 {
        let history = self.state.ohlcv_history.read().await;

        if let Some(bars) = history.get(symbol) {
            if bars.len() < 20 {
                return 0.5;
            }

            // Calculate multiple signals
            let volume_signal = self.calculate_volume_signal(bars);
            let price_signal = self.calculate_price_signal(bars);
            let volatility_signal = self.calculate_volatility_signal(bars);

            // Weighted combination:
            // Volume is most important for on-chain (traders moving money)
            // Price confirms direction
            // Volatility contraction often precedes big moves (smart money accumulating)
            let score = (volume_signal * 0.4) + (price_signal * 0.35) + (volatility_signal * 0.25);

            eprintln!(
                "[OnChain] {}: vol={:.2}, price={:.2}, vol_sig={:.2} -> score={:.2}",
                symbol, volume_signal, price_signal, volatility_signal, score
            );

            return score.clamp(0.0, 1.0);
        }

        // No history available
        0.5 // neutral when no data
    }

    fn calculate_volume_signal(&self, bars: &[tredo_core::OhlcvBar]) -> f64 {
        if bars.len() < 20 {
            return 0.5;
        }

        let recent_volumes: Vec<f64> = bars.iter().map(|b| b.volume).collect();
        let avg_volume = recent_volumes.iter().sum::<f64>() / recent_volumes.len() as f64;

        // Last 5 days volume vs average
        let recent_avg: f64 = recent_volumes.iter().rev().take(5).sum::<f64>() / 5.0;
        let volume_ratio = recent_avg / avg_volume;

        // Also check price action with volume
        let last_5_prices: Vec<f64> = bars.iter().rev().take(5).map(|b| b.close).collect();
        let price_change = if last_5_prices.len() >= 2 {
            (last_5_prices[0] - last_5_prices[last_5_prices.len() - 1])
                / last_5_prices[last_5_prices.len() - 1]
        } else {
            0.0
        };

        // High volume + price rise = accumulation (bullish)
        // High volume + price fall = distribution (bearish)
        // Low volume = smart money accumulating quietly (slightly bullish)

        let vol_score = if volume_ratio > 1.2 {
            // High volume - check direction
            if price_change > 0.01 {
                0.7
            }
            // High volume + up = strong accumulation
            else if price_change < -0.01 {
                0.3
            }
            // High volume + down = distribution
            else {
                0.5
            }
        } else if volume_ratio < 0.8 {
            // Low volume - potential accumulation (smart money)
            if price_change > 0.0 {
                0.6
            } else {
                0.4
            }
        } else {
            // Normal volume
            0.5 + (price_change * 5.0)
        };

        vol_score.clamp(0.0, 1.0)
    }

    fn calculate_price_signal(&self, bars: &[tredo_core::OhlcvBar]) -> f64 {
        if bars.len() < 10 {
            return 0.5;
        }

        let prices: Vec<f64> = bars.iter().map(|b| b.close).collect();

        // Multiple timeframe analysis
        let _recent_5 = prices.iter().rev().take(5);
        let _recent_10 = prices.iter().rev().take(10);
        let _recent_20 = prices.iter().rev().take(20);

        let change_5 =
            (prices[0] - prices[4.min(prices.len() - 1)]) / prices[4.min(prices.len() - 1)];
        let change_10 =
            (prices[0] - prices[9.min(prices.len() - 1)]) / prices[9.min(prices.len() - 1)];

        // Trend alignment - if all timeframes agree, strong signal
        let mut score: f64 = 0.5;

        if change_5 > 0.02 {
            score += 0.15;
        } else if change_5 < -0.02 {
            score -= 0.15;
        }

        if change_10 > 0.05 {
            score += 0.2;
        } else if change_10 < -0.05 {
            score -= 0.2;
        }

        // Check if price is above moving averages
        let ma5 = prices.iter().rev().take(5).sum::<f64>() / 5.0;
        let ma10 = prices.iter().rev().take(10).sum::<f64>() / 10.0;

        if prices[0] > ma5 {
            score += 0.1;
        }
        if prices[0] > ma10 {
            score += 0.1;
        }

        score.clamp(0.0, 1.0)
    }

    fn calculate_volatility_signal(&self, bars: &[tredo_core::OhlcvBar]) -> f64 {
        if bars.len() < 20 {
            return 0.5;
        }

        // Volatility contraction often precedes big moves (smart money accumulating)
        let recent: Vec<f64> = bars.iter().rev().take(10).map(|b| b.high - b.low).collect();
        let older: Vec<f64> = bars
            .iter()
            .rev()
            .skip(10)
            .take(10)
            .map(|b| b.high - b.low)
            .collect();

        let recent_vol = recent.iter().sum::<f64>() / recent.len() as f64;
        let older_vol = older.iter().sum::<f64>() / older.len() as f64;

        if older_vol > 0.0 {
            let vol_ratio = recent_vol / older_vol;

            // Low volatility (contraction) = potential accumulation
            // High volatility = uncertainty/distribution
            if vol_ratio < 0.6 {
                return 0.65; // Contraction - potential smart money accumulation
            } else if vol_ratio > 1.4 {
                return 0.4; // Expansion - high uncertainty
            }
        }

        0.5 // Neutral
    }
}

#[async_trait]
impl AgentSkill for OnChainData {
    fn name(&self) -> &str {
        "OnChainData"
    }
    fn description(&self) -> &str {
        "Proxy on-chain / accumulation score from volume + price action + vol contraction (how to read 'smart money' flow for crypto without paid APIs)."
    }

    async fn execute(
        &self,
        input: &AgentInput,
    ) -> Result<AgentOutput, Box<dyn Error + Send + Sync>> {
        if let AgentInput::ConfluenceRequest { context } = input {
            let score = self.fetch_onchain(&context.symbol).await;
            println!(
                "[Skill] {} executed for {}: onchain_score={:.2}",
                self.name(),
                context.symbol,
                score
            );
            Ok(AgentOutput::SkillResult {
                name: self.name().to_string(),
                score,
                note: "volume+price+vol proxy for accumulation".to_string(),
                confidence: 0.7,
            })
        } else {
            Ok(AgentOutput::Done)
        }
    }
}
