// OnChainData tool/skill for crypto on-chain metrics.
// Tries free blockchain APIs first (Blockchain.com, CoinGecko), falls back to local volume/price proxy.
// Real APIs provide: mempool size, hashrate, exchange volume — genuine on-chain signals.

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

    /// Fetches on-chain signals from free blockchain APIs + local proxy.
    /// Returns: >0.5 = bullish (accumulation), <0.5 = bearish (distribution)
    ///
    /// Tier 1: Free APIs (no key required)
    ///     - Blockchain.com: BTC mempool size, hashrate, unconfirmed txs
    ///     - CoinGecko: market cap, volume, 24h change (keyless)
    /// Tier 2: Local proxy (always available)
    ///     - Volume trend, price momentum, volatility contraction
    pub async fn fetch_onchain(&self, symbol: &str) -> f64 {
        let mut api_score: Option<f64> = None;
        let mut api_note = String::new();

        // === Tier 1: Free blockchain APIs ===
        if symbol == "BTC" {
            if let Ok(score) = self.fetch_blockchain_com_signals().await {
                api_score = Some(score);
                api_note = "blockchain.com".to_string();
                println!(
                    "[OnChain] {} API score from blockchain.com: {:.2}",
                    symbol, score
                );
            }
        }
        // CoinGecko works for BTC/ETH/SOL (keyless)
        if api_score.is_none() {
            if let Ok((score, note)) = self.fetch_coingecko_onchain(symbol).await {
                api_score = Some(score);
                api_note = note.clone();
                println!("[OnChain] {} API score from {}: {:.2}", symbol, note, score);
            } else {
                println!(
                    "[OnChain] {} CoinGecko API call failed — falling back to local proxy",
                    symbol
                );
            }
        }

        // === Tier 2: Local proxy (always available) ===
        let history = self.state.ohlcv_history.read().await;
        let proxy_score = if let Some(bars) = history.get(symbol) {
            if bars.len() < 20 {
                0.5
            } else {
                let volume_signal = self.calculate_volume_signal(bars);
                let price_signal = self.calculate_price_signal(bars);
                let volatility_signal = self.calculate_volatility_signal(bars);
                (volume_signal * 0.4) + (price_signal * 0.35) + (volatility_signal * 0.25)
            }
        } else {
            0.5
        };

        // Blend: 60% real API data (if available) + 40% local proxy
        let score = if let Some(api) = api_score {
            let blended = api * 0.6 + proxy_score * 0.4;
            println!(
                "[OnChain] {}: API({})={:.2} + proxy={:.2} -> blended={:.2}",
                symbol, api_note, api, proxy_score, blended
            );
            blended
        } else {
            println!(
                "[OnChain] {}: proxy-only vol={:.2} (no API data)",
                symbol, proxy_score
            );
            proxy_score
        };

        score.clamp(0.0, 1.0)
    }

    /// Fetch BTC mempool size + hashrate from Blockchain.com (free, no key).
    /// Higher mempool = more pending transactions = more network activity = bullish signal.
    /// Higher hashrate = more miner confidence = bullish signal.
    async fn fetch_blockchain_com_signals(&self) -> Result<f64, Box<dyn Error + Send + Sync>> {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(5))
            .build()?;

        // Mempool size (unconfirmed transactions)
        let mempool_size: u64 = client
            .get("https://blockchain.info/q/unconfirmedcount")
            .timeout(std::time::Duration::from_secs(5))
            .send()
            .await?
            .text()
            .await?
            .trim()
            .parse()?;

        // Hashrate (TH/s)
        let hashrate_str = client
            .get("https://blockchain.info/q/hashrate")
            .timeout(std::time::Duration::from_secs(5))
            .send()
            .await?
            .text()
            .await?;
        let hashrate: f64 = hashrate_str.trim().parse().unwrap_or(0.0);

        println!(
            "[OnChain/BTC] mempool={} txs, hashrate={:.0} TH/s",
            mempool_size, hashrate
        );

        // Score from mempool activity: high mempool = bullish (network usage up)
        // Typical mempool: 10k-100k. Above 50k is high activity.
        let mempool_score = if mempool_size > 80_000 {
            0.7 // Very high activity — bullish
        } else if mempool_size > 40_000 {
            0.6 // High activity
        } else if mempool_size < 5_000 {
            0.4 // Very low activity — bearish
        } else {
            0.5 // Normal
        };

        // Hashrate: higher = more miner confidence = bullish
        // Score based on relative change (we use absolute as rough proxy)
        let hr_score = if hashrate > 500.0 {
            0.65 // High hashrate
        } else if hashrate > 100.0 {
            0.55
        } else {
            0.45 // Low hashrate
        };

        Ok(mempool_score * 0.5 + hr_score * 0.5)
    }

    /// Fetch crypto market data from CoinGecko (keyless, free tier).
    /// Provides: volume spike, 24h change, market cap trend.
    async fn fetch_coingecko_onchain(
        &self,
        symbol: &str,
    ) -> Result<(f64, String), Box<dyn Error + Send + Sync>> {
        let id = match symbol {
            "BTC" => "bitcoin",
            "ETH" => "ethereum",
            "SOL" => "solana",
            _ => return Err("not a supported CoinGecko symbol".into()),
        };

        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(5))
            .build()?;
        let url = format!(
            "https://api.coingecko.com/api/v3/coins/{}?localization=false&tickers=false&market_data=true&community_data=false",
            id
        );

        let v: serde_json::Value = client
            .get(&url)
            .timeout(std::time::Duration::from_secs(5))
            .send()
            .await?
            .json()
            .await?;

        let md = v.get("market_data").ok_or("no market_data")?;
        let vol_24h = md
            .get("total_volume")
            .and_then(|x| x.get("usd"))
            .and_then(|x| x.as_f64())
            .unwrap_or(0.0);
        let price_chg_24h = md
            .get("price_change_percentage_24h")
            .and_then(|x| x.as_f64())
            .unwrap_or(0.0);
        let mcap = md
            .get("market_cap")
            .and_then(|x| x.get("usd"))
            .and_then(|x| x.as_f64())
            .unwrap_or(0.0);

        println!(
            "[OnChain/CoinGecko] {} vol_24h=${:.0}B chg={:+.1}% mcap=${:.0}B",
            symbol,
            vol_24h / 1e9,
            price_chg_24h,
            mcap / 1e9
        );

        // Volume spike = more activity = bullish signal
        // For BTC: >$10B is high, <$2B is low
        let vol_threshold = match symbol {
            "BTC" => 10e9,
            "ETH" => 5e9,
            _ => 1e9,
        };
        let vol_score = if vol_24h > vol_threshold {
            0.65 // High volume = bullish
        } else if vol_24h < vol_threshold * 0.2 {
            0.4 // Low volume = bearish
        } else {
            0.5
        };

        // 24h price change as momentum signal
        let momentum_score = if price_chg_24h > 3.0 {
            0.7 // Strong bullish momentum
        } else if price_chg_24h > 0.5 {
            0.6 // Mild bullish
        } else if price_chg_24h < -3.0 {
            0.3 // Strong bearish momentum
        } else if price_chg_24h < -0.5 {
            0.4 // Mild bearish
        } else {
            0.5
        };

        let score = vol_score * 0.5 + momentum_score * 0.5;
        let note = format!(
            "CoinGecko vol=${:.1}B chg={:+.1}%",
            vol_24h / 1e9,
            price_chg_24h
        );
        Ok((score, note))
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
        "Real on-chain signals from free blockchain APIs (Blockchain.com mempool/hashrate, CoinGecko volume/momentum) + local volume/price proxy for accumulation detection."
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
                direction: if score > 0.55 {
                    tredo_core::agent::SkillDirection::Bullish
                } else if score < 0.45 {
                    tredo_core::agent::SkillDirection::Bearish
                } else {
                    tredo_core::agent::SkillDirection::Neutral
                },
                weight: 0.15,
            })
        } else {
            Ok(AgentOutput::Done)
        }
    }
}
