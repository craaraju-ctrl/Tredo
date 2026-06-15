use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::weight_tuner::AttributionEngine;

/// Internal representation of a skill's output during walk-forward validation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillResult {
    pub score: f64,
    pub confidence: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoricalCandle {
    pub timestamp: u64,
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
    pub volume: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WalkForwardConfig {
    pub train_window_size: usize, // Number of candles for the training (in-sample) block
    pub test_window_size: usize,  // Number of candles for the evaluation (out-of-sample) block
    pub step_size: usize,         // Step increment to walk forward (e.g. equal to test_window_size)
    pub initial_capital: f64,
    pub base_learning_rate: f64,
    pub overfitting_threshold: f64, // Max allowed drop ratio between in-sample and out-of-sample performance
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FoldResult {
    pub fold_index: usize,
    pub train_start_ts: u64,
    pub train_end_ts: u64,
    pub test_start_ts: u64,
    pub test_end_ts: u64,
    pub in_sample_sharpe: f64,
    pub out_of_sample_sharpe: f64,
    pub test_win_rate: f64,
    pub test_avg_regret: f64,
    pub rule_version_at_test: u32,
    pub passed_stability: bool,
    pub degradation_ratio: f64, // Calculated as (1.0 - (OOS_Sharpe / IS_Sharpe))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WalkForwardReport {
    pub total_folds_evaluated: usize,
    pub folds: Vec<FoldResult>,
    pub mean_in_sample_sharpe: f64,
    pub mean_out_of_sample_sharpe: f64,
    pub structural_stability_score: f64, // Percentage of folds that passed the degradation check
    pub overall_recommendation: String,
}

pub struct WalkForwardRunner {
    pub config: WalkForwardConfig,
    pub weight_tuner: AttributionEngine,
    // regime_classifier can be injected when a full SharedState is available in the calling context
}

impl WalkForwardRunner {
    pub fn new(config: WalkForwardConfig) -> Self {
        let lr = config.base_learning_rate;
        Self {
            config,
            weight_tuner: AttributionEngine::new(lr),
        }
    }

    /// Evaluates dynamic skill learning performance across rolling data folds.
    pub async fn run_validation<F>(
        &self,
        symbol: &str,
        data_series: &[HistoricalCandle],
        mut active_weights: HashMap<String, f64>,
        mut signal_generation_fn: F,
    ) -> Result<WalkForwardReport, String>
    where
        F: FnMut(
            &[HistoricalCandle],
            &HashMap<String, f64>,
        ) -> Result<Option<Vec<SkillResult>>, String>,
    {
        if data_series.len() < (self.config.train_window_size + self.config.test_window_size) {
            return Err(
                "Insufficient historical series length for the configured walk-forward parameters"
                    .to_string(),
            );
        }

        let mut folds = Vec::new();
        let mut fold_index = 0;
        let mut pointer = 0;

        // Walk through the data series using rolling window splits
        while (pointer + self.config.train_window_size + self.config.test_window_size)
            <= data_series.len()
        {
            let train_start = pointer;
            let train_end = pointer + self.config.train_window_size;
            let test_start = train_end;
            let test_end = test_start + self.config.test_window_size;

            let train_slice = &data_series[train_start..train_end];
            let test_slice = &data_series[test_start..test_end];

            // 1. Train Stage: Optimize weights on the in-sample (train) partition
            let (is_sharpe, trained_weights) = self
                .simulate_and_train_weights(
                    symbol,
                    train_slice,
                    active_weights.clone(),
                    &mut signal_generation_fn,
                )
                .await?;

            // 2. Evaluate Stage: Run out-of-sample backtest with frozen trained weights
            let (oos_sharpe, oos_win_rate, oos_regret, rule_version) = self
                .evaluate_out_of_sample(
                    symbol,
                    test_slice,
                    trained_weights.clone(),
                    &mut signal_generation_fn,
                )
                .await?;

            // 3. Overfitting Check: Measure performance degradation from train to test
            let degradation_ratio = if is_sharpe > 0.0 {
                (is_sharpe - oos_sharpe) / is_sharpe
            } else {
                0.0
            };

            let passed_stability =
                oos_sharpe > 0.0 && degradation_ratio < self.config.overfitting_threshold;

            folds.push(FoldResult {
                fold_index,
                train_start_ts: train_slice.first().unwrap().timestamp,
                train_end_ts: train_slice.last().unwrap().timestamp,
                test_start_ts: test_slice.first().unwrap().timestamp,
                test_end_ts: test_slice.last().unwrap().timestamp,
                in_sample_sharpe: is_sharpe,
                out_of_sample_sharpe: oos_sharpe,
                test_win_rate: oos_win_rate,
                test_avg_regret: oos_regret,
                rule_version_at_test: rule_version,
                passed_stability,
                degradation_ratio,
            });

            // Retain the trained weights to bootstrap the next fold step
            active_weights = trained_weights;

            // Advance window pointer
            pointer += self.config.step_size;
            fold_index += 1;
        }

        // Aggregate overall cross-fold metrics
        let total_folds = folds.len();
        if total_folds == 0 {
            return Err("Zero folds were generated. Check window boundary variables.".to_string());
        }

        let sum_is_sharpe: f64 = folds.iter().map(|f| f.in_sample_sharpe).sum();
        let sum_oos_sharpe: f64 = folds.iter().map(|f| f.out_of_sample_sharpe).sum();
        let stable_count = folds.iter().filter(|f| f.passed_stability).count();

        let mean_in_sample_sharpe = sum_is_sharpe / total_folds as f64;
        let mean_out_of_sample_sharpe = sum_oos_sharpe / total_folds as f64;
        let stability_score = stable_count as f64 / total_folds as f64;

        // Generate algorithmic deployment recommendation based on validation metrics
        let overall_recommendation = if mean_out_of_sample_sharpe > 1.2 && stability_score >= 0.75 {
            "DEPLOY_APPROVED: Strong out-of-sample performance with low parameter overfitting."
                .to_string()
        } else if mean_out_of_sample_sharpe > 0.5 && stability_score >= 0.50 {
            "CAUTION_DEGRADED: System shows mild edge, but parameters are vulnerable to regime changes.".to_string()
        } else {
            "REJECT_OVERFITTED: High performance degradation. The learning loop is overfitting to history.".to_string()
        };

        Ok(WalkForwardReport {
            total_folds_evaluated: total_folds,
            folds,
            mean_in_sample_sharpe,
            mean_out_of_sample_sharpe,
            structural_stability_score: stability_score,
            overall_recommendation,
        })
    }

    /// Backtests and continuously learns weights over the training segment.
    async fn simulate_and_train_weights<F>(
        &self,
        _symbol: &str,
        slice: &[HistoricalCandle],
        mut weights: HashMap<String, f64>,
        signal_generation_fn: &mut F,
    ) -> Result<(f64, HashMap<String, f64>), String>
    where
        F: FnMut(
            &[HistoricalCandle],
            &HashMap<String, f64>,
        ) -> Result<Option<Vec<SkillResult>>, String>,
    {
        let mut returns = Vec::new();
        let mut position_open = false;
        let mut pre_trade_weights = weights.clone();

        for idx in 14..slice.len() {
            let current_sub_slice = &slice[0..idx];
            let current_candle = &slice[idx];

            if position_open {
                // Manage existing position: simple exit after 5 ticks to collect outcomes
                let exit_price = current_candle.close;
                let entry_price = slice[idx - 5].close;
                let trade_pnl = (exit_price - entry_price) / entry_price;

                returns.push(trade_pnl);

                // Construct skill predictions matching trade direction
                let skill_predictions: HashMap<String, f64> = pre_trade_weights
                    .keys()
                    .map(|k| (k.clone(), 0.8)) // Assume buy prediction strength
                    .collect();

                // Run symmetric backpropagation learning loop
                let update = self.weight_tuner.tune_skill_weights(
                    "wf_train_episode",
                    trade_pnl,
                    "BUY",
                    &skill_predictions,
                    &weights,
                    current_candle.timestamp,
                );

                weights = update.updated_weights;
                position_open = false;
            } else {
                // Query active learning models
                if let Ok(Some(skills)) = signal_generation_fn(current_sub_slice, &weights) {
                    let buy_votes = skills.iter().filter(|s| s.score > 0.2).count();
                    if buy_votes >= 4 {
                        pre_trade_weights = weights.clone();
                        position_open = true;
                    }
                }
            }
        }

        let sharpe = self.calculate_sharpe_ratio(&returns);
        Ok((sharpe, weights))
    }

    /// Evaluates frozen parameters on out-of-sample data.
    async fn evaluate_out_of_sample<F>(
        &self,
        _symbol: &str,
        slice: &[HistoricalCandle],
        weights: HashMap<String, f64>,
        signal_generation_fn: &mut F,
    ) -> Result<(f64, f64, f64, u32), String>
    where
        F: FnMut(
            &[HistoricalCandle],
            &HashMap<String, f64>,
        ) -> Result<Option<Vec<SkillResult>>, String>,
    {
        let mut returns = Vec::new();
        let mut position_open = false;
        let mut total_trades = 0;
        let mut wins = 0;
        let mut total_regret = 0.0;

        for idx in 14..slice.len() {
            let current_sub_slice = &slice[0..idx];
            let current_candle = &slice[idx];

            if position_open {
                let exit_price = current_candle.close;
                let entry_price = slice[idx - 5].close;
                let trade_pnl = (exit_price - entry_price) / entry_price;

                returns.push(trade_pnl);
                total_trades += 1;

                if trade_pnl > 0.0 {
                    wins += 1;
                } else {
                    total_regret += trade_pnl.abs();
                }

                position_open = false;
            } else {
                if let Ok(Some(skills)) = signal_generation_fn(current_sub_slice, &weights) {
                    let buy_votes = skills.iter().filter(|s| s.score > 0.2).count();
                    if buy_votes >= 4 {
                        position_open = true;
                    }
                }
            }
        }

        let sharpe = self.calculate_sharpe_ratio(&returns);
        let win_rate = if total_trades > 0 {
            wins as f64 / total_trades as f64
        } else {
            0.0
        };
        let avg_regret = if total_trades - wins > 0 {
            total_regret / (total_trades - wins) as f64
        } else {
            0.0
        };

        Ok((sharpe, win_rate, avg_regret, 1))
    }

    fn calculate_sharpe_ratio(&self, returns: &[f64]) -> f64 {
        if returns.len() < 2 {
            return 0.0;
        }

        let mean: f64 = returns.iter().sum::<f64>() / returns.len() as f64;
        let variance: f64 = returns
            .iter()
            .map(|&r| {
                let diff = r - mean;
                diff * diff
            })
            .sum::<f64>()
            / (returns.len() - 1) as f64;

        let std_dev = variance.sqrt();
        if std_dev == 0.0 {
            return 0.0;
        }

        // Annualized Sharpe Ratio assuming daily interval steps (252 steps/year)
        (mean / std_dev) * (252.0_f64).sqrt()
    }
}
